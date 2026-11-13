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
const OBJECT_SIZE: u64 = std::mem::size_of::<ObjectUniform>() as u64;
/// 256 матриц 4×4 — массив `Joints` в шейдере.
const JOINTS_SIZE: u64 = (crate::extract::MAX_JOINTS * 64) as u64;

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
    /// Индексы костей и веса — только у скелетных мешей.
    skin: Option<wgpu::Buffer>,
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

/// Вариант PBR-пайплайна: 2 × 2 × 2 = 8 сочетаний.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKind {
    blend: bool,
    double_sided: bool,
    skinned: bool,
}

/// Объект, прошедший отсечение.
#[derive(Clone, Copy)]
struct VisibleDraw {
    index: usize,
    kind: PipelineKind,
    distance: f32,
    /// Слот матриц костей в буфере скелетов.
    joint_slot: Option<u32>,
}

/// Индексы четырёх костей и их веса — 24 байта на вершину.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinVertex {
    joints: [u16; 4],
    weights: [f32; 4],
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
    joints_layout: wgpu::BindGroupLayout,
    joint_buffer: wgpu::Buffer,
    joint_bind_group: wgpu::BindGroup,
    joint_capacity: u64,
    joint_stride: u64,
    joint_data: Vec<u8>,
    /// Начало скелета в снимке → слот в буфере (один скелет — несколько примитивов).
    joint_slots: HashMap<u32, u32>,
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
    visible: Vec<VisibleDraw>,
    /// Порядок отрисовки — индексы в `visible`.
    order: Vec<usize>,
    line_pipeline: wgpu::RenderPipeline,
    color_format: wgpu::TextureFormat,
    line_buffer: Option<(wgpu::Buffer, u64)>,
    line_count: u32,
}

/// Куда рисовать кадр.
pub(crate) struct FrameTarget<'a> {
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub size: (u32, u32),
    /// Метки времени GPU для прохода (если поддерживаются).
    pub timestamps: Option<wgpu::RenderPassTimestampWrites<'a>>,
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
        sources: &HashMap<String, String>,
    ) -> Self {
        let _zone = tracing::trace_span!("scene_renderer_new").entered();
        let (frame_layout, material_layout, object_layout, joints_layout) = create_layouts(device);

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
        let (object_buffer, object_bind_group) = create_object_buffer(
            device,
            &object_layout,
            object_stride,
            object_capacity,
            OBJECT_SIZE,
        );
        // Скелет — 256 матриц (16 КБ, минимально гарантированный размер uniform-привязки)
        let joint_stride = JOINTS_SIZE.div_ceil(alignment) * alignment;
        let joint_capacity = 4;
        let (joint_buffer, joint_bind_group) = create_object_buffer(
            device,
            &joints_layout,
            joint_stride,
            joint_capacity,
            JOINTS_SIZE,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let [white_srgb, white_linear, flat_normal] = fallback_textures(device, queue);

        let layouts = [
            &frame_layout,
            &material_layout,
            &object_layout,
            &joints_layout,
        ];
        // Встроенные шейдеры обязаны собираться — иначе это ошибка в самом движке
        let (pipelines, line_pipeline) = build_pipelines(device, layouts, color_format, sources)
            .unwrap_or_else(|error| panic!("шейдеры движка не собрались: {error}"));
        let default_material = GpuMaterial::new(device, MaterialData::from(&Material::default()));
        Self {
            frame_layout,
            material_layout,
            object_layout,
            joints_layout,
            joint_buffer,
            joint_bind_group,
            joint_capacity,
            joint_stride,
            joint_data: Vec::new(),
            joint_slots: HashMap::new(),
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
            line_pipeline,
            color_format,
            line_buffer: None,
            line_count: 0,
        }
    }

    /// Пересобирает пайплайны из новых исходников шейдеров. При ошибке старые остаются.
    pub fn rebuild_shaders(
        &mut self,
        device: &wgpu::Device,
        sources: &HashMap<String, String>,
    ) -> Result<(), String> {
        let layouts = [
            &self.frame_layout,
            &self.material_layout,
            &self.object_layout,
            &self.joints_layout,
        ];
        let (pipelines, line_pipeline) =
            build_pipelines(device, layouts, self.color_format, sources)?;
        self.pipelines = pipelines;
        self.line_pipeline = line_pipeline;
        Ok(())
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
                    skin: (!p.joints.is_empty() && p.joints.len() == p.positions.len()).then(
                        || {
                            let skin: Vec<SkinVertex> = p
                                .joints
                                .iter()
                                .zip(
                                    p.weights
                                        .iter()
                                        .chain(std::iter::repeat(&[1.0, 0.0, 0.0, 0.0])),
                                )
                                .map(|(joints, weights)| SkinVertex {
                                    joints: *joints,
                                    weights: *weights,
                                })
                                .collect();
                            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("skin"),
                                contents: bytemuck::cast_slice(&skin),
                                usage: wgpu::BufferUsages::VERTEX,
                            })
                        },
                    ),
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
        self.ensure_joint_capacity(device, self.joint_slots.len() as u64);
        if !self.joint_data.is_empty() {
            queue.write_buffer(&self.joint_buffer, 0, &self.joint_data);
        }
        let frame = self.frame_uniform(snapshot, &view_proj, camera_position);
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));
        self.prepare_materials(device, snapshot);
        self.upload_lines(device, queue, snapshot);
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
        self.joint_data.clear();
        self.joint_slots.clear();
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
            let skin = draw.skin.filter(|_| primitive.skin.is_some());
            let mut world_bounds = primitive.bounds.transformed(&model);
            if skin.is_some() {
                // Анимация выводит конечности за AABB позы привязки — берём с запасом
                let (center, half) = (world_bounds.center(), world_bounds.half_extents() * 1.5);
                world_bounds = engine_math::Aabb {
                    min: center - half,
                    max: center + half,
                };
            }
            if !frustum.intersects_aabb(&world_bounds) {
                stats.culled += 1;
                continue;
            }
            let material = draw
                .material
                .and_then(|id| self.materials.get(&id))
                .map_or(&self.default_material.data, |m| &m.data);
            let kind = PipelineKind {
                blend: material.alpha_mode == AlphaMode::Blend,
                double_sided: material.double_sided,
                skinned: skin.is_some(),
            };
            // Скелетный меш: вершины уже в мире после скиннинга, матрица модели — единичная
            let model = if skin.is_some() {
                Mat4::IDENTITY
            } else {
                model
            };
            let joint_slot = skin.map(|(start, count)| self.joint_slot(snapshot, start, count));
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
            self.visible.push(VisibleDraw {
                index,
                kind,
                distance,
                joint_slot,
            });
        }
        // Порядок: непрозрачные, затем прозрачные от дальних к ближним.
        // Матрицы уже лежат в буфере по старым позициям — запоминаем слот до сортировки.
        let mut order: Vec<usize> = (0..self.visible.len()).collect();
        order.sort_by(|&a, &b| {
            let (a, b) = (&self.visible[a], &self.visible[b]);
            a.kind.blend.cmp(&b.kind.blend).then_with(|| {
                if a.kind.blend {
                    b.distance.total_cmp(&a.distance)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });
        self.order = order;
    }

    /// Слот матриц скелета в буфере: один на скелет, даже если у меша несколько примитивов.
    fn joint_slot(&mut self, snapshot: &FrameSnapshot, start: u32, count: u32) -> u32 {
        if let Some(&slot) = self.joint_slots.get(&start) {
            return slot;
        }
        let slot = u32::try_from(self.joint_slots.len()).unwrap_or(0);
        self.joint_slots.insert(start, slot);
        let begin = self.joint_data.len();
        let matrices = &snapshot.joints[start as usize..(start + count) as usize];
        for matrix in matrices {
            self.joint_data
                .extend_from_slice(bytemuck::bytes_of(&matrix.to_cols_array_2d()));
        }
        self.joint_data.resize(
            begin + usize::try_from(self.joint_stride).unwrap_or(16384),
            0,
        );
        slot
    }

    /// Группы привязок материалов: создаются при первой нужде и пересоздаются,
    /// когда догрузились текстуры.
    fn prepare_materials(&mut self, device: &wgpu::Device, snapshot: &FrameSnapshot) {
        // [Rust] Обход по индексу, а не `for x in &self.visible`: ссылка на `self.visible`
        // жила бы весь цикл, а `prepare_material` требует `&mut self` — borrow checker
        // запрещает такое пересечение (в C++ это была бы потенциальная инвалидация итератора).
        for slot in 0..self.visible.len() {
            let index = self.visible[slot].index;
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
            timestamp_writes: target.timestamps.clone(),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.frame_bind_group, &[]);
        let mut current_pipeline = None;
        for &slot in &self.order {
            let VisibleDraw {
                index,
                kind,
                joint_slot,
                ..
            } = self.visible[slot];
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
            if let (Some(joint_slot), Some(skin)) = (joint_slot, &primitive.skin) {
                let offset = u32::try_from(u64::from(joint_slot) * self.joint_stride).unwrap_or(0);
                pass.set_bind_group(3, &self.joint_bind_group, &[offset]);
                pass.set_vertex_buffer(1, skin.slice(..));
            }
            pass.set_index_buffer(primitive.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..primitive.index_count, 0, 0..1);
            stats.drawn += 1;
        }
        // Отладочные линии — поверх сцены, с проверкой глубины
        if let Some((buffer, _)) = &self.line_buffer
            && self.line_count > 0
        {
            pass.set_pipeline(&self.line_pipeline);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..self.line_count, 0..1);
        }
    }

    /// Копирует отладочные линии в GPU. Буфер растёт по мере надобности.
    fn upload_lines(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &FrameSnapshot,
    ) {
        self.line_count = u32::try_from(snapshot.lines.len()).unwrap_or(0);
        if snapshot.lines.is_empty() {
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(&snapshot.lines);
        let needed = bytes.len() as u64;
        if self
            .line_buffer
            .as_ref()
            .is_none_or(|(_, capacity)| *capacity < needed)
        {
            let capacity = needed.next_power_of_two();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("debug_lines"),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.line_buffer = Some((buffer, capacity));
        }
        if let Some((buffer, _)) = &self.line_buffer {
            queue.write_buffer(buffer, 0, bytes);
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

    fn ensure_joint_capacity(&mut self, device: &wgpu::Device, count: u64) {
        if count <= self.joint_capacity {
            return;
        }
        self.joint_capacity = count.next_power_of_two();
        let (buffer, bind_group) = create_object_buffer(
            device,
            &self.joints_layout,
            self.joint_stride,
            self.joint_capacity,
            JOINTS_SIZE,
        );
        self.joint_buffer = buffer;
        self.joint_bind_group = bind_group;
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
            OBJECT_SIZE,
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
    // Матрицы костей скелетного меша — тоже по смещению, как объекты
    let joints = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("joints"),
        entries: &[uniform(0, wgpu::ShaderStages::VERTEX, true)],
    });
    (frame, material, object, joints)
}

/// Текстуры-заглушки 1×1 для пустых слотов материала: белая sRGB, белая линейная,
/// «плоская» нормаль (0, 0, 1).
fn fallback_textures(device: &wgpu::Device, queue: &wgpu::Queue) -> [wgpu::TextureView; 3] {
    let solid = |rgba: [u8; 4], color_space| {
        let image = Image {
            width: 1,
            height: 1,
            color_space,
            data: rgba.to_vec(),
            mips: Vec::new(),
        };
        create_texture(device, queue, &image).view
    };
    [
        solid([255; 4], ColorSpace::Srgb),
        solid([255; 4], ColorSpace::Linear),
        solid([128, 128, 255, 255], ColorSpace::Linear),
    ]
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

/// Буфер «много элементов по смещению»: объекты (матрица модели) или скелеты (матрицы костей).
/// Шейдер видит один элемент размером `item_size`; какой — задаёт смещение при отрисовке.
fn create_object_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: u64,
    item_size: u64,
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
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: NonZeroU64::new(item_size),
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

type Pipelines = (
    HashMap<PipelineKind, wgpu::RenderPipeline>,
    wgpu::RenderPipeline,
);

/// Все пайплайны сцены из исходников шейдеров. Ошибка — текстом, без паники.
fn build_pipelines(
    device: &wgpu::Device,
    layouts: [&wgpu::BindGroupLayout; 4],
    color_format: wgpu::TextureFormat,
    sources: &HashMap<String, String>,
) -> Result<Pipelines, String> {
    let pbr = crate::shaders::compile(device, sources, "shaders/pbr.wgsl")?;
    let debug = crate::shaders::compile(device, sources, "shaders/debug.wgsl")?;
    crate::shaders::checked(device, || {
        (
            create_pipelines(device, layouts, color_format, &pbr),
            create_line_pipeline(device, layouts[0], color_format, &debug),
        )
    })
}

/// PBR-пайплайны для всех сочетаний: прозрачность × двусторонность × скиннинг.
fn create_pipelines(
    device: &wgpu::Device,
    layouts: [&wgpu::BindGroupLayout; 4],
    color_format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
) -> HashMap<PipelineKind, wgpu::RenderPipeline> {
    // Обычным мешам группа костей (3) не нужна — у них своя раскладка из трёх групп
    let static_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pbr"),
        bind_group_layouts: &[Some(layouts[0]), Some(layouts[1]), Some(layouts[2])],
        ..Default::default()
    });
    let skinned_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pbr_skinned"),
        bind_group_layouts: &layouts.map(Some),
        ..Default::default()
    });
    let mut pipelines = HashMap::new();
    for skinned in [false, true] {
        for double_sided in [false, true] {
            for blend in [false, true] {
                let kind = PipelineKind {
                    blend,
                    double_sided,
                    skinned,
                };
                let layout = if skinned {
                    &skinned_layout
                } else {
                    &static_layout
                };
                pipelines.insert(
                    kind,
                    create_pipeline(device, layout, shader, color_format, kind),
                );
            }
        }
    }
    pipelines
}

fn create_line_pipeline(
    device: &wgpu::Device,
    frame_layout: &wgpu::BindGroupLayout,
    color_format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("debug"),
        bind_group_layouts: &[Some(frame_layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("debug_lines"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<crate::debug_draw::LineVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            ..Default::default()
        },
        // Глубина проверяется (линии прячутся за объектами), но не записывается
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    color_format: wgpu::TextureFormat,
    kind: PipelineKind,
) -> wgpu::RenderPipeline {
    let blend = kind.blend.then_some(wgpu::BlendState::ALPHA_BLENDING);
    let (double_sided, depth_write) = (kind.double_sided, !kind.blend);
    let vertex_layout = Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
    });
    let skin_layout = Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<SkinVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![3 => Uint16x4, 4 => Float32x4],
    });
    let skinned_buffers = [vertex_layout.clone(), skin_layout];
    let static_buffers = [vertex_layout];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pbr"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if kind.skinned {
                "vs_skinned"
            } else {
                "vs_main"
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: if kind.skinned {
                &skinned_buffers
            } else {
                &static_buffers
            },
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: if double_sided {
                None
            } else {
                Some(wgpu::Face::Back)
            },
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
