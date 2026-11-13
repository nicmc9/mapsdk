//! GPU-ресурсы сцены и отрисовка PBR: меши, текстуры, материалы, пайплайны.

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use engine_assets::AssetId;
use engine_assets::formats::{AlphaMode, ColorSpace, Image, Material, Mesh};
use engine_math::{Aabb, Frustum, Mat4, Vec3, camera};
use wgpu::util::DeviceExt;

use crate::command::MaterialData;
use crate::snapshot::{FrameSnapshot, interpolate};

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_POINT_LIGHTS: usize = 8;

// ---------- данные для шейдера ----------
// [Rust] `#[repr(C)]` — раскладка полей как в C (без перестановок компилятором).
// `Pod` (plain old data) из bytemuck разрешает смотреть на структуру как на байты.
// Размеры и выравнивание совпадают с WGSL (std140): только vec4 и mat4.

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PointLightUniform {
    position_range: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    camera_position: [f32; 4],
    ambient: [f32; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
    counts: [u32; 4],
    points: [PointLightUniform; MAX_POINT_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    normal_matrix: [[f32; 4]; 4],
}

/// Вершина: позиция, нормаль, UV — 32 байта.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

// ---------- GPU-ресурсы ----------

struct GpuPrimitive {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    bounds: Aabb,
}

struct GpuTexture {
    view: wgpu::TextureView,
}

struct GpuMaterial {
    data: MaterialData,
    uniform: wgpu::Buffer,
    /// Группа привязок и какие текстуры в ней были настоящими (остальные — заглушки).
    bind_group: Option<(wgpu::BindGroup, [bool; 5])>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PipelineKind {
    Opaque { double_sided: bool },
    Blend { double_sided: bool },
}

/// Статистика кадра.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DrawStats {
    pub drawn: u32,
    pub culled: u32,
    pub missing: u32,
}

pub(crate) struct SceneRenderer {
    frame_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    object_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<PipelineKind, wgpu::RenderPipeline>,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    object_buffer: wgpu::Buffer,
    object_bind_group: wgpu::BindGroup,
    object_capacity: u64,
    object_stride: u64,
    object_data: Vec<u8>,
    sampler: wgpu::Sampler,
    /// Заглушки: белая (sRGB и линейная), «плоская» нормаль.
    white_srgb: wgpu::TextureView,
    white_linear: wgpu::TextureView,
    flat_normal: wgpu::TextureView,
    default_material: GpuMaterial,
    meshes: HashMap<AssetId<Mesh>, Vec<GpuPrimitive>>,
    textures: HashMap<AssetId<Image>, GpuTexture>,
    materials: HashMap<AssetId<Material>, GpuMaterial>,
    manual_srgb: bool,
    /// Объекты, прошедшие отсечение в этом кадре: (индекс в снимке, пайплайн, дистанция²).
    /// Слот объекта в буфере матриц = позиция в этом списке.
    visible: Vec<(usize, PipelineKind, f32)>,
    /// Порядок отрисовки — индексы в `visible`.
    order: Vec<usize>,
}

/// Куда рисовать кадр.
pub(crate) struct FrameTarget<'a> {
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub size: (u32, u32),
}

/// Матрица «проекция × вид» и позиция камеры. Без камеры — вид издалека на начало координат.
fn camera_matrices(snapshot: &FrameSnapshot, aspect: f32, alpha: f32) -> (Mat4, Vec3) {
    let (world, projection) = snapshot.camera.map_or_else(
        || {
            (
                camera::look_at(Vec3::new(0.0, 1.0, 5.0), Vec3::ZERO, Vec3::Y).inverse(),
                camera::perspective(0.9, aspect, 0.05, 1000.0),
            )
        },
        |c| {
            (
                interpolate(&c.prev, &c.curr, alpha),
                camera::perspective(c.fov_y, aspect, c.near, c.far),
            )
        },
    );
    (projection * world.inverse(), world.w_axis.truncate())
}

impl SceneRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let _zone = tracing::trace_span!("scene_renderer_new").entered();
        let (frame_layout, material_layout, object_layout) = create_layouts(device);

        // Объекты кладутся в один буфер, каждый по своему смещению (dynamic offset).
        // Смещение обязано быть кратно min_uniform_buffer_offset_alignment (обычно 256).
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let object_stride =
            (std::mem::size_of::<ObjectUniform>() as u64).div_ceil(alignment) * alignment;

        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let object_capacity = 256;
        let (object_buffer, object_bind_group) =
            create_object_buffer(device, &object_layout, object_stride, object_capacity);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let solid = |rgba: [u8; 4], srgb: bool| {
            create_texture(
                device,
                queue,
                &Image {
                    width: 1,
                    height: 1,
                    color_space: if srgb {
                        ColorSpace::Srgb
                    } else {
                        ColorSpace::Linear
                    },
                    data: rgba.to_vec(),
                    mips: Vec::new(),
                },
            )
            .view
        };
        let white_srgb = solid([255; 4], true);
        let white_linear = solid([255; 4], false);
        let flat_normal = solid([128, 128, 255, 255], false);

        let pipelines = create_pipelines(
            device,
            &[&frame_layout, &material_layout, &object_layout],
            color_format,
        );

        let default_material = GpuMaterial::new(device, MaterialData::from(&Material::default()));
        Self {
            frame_layout,
            material_layout,
            object_layout,
            pipelines,
            frame_buffer,
            frame_bind_group,
            object_buffer,
            object_bind_group,
            object_capacity,
            object_stride,
            object_data: Vec::new(),
            sampler,
            white_srgb,
            white_linear,
            flat_normal,
            default_material,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            materials: HashMap::new(),
            manual_srgb: !color_format.is_srgb(),
            visible: Vec::new(),
            order: Vec::new(),
        }
    }

    pub fn upload_mesh(&mut self, device: &wgpu::Device, id: AssetId<Mesh>, mesh: &Mesh) {
        let _zone = tracing::trace_span!("upload_mesh").entered();
        let primitives = mesh
            .primitives
            .iter()
            .map(|p| {
                let vertices: Vec<Vertex> = (0..p.positions.len())
                    .map(|i| Vertex {
                        position: p.positions[i],
                        normal: p.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                        uv: p.uvs.get(i).copied().unwrap_or_default(),
                    })
                    .collect();
                GpuPrimitive {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("indices"),
                        contents: bytemuck::cast_slice(&p.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: u32::try_from(p.indices.len()).unwrap_or(u32::MAX),
                    bounds: p.bounds,
                }
            })
            .collect();
        self.meshes.insert(id, primitives);
    }

    pub fn upload_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: AssetId<Image>,
        image: &Image,
    ) {
        let _zone = tracing::trace_span!("upload_image").entered();
        self.textures
            .insert(id, create_texture(device, queue, image));
    }

    pub fn upload_material(
        &mut self,
        device: &wgpu::Device,
        id: AssetId<Material>,
        data: MaterialData,
    ) {
        self.materials.insert(id, GpuMaterial::new(device, data));
    }

    pub fn remove_mesh(&mut self, id: AssetId<Mesh>) {
        self.meshes.remove(&id);
    }
    pub fn remove_image(&mut self, id: AssetId<Image>) {
        self.textures.remove(&id);
    }
    pub fn remove_material(&mut self, id: AssetId<Material>) {
        self.materials.remove(&id);
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        (self.meshes.len(), self.textures.len(), self.materials.len())
    }

    /// Рисует сцену. `alpha` — коэффициент интерполяции между шагами логики.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &FrameTarget<'_>,
        snapshot: &FrameSnapshot,
        alpha: f32,
    ) -> DrawStats {
        let _zone = tracing::trace_span!("render_scene").entered();
        let mut stats = DrawStats::default();
        #[allow(clippy::cast_precision_loss)]
        let aspect = target.size.0 as f32 / target.size.1.max(1) as f32;
        let (view_proj, camera_position) = camera_matrices(snapshot, aspect, alpha);

        self.cull_and_collect(snapshot, alpha, &view_proj, camera_position, &mut stats);
        self.ensure_object_capacity(device, self.visible.len() as u64);
        if !self.object_data.is_empty() {
            queue.write_buffer(&self.object_buffer, 0, &self.object_data);
        }
        let frame = self.frame_uniform(snapshot, &view_proj, camera_position);
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));
        self.prepare_materials(device, snapshot);
        self.draw(encoder, target, snapshot, alpha, &mut stats);
        stats
    }

    /// Отсечение по фрустуму и сбор матриц видимых объектов в один буфер.
    fn cull_and_collect(
        &mut self,
        snapshot: &FrameSnapshot,
        alpha: f32,
        view_proj: &Mat4,
        camera_position: Vec3,
        stats: &mut DrawStats,
    ) {
        let _zone = tracing::trace_span!("cull").entered();
        let frustum = Frustum::from_view_projection(view_proj);
        self.object_data.clear();
        self.visible.clear();
        for (index, draw) in snapshot.draws.iter().enumerate() {
            let Some(primitive) = self
                .meshes
                .get(&draw.mesh)
                .and_then(|p| p.get(draw.primitive as usize))
            else {
                stats.missing += 1;
                continue;
            };
            let model = interpolate(&draw.prev, &draw.curr, alpha);
            let world_bounds = primitive.bounds.transformed(&model);
            if !frustum.intersects_aabb(&world_bounds) {
                stats.culled += 1;
                continue;
            }
            let material = draw
                .material
                .and_then(|id| self.materials.get(&id))
                .map_or(&self.default_material.data, |m| &m.data);
            let double_sided = material.double_sided;
            let kind = if material.alpha_mode == AlphaMode::Blend {
                PipelineKind::Blend { double_sided }
            } else {
                PipelineKind::Opaque { double_sided }
            };
            let distance = (world_bounds.center() - camera_position).length_squared();
            let object = ObjectUniform {
                model: model.to_cols_array_2d(),
                normal_matrix: model.inverse().transpose().to_cols_array_2d(),
            };
            let offset = self.object_data.len();
            self.object_data
                .extend_from_slice(bytemuck::bytes_of(&object));
            self.object_data.resize(
                offset + usize::try_from(self.object_stride).unwrap_or(256),
                0,
            );
            self.visible.push((index, kind, distance));
        }
        // Порядок: непрозрачные, затем прозрачные от дальних к ближним.
        // Матрицы уже лежат в буфере по старым позициям — запоминаем слот до сортировки.
        let mut order: Vec<usize> = (0..self.visible.len()).collect();
        let blend = |k: &PipelineKind| matches!(k, PipelineKind::Blend { .. });
        order.sort_by(|&a, &b| {
            let (a, b) = (&self.visible[a], &self.visible[b]);
            blend(&a.1).cmp(&blend(&b.1)).then_with(|| {
                if blend(&a.1) {
                    b.2.total_cmp(&a.2)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });
        self.order = order;
    }

    /// Группы привязок материалов: создаются при первой нужде и пересоздаются,
    /// когда догрузились текстуры.
    fn prepare_materials(&mut self, device: &wgpu::Device, snapshot: &FrameSnapshot) {
        // [Rust] Обход по индексу, а не `for x in &self.visible`: ссылка на `self.visible`
        // жила бы весь цикл, а `prepare_material` требует `&mut self` — borrow checker
        // запрещает такое пересечение (в C++ это была бы потенциальная инвалидация итератора).
        for slot in 0..self.visible.len() {
            let index = self.visible[slot].0;
            if let Some(id) = snapshot.draws[index].material {
                self.prepare_material(device, id);
            }
        }
        if self.default_material.bind_group.is_none() {
            let bind_group = self.material_bind_group(
                device,
                &self.default_material.data,
                &self.default_material.uniform,
            );
            self.default_material.bind_group = Some((bind_group, [false; 5]));
        }
    }

    fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &FrameTarget<'_>,
        snapshot: &FrameSnapshot,
        alpha: f32,
        stats: &mut DrawStats,
    ) {
        let mut clear = snapshot.interpolated_clear_color(alpha).to_wgpu();
        if self.manual_srgb {
            // Цвет очистки не проходит через шейдер: на поверхности без sRGB (Web, GLES)
            // кодируем его сами, как шейдер кодирует пиксели
            let encode = |c: f64| {
                if c <= 0.003_130_8 {
                    c * 12.92
                } else {
                    1.055 * c.powf(1.0 / 2.4) - 0.055
                }
            };
            clear = wgpu::Color {
                r: encode(clear.r),
                g: encode(clear.g),
                b: encode(clear.b),
                a: clear.a,
            };
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.frame_bind_group, &[]);
        let mut current_pipeline = None;
        for &slot in &self.order {
            let (index, kind, _) = self.visible[slot];
            let draw = &snapshot.draws[index];
            let primitive = &self.meshes[&draw.mesh][draw.primitive as usize];
            let material = draw
                .material
                .and_then(|id| self.materials.get(&id))
                .unwrap_or(&self.default_material);
            let Some((bind_group, _)) = material.bind_group.as_ref() else {
                continue;
            };
            if current_pipeline != Some(kind) {
                pass.set_pipeline(&self.pipelines[&kind]);
                current_pipeline = Some(kind);
            }
            pass.set_bind_group(1, bind_group, &[]);
            let offset = u32::try_from(slot as u64 * self.object_stride).unwrap_or(0);
            pass.set_bind_group(2, &self.object_bind_group, &[offset]);
            pass.set_vertex_buffer(0, primitive.vertices.slice(..));
            pass.set_index_buffer(primitive.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..primitive.index_count, 0, 0..1);
            stats.drawn += 1;
        }
    }

    fn frame_uniform(
        &self,
        snapshot: &FrameSnapshot,
        view_proj: &Mat4,
        camera_position: Vec3,
    ) -> FrameUniform {
        let lights = &snapshot.lights;
        let mut points = [PointLightUniform::zeroed(); MAX_POINT_LIGHTS];
        for (slot, (position, range, color)) in points.iter_mut().zip(&lights.points) {
            *slot = PointLightUniform {
                position_range: [position.x, position.y, position.z, *range],
                color: [color[0], color[1], color[2], 0.0],
            };
        }
        let (sun_direction, sun_color) = lights.sun.map_or(([0.0; 4], [0.0; 4]), |(dir, color)| {
            (
                [dir.x, dir.y, dir.z, 1.0],
                [color[0], color[1], color[2], 0.0],
            )
        });
        FrameUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera_position: camera_position.extend(1.0).to_array(),
            ambient: [lights.ambient[0], lights.ambient[1], lights.ambient[2], 0.0],
            sun_direction,
            sun_color,
            counts: [
                u32::try_from(lights.points.len().min(MAX_POINT_LIGHTS)).unwrap_or(0),
                u32::from(self.manual_srgb),
                0,
                0,
            ],
            points,
        }
    }

    fn ensure_object_capacity(&mut self, device: &wgpu::Device, count: u64) {
        if count <= self.object_capacity {
            return;
        }
        // Растим с запасом, чтобы не пересоздавать каждый кадр
        self.object_capacity = count.next_power_of_two();
        let (buffer, bind_group) = create_object_buffer(
            device,
            &self.object_layout,
            self.object_stride,
            self.object_capacity,
        );
        self.object_buffer = buffer;
        self.object_bind_group = bind_group;
    }

    fn prepare_material(&mut self, device: &wgpu::Device, id: AssetId<Material>) {
        let Some(material) = self.materials.get(&id) else {
            return;
        };
        let available = texture_slots(&material.data)
            .map(|t| t.is_some_and(|id| self.textures.contains_key(&id)));
        if material
            .bind_group
            .as_ref()
            .is_some_and(|(_, ready)| *ready == available)
        {
            return;
        }
        let bind_group = self.material_bind_group(device, &material.data, &material.uniform);
        if let Some(material) = self.materials.get_mut(&id) {
            material.bind_group = Some((bind_group, available));
        }
    }

    fn material_bind_group(
        &self,
        device: &wgpu::Device,
        data: &MaterialData,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let [base, mr, normal, occlusion, emissive] = texture_slots(data);
        let view =
            |id: Option<AssetId<Image>>, fallback: &'_ wgpu::TextureView| -> wgpu::TextureView {
                id.and_then(|id| self.textures.get(&id))
                    .map_or_else(|| fallback.clone(), |t| t.view.clone())
            };
        let views = [
            view(base, &self.white_srgb),
            view(mr, &self.white_linear),
            view(normal, &self.flat_normal),
            view(occlusion, &self.white_linear),
            view(emissive, &self.white_srgb),
        ];
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        for (i, view) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: u32::try_from(i + 1).unwrap_or(1),
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material"),
            layout: &self.material_layout,
            entries: &entries,
        })
    }

    #[allow(dead_code)]
    fn layouts(&self) -> (&wgpu::BindGroupLayout, &wgpu::BindGroupLayout) {
        (&self.frame_layout, &self.material_layout)
    }
}

/// Раскладки групп привязок: 0 — кадр, 1 — материал, 2 — объект.
fn create_layouts(
    device: &wgpu::Device,
) -> (
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
) {
    let uniform = |binding, visibility, dynamic: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    };
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("frame"),
        entries: &[uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT, false)],
    });
    let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("material"),
        entries: &[
            uniform(0, wgpu::ShaderStages::FRAGMENT, false),
            texture(1),
            texture(2),
            texture(3),
            texture(4),
            texture(5),
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let object = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("object"),
        entries: &[uniform(0, wgpu::ShaderStages::VERTEX, true)],
    });
    (frame, material, object)
}

fn texture_slots(data: &MaterialData) -> [Option<AssetId<Image>>; 5] {
    [
        data.base_color_texture,
        data.metallic_roughness_texture,
        data.normal_texture,
        data.occlusion_texture,
        data.emissive_texture,
    ]
}

impl GpuMaterial {
    fn new(device: &wgpu::Device, data: MaterialData) -> Self {
        let alpha_cutoff = match data.alpha_mode {
            AlphaMode::Mask(cutoff) => f32::from(cutoff) / 255.0,
            _ => -1.0,
        };
        let uniform = MaterialUniform {
            base_color: data.base_color,
            emissive: [
                data.emissive[0],
                data.emissive[1],
                data.emissive[2],
                alpha_cutoff,
            ],
            params: [
                data.metallic,
                data.roughness,
                1.0,
                if data.normal_texture.is_some() {
                    1.0
                } else {
                    0.0
                },
            ],
        };
        Self {
            uniform: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("material"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            }),
            data,
            bind_group: None,
        }
    }
}

fn create_object_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: u64,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("objects"),
        size: stride * capacity,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("objects"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            // Шейдер видит один ObjectUniform; какой именно — задаёт смещение при отрисовке
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: NonZeroU64::new(std::mem::size_of::<ObjectUniform>() as u64),
            }),
        }],
    });
    (buffer, bind_group)
}

fn create_texture(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image) -> GpuTexture {
    let format = match image.color_space {
        ColorSpace::Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        ColorSpace::Linear => wgpu::TextureFormat::Rgba8Unorm,
    };
    let size = wgpu::Extent3d {
        width: image.width,
        height: image.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image"),
        size,
        mip_level_count: image.mip_level_count(),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let levels = std::iter::once(&image.data).chain(image.mips.iter());
    for (level, data) in levels.enumerate() {
        let level = u32::try_from(level).unwrap_or(0);
        let width = (image.width >> level).max(1);
        let height = (image.height >> level).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    GpuTexture {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
    }
}

/// PBR-пайплайны для всех сочетаний: прозрачность × двусторонность.
fn create_pipelines(
    device: &wgpu::Device,
    layouts: &[&wgpu::BindGroupLayout; 3],
    color_format: wgpu::TextureFormat,
) -> HashMap<PipelineKind, wgpu::RenderPipeline> {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pbr"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/pbr.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pbr"),
        bind_group_layouts: &layouts.map(Some),
        ..Default::default()
    });
    let mut pipelines = HashMap::new();
    for double_sided in [false, true] {
        for kind in [
            PipelineKind::Opaque { double_sided },
            PipelineKind::Blend { double_sided },
        ] {
            pipelines.insert(
                kind,
                create_pipeline(device, &layout, &shader, color_format, kind),
            );
        }
    }
    pipelines
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    color_format: wgpu::TextureFormat,
    kind: PipelineKind,
) -> wgpu::RenderPipeline {
    let (blend, double_sided, depth_write) = match kind {
        PipelineKind::Opaque { double_sided } => (None, double_sided, true),
        PipelineKind::Blend { double_sided } => {
            (Some(wgpu::BlendState::ALPHA_BLENDING), double_sided, false)
        }
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pbr"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
            })],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: if double_sided { None } else { Some(wgpu::Face::Back) },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
