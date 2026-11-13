//! Системы главного потока: из ECS — в команды и снимок кадра для рендера.

use std::collections::HashMap;

use engine_assets::formats::{Image, Material, Mesh};
use engine_assets::{AssetEvent, Assets};
use engine_core::{FixedTime, Time};
use engine_ecs::prelude::*;
use engine_math::Mat4;
use engine_platform::Instant;
use engine_scene::{
    AmbientLight, Camera, DirectionalLight, GlobalTransform, Mesh3d, PointLight, SkinnedMesh,
    Visibility, is_visible,
};

use crate::ClearColor;
use crate::command::{MaterialData, RenderCommand};
use crate::frontend::RenderFrontend;
use crate::snapshot::{CameraSnapshot, DrawItem};

/// Загруженные и выгруженные ресурсы → команды рендеру.
pub(crate) fn sync_assets(
    frontend: NonSend<RenderFrontend>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    materials: Res<Assets<Material>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    mut material_events: MessageReader<AssetEvent<Material>>,
) {
    let _zone = tracing::trace_span!("sync_assets").entered();
    let send = |command| {
        frontend.send(command);
    };
    for event in image_events.read() {
        match event {
            AssetEvent::Loaded(id) => {
                if let Some(image) = images
                    .get_by_id(*id)
                    .filter(engine_assets::Resolved::is_ready)
                {
                    send(RenderCommand::UploadImage(
                        *id,
                        Box::new(image.asset.clone()),
                    ));
                }
            }
            AssetEvent::Unloaded(id) => send(RenderCommand::RemoveImage(*id)),
            AssetEvent::Failed(..) => {}
        }
    }
    for event in mesh_events.read() {
        match event {
            AssetEvent::Loaded(id) => {
                if let Some(mesh) = meshes
                    .get_by_id(*id)
                    .filter(engine_assets::Resolved::is_ready)
                {
                    send(RenderCommand::UploadMesh(*id, Box::new(mesh.asset.clone())));
                }
            }
            AssetEvent::Unloaded(id) => send(RenderCommand::RemoveMesh(*id)),
            AssetEvent::Failed(..) => {}
        }
    }
    for event in material_events.read() {
        match event {
            AssetEvent::Loaded(id) => {
                if let Some(material) = materials
                    .get_by_id(*id)
                    .filter(engine_assets::Resolved::is_ready)
                {
                    send(RenderCommand::UploadMaterial(
                        *id,
                        MaterialData::from(material.asset),
                    ));
                }
            }
            AssetEvent::Unloaded(id) => send(RenderCommand::RemoveMaterial(*id)),
            AssetEvent::Failed(..) => {}
        }
    }
}

/// Максимум костей на скелет — размер массива в шейдере.
pub(crate) const MAX_JOINTS: usize = 256;

/// Матрицы костей: мировая матрица кости × обратная матрица привязки.
fn push_joints(
    joints: &mut Vec<Mat4>,
    skin: &SkinnedMesh,
    globals: &Query<&GlobalTransform>,
) -> (u32, u32) {
    let start = u32::try_from(joints.len()).unwrap_or(u32::MAX);
    for (entity, inverse_bind) in skin
        .joints
        .iter()
        .zip(&skin.inverse_bind_matrices)
        .take(MAX_JOINTS)
    {
        let global = globals.get(*entity).map_or(Mat4::IDENTITY, |g| g.0);
        joints.push(global * *inverse_bind);
    }
    let count = u32::try_from(joints.len()).unwrap_or(u32::MAX) - start;
    (start, count)
}

/// Трансформ объекта на двух последних шагах логики — для интерполяции.
#[derive(Default)]
pub(crate) struct InterpolationState {
    steps: u64,
    transforms: HashMap<Entity, (Mat4, Mat4)>,
    /// `None` до первого кадра: тогда «предыдущий» цвет = текущему, иначе фон первых
    /// кадров смешивался бы с нулевым (прозрачным чёрным) цветом.
    clear_color: Option<(crate::Color, crate::Color)>,
}

impl InterpolationState {
    /// Сдвигает «текущее → предыдущее», если с прошлого раза был шаг логики.
    fn update(&mut self, entity: Entity, current: Mat4, stepped: bool) -> (Mat4, Mat4) {
        let entry = self.transforms.entry(entity).or_insert((current, current));
        if stepped {
            entry.0 = entry.1;
        }
        entry.1 = current;
        *entry
    }
}

/// Заполняет снимок кадра и публикует его рендеру.
#[allow(clippy::too_many_arguments)] // система ECS: параметры — это запрашиваемые данные
pub(crate) fn extract_frame(
    clear_color: Res<ClearColor>,
    time: Res<Time>,
    fixed: Res<FixedTime>,
    ambient: Res<AmbientLight>,
    meshes: Res<Assets<Mesh>>,
    cameras: Query<(Entity, &Camera, &GlobalTransform)>,
    suns: Query<(&DirectionalLight, &GlobalTransform)>,
    points: Query<(&PointLight, &GlobalTransform)>,
    objects: Query<(Entity, &Mesh3d, &GlobalTransform, Option<&SkinnedMesh>)>,
    globals: Query<&GlobalTransform>,
    visibility: Query<(&Visibility, Option<&ChildOf>)>,
    mut frontend: NonSendMut<RenderFrontend>,
    mut debug: ResMut<crate::DebugDraw>,
    // [Rust] `Local` — состояние, принадлежащее этой системе (как static-переменная внутри
    // функции, но своя у каждого экземпляра системы)
    mut state: Local<InterpolationState>,
) {
    let _zone = tracing::trace_span!("extract_frame").entered();
    let stepped = fixed.steps_total() != state.steps;
    state.steps = fixed.steps_total();
    let (prev_clear, curr_clear) = match state.clear_color {
        None => (clear_color.0, clear_color.0),
        Some((_, curr)) if stepped => (curr, clear_color.0),
        Some(colors) => colors,
    };
    state.clear_color = Some((prev_clear, curr_clear));

    let snapshot = frontend.snapshots.write();
    snapshot.frame = time.frame();
    snapshot.prev_clear_color = prev_clear;
    snapshot.clear_color = curr_clear;
    snapshot.step = fixed.step();
    snapshot.overstep = fixed.overstep_fraction();
    snapshot.published_at = Some(Instant::now());

    // Камера с наименьшим order
    snapshot.camera = cameras
        .iter()
        .min_by_key(|(_, camera, _)| camera.order)
        .map(|(entity, camera, global)| {
            let (prev, curr) = state.update(entity, global.0, stepped);
            CameraSnapshot {
                prev,
                curr,
                fov_y: camera.fov_y,
                near: camera.near,
                far: camera.far,
            }
        });

    let lights = &mut snapshot.lights;
    lights.ambient = ambient.color.map(|c| c * ambient.intensity);
    lights.sun = suns
        .iter()
        .next()
        .map(|(sun, global)| (global.forward(), sun.color.map(|c| c * sun.intensity)));
    lights.points.clear();
    lights.points.extend(points.iter().map(|(light, global)| {
        (
            global.translation(),
            light.range,
            light.color.map(|c| c * light.intensity),
        )
    }));

    snapshot.draws.clear();
    snapshot.joints.clear();
    for (entity, mesh3d, global, skinned) in &objects {
        if !is_visible(entity, &visibility) {
            continue;
        }
        let Some(mesh) = meshes.get_loaded(&mesh3d.mesh) else {
            continue;
        };
        let (prev, curr) = state.update(entity, global.0, stepped);
        let skin = skinned
            .filter(|_| mesh.primitives.iter().any(|p| !p.joints.is_empty()))
            .map(|skin| push_joints(&mut snapshot.joints, skin, &globals));
        for (index, _) in mesh.primitives.iter().enumerate() {
            snapshot.draws.push(DrawItem {
                mesh: mesh3d.mesh.id(),
                primitive: u32::try_from(index).unwrap_or(u32::MAX),
                material: mesh3d
                    .materials
                    .get(index)
                    .and_then(|m| m.as_ref().map(engine_assets::Handle::id)),
                prev,
                curr,
                skin,
            });
        }
    }
    // Линии живут один кадр: переносим в снимок и очищаем
    snapshot.lines.clear();
    snapshot.lines.append(&mut debug.vertices);
    frontend.snapshots.publish();

    // Удалённые сущности не должны копиться в состоянии интерполяции
    if time.frame().is_multiple_of(600) {
        state
            .transforms
            .retain(|entity, _| objects.contains(*entity) || cameras.contains(*entity));
    }
}
