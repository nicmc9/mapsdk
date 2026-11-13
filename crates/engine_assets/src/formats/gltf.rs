//! glTF 2.0 (`.gltf` с внешними файлами и `.glb`). Разбор — крейт `gltf`, но буферы и
//! внешние текстуры читаются через нашу VFS: тот же кеш, сеть и пути, что у всех ресурсов.
//!
//! Результат — данные на CPU. В GPU их переведёт рендер (этап 5).

use std::collections::HashSet;

use engine_core::tasks::BoxedFuture;
use engine_math::{Aabb, Quat, Vec3};

use super::image::{ColorSpace, Image};
use crate::{Asset, AssetError, AssetLoader, Bytes, Handle, LoadContext};

/// Часть меша с одним материалом.
#[derive(Debug, Clone)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    /// Индексы костей и веса для скиннинга (4 кости на вершину).
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// Индекс в `Gltf::materials`.
    pub material: Option<usize>,
    /// Границы в локальных координатах меша — для отсечения.
    pub bounds: Aabb,
}

/// Меш. Вложенный ресурс: путь `файл.glb#Mesh0`.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
}

impl Asset for Mesh {}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.primitives.iter().map(|p| p.positions.len()).sum()
    }
    pub fn triangle_count(&self) -> usize {
        self.primitives.iter().map(|p| p.indices.len() / 3).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask(u8),
    Blend,
}

/// PBR-материал (metallic-roughness, как в glTF). Вложенный ресурс: `файл.glb#Material0`.
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub base_color: [f32; 4],
    pub base_color_texture: Option<Handle<Image>>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_texture: Option<Handle<Image>>,
    pub normal_texture: Option<Handle<Image>>,
    pub occlusion_texture: Option<Handle<Image>>,
    pub emissive: [f32; 3],
    pub emissive_texture: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
}

impl Asset for Material {}

impl Default for Material {
    /// Белый матовый неметалл — как материал по умолчанию в glTF.
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: [1.0; 4],
            base_color_texture: None,
            metallic: 1.0,
            roughness: 1.0,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive: [0.0; 3],
            emissive_texture: None,
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
        }
    }
}

/// Узел иерархии сцены.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    pub children: Vec<usize>,
}

/// Скелет: узлы-кости.
#[derive(Debug, Clone)]
pub struct Skin {
    pub name: String,
    pub joints: Vec<usize>,
    /// Обратные матрицы привязки: переводят вершину из пространства меша в пространство кости.
    pub inverse_bind_matrices: Vec<engine_math::Mat4>,
}

/// Загруженный файл glTF.
#[derive(Debug, Clone)]
pub struct Gltf {
    pub meshes: Vec<Handle<Mesh>>,
    pub materials: Vec<Handle<Material>>,
    pub images: Vec<Handle<Image>>,
    pub nodes: Vec<Node>,
    pub skins: Vec<Skin>,
    /// Корневые узлы каждой сцены.
    pub scenes: Vec<Vec<usize>>,
    pub default_scene: Option<usize>,
    /// Имена анимаций. Сами анимации — итерация 2.
    pub animations: Vec<String>,
}

impl Asset for Gltf {}

#[derive(Debug)]
pub struct GltfLoader;

impl AssetLoader for GltfLoader {
    type Asset = Gltf;

    fn extensions(&self) -> &[&'static str] {
        &["gltf", "glb"]
    }

    fn load<'a>(
        &'a self,
        bytes: Bytes,
        ctx: &'a mut LoadContext,
    ) -> BoxedFuture<'a, Result<Gltf, AssetError>> {
        Box::pin(async move { load_gltf(&bytes, ctx).await })
    }
}

async fn load_gltf(bytes: &[u8], ctx: &mut LoadContext) -> Result<Gltf, AssetError> {
    let path = ctx.path().clone();
    let gltf::Gltf { document, blob } =
        gltf::Gltf::from_slice(bytes).map_err(|e| AssetError::format(&path, e))?;

    let buffers = read_buffers(&document, blob, ctx).await?;
    let images = load_images(&document, &buffers, ctx)?;
    let materials = read_materials(&document, &images)
        .into_iter()
        .enumerate()
        .map(|(index, material)| ctx.add_labeled(&format!("Material{index}"), material))
        .collect();
    let meshes = read_meshes(&document, &buffers, ctx)?;
    let nodes = read_nodes(&document);

    Ok(Gltf {
        meshes,
        materials,
        images,
        nodes,
        skins: document
            .skins()
            .map(|skin| {
                let reader = skin.reader(|buffer| buffers.get(buffer.index()).map(|b| &b[..]));
                Skin {
                    name: skin.name().unwrap_or_default().to_owned(),
                    joints: skin.joints().map(|j| j.index()).collect(),
                    inverse_bind_matrices: reader
                        .read_inverse_bind_matrices()
                        .map(|m| {
                            m.map(|c| engine_math::Mat4::from_cols_array_2d(&c))
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect(),
        scenes: document
            .scenes()
            .map(|scene| scene.nodes().map(|n| n.index()).collect())
            .collect(),
        default_scene: document.default_scene().map(|s| s.index()),
        animations: document
            .animations()
            .map(|a| a.name().unwrap_or_default().to_owned())
            .collect(),
    })
}

/// Буферы: встроенный блок `.glb` или внешние `.bin`.
async fn read_buffers(
    document: &gltf::Document,
    blob: Option<Vec<u8>>,
    ctx: &mut LoadContext,
) -> Result<Vec<Bytes>, AssetError> {
    let mut buffers = Vec::with_capacity(document.buffers().len());
    for buffer in document.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => blob
                .as_deref()
                .map(Bytes::from)
                .ok_or_else(|| AssetError::format(ctx.path(), "нет бинарного блока glb"))?,
            gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => {
                return Err(AssetError::format(
                    ctx.path(),
                    "встроенные data: URI не поддерживаются — используйте .glb",
                ));
            }
            gltf::buffer::Source::Uri(uri) => ctx.read(&decode_uri(uri)).await?,
        };
        buffers.push(data);
    }
    Ok(buffers)
}

/// Изображения: встроенные декодируются сразу, внешние грузятся отдельными ресурсами.
/// Цветовое пространство — по тому, как текстура используется в материалах.
fn load_images(
    document: &gltf::Document,
    buffers: &[Bytes],
    ctx: &mut LoadContext,
) -> Result<Vec<Handle<Image>>, AssetError> {
    let linear = linear_images(document);
    let mut images = Vec::new();
    for image in document.images() {
        let color_space = if linear.contains(&image.index()) {
            ColorSpace::Linear
        } else {
            ColorSpace::Srgb
        };
        let handle = match image.source() {
            gltf::image::Source::View { view, .. } => {
                let buffer = &buffers[view.buffer().index()];
                let data = &buffer[view.offset()..view.offset() + view.length()];
                let decoded = Image::decode(data, color_space)
                    .map_err(|e| AssetError::format(ctx.path(), e))?;
                ctx.add_labeled(&format!("Image{}", image.index()), decoded)
            }
            // TODO(R-08): цветовое пространство для внешних текстур (сейчас всегда sRGB)
            gltf::image::Source::Uri { uri, .. } => ctx.load::<Image>(&decode_uri(uri)),
        };
        images.push(handle);
    }
    Ok(images)
}

fn read_materials(document: &gltf::Document, images: &[Handle<Image>]) -> Vec<Material> {
    let image = |texture: gltf::Texture<'_>| images[texture.source().index()].clone();
    document
        .materials()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            Material {
                name: material.name().unwrap_or_default().to_owned(),
                base_color: pbr.base_color_factor(),
                base_color_texture: pbr.base_color_texture().map(|t| image(t.texture())),
                metallic: pbr.metallic_factor(),
                roughness: pbr.roughness_factor(),
                metallic_roughness_texture: pbr
                    .metallic_roughness_texture()
                    .map(|t| image(t.texture())),
                normal_texture: material.normal_texture().map(|t| image(t.texture())),
                occlusion_texture: material.occlusion_texture().map(|t| image(t.texture())),
                emissive: material.emissive_factor(),
                emissive_texture: material.emissive_texture().map(|t| image(t.texture())),
                alpha_mode: match material.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    gltf::material::AlphaMode::Mask => AlphaMode::Mask(
                        (material.alpha_cutoff().unwrap_or(0.5).clamp(0.0, 1.0) * 255.0) as u8,
                    ),
                    gltf::material::AlphaMode::Blend => AlphaMode::Blend,
                },
                double_sided: material.double_sided(),
            }
        })
        .collect()
}

fn read_meshes(
    document: &gltf::Document,
    buffers: &[Bytes],
    ctx: &mut LoadContext,
) -> Result<Vec<Handle<Mesh>>, AssetError> {
    let mut meshes = Vec::new();
    for mesh in document.meshes() {
        let mut primitives = Vec::new();
        for primitive in mesh.primitives() {
            // [Rust] Замыкание даёт ридеру доступ к буферам по индексу
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|b| &b[..]));
            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or_else(|| AssetError::format(ctx.path(), "у примитива нет позиций"))?
                .collect();
            let indices = reader.read_indices().map_or_else(
                || (0..u32::try_from(positions.len()).unwrap_or(u32::MAX)).collect(),
                |indices| indices.into_u32().collect(),
            );
            let bounds = Aabb::from_points(positions.iter().map(|p| Vec3::from_array(*p)));
            primitives.push(Primitive {
                bounds,
                normals: reader
                    .read_normals()
                    .map(Iterator::collect)
                    .unwrap_or_default(),
                tangents: reader
                    .read_tangents()
                    .map(Iterator::collect)
                    .unwrap_or_default(),
                uvs: reader
                    .read_tex_coords(0)
                    .map(|uv| uv.into_f32().collect())
                    .unwrap_or_default(),
                joints: reader
                    .read_joints(0)
                    .map(|j| j.into_u16().collect())
                    .unwrap_or_default(),
                weights: reader
                    .read_weights(0)
                    .map(|w| w.into_f32().collect())
                    .unwrap_or_default(),
                positions,
                indices,
                material: primitive.material().index(),
            });
        }
        let name = mesh.name().unwrap_or_default().to_owned();
        meshes.push(ctx.add_labeled(&format!("Mesh{}", mesh.index()), Mesh { name, primitives }));
    }
    Ok(meshes)
}

fn read_nodes(document: &gltf::Document) -> Vec<Node> {
    document
        .nodes()
        .map(|node| {
            let (translation, rotation, scale) = node.transform().decomposed();
            Node {
                name: node.name().unwrap_or_default().to_owned(),
                translation: Vec3::from_array(translation),
                rotation: Quat::from_array(rotation),
                scale: Vec3::from_array(scale),
                mesh: node.mesh().map(|m| m.index()),
                skin: node.skin().map(|s| s.index()),
                children: node.children().map(|c| c.index()).collect(),
            }
        })
        .collect()
}

/// Изображения, которые хранят данные, а не цвет: нормали, металличность, затенение.
fn linear_images(document: &gltf::Document) -> HashSet<usize> {
    let mut linear = HashSet::new();
    for material in document.materials() {
        let mut add = |texture: gltf::Texture<'_>| {
            linear.insert(texture.source().index());
        };
        if let Some(t) = material.normal_texture() {
            add(t.texture());
        }
        if let Some(t) = material.occlusion_texture() {
            add(t.texture());
        }
        if let Some(t) = material
            .pbr_metallic_roughness()
            .metallic_roughness_texture()
        {
            add(t.texture());
        }
    }
    linear
}

/// URI в glTF кодируются как URL: пробел — `%20`.
fn decode_uri(uri: &str) -> String {
    let mut out = Vec::with_capacity(uri.len());
    let bytes = uri.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&uri[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
