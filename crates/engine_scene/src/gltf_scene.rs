use engine_assets::formats::{Gltf, Mesh};
use engine_assets::{Assets, Handle};
use engine_ecs::prelude::*;

use crate::{Mesh3d, Transform};

/// Разворачивает сцену glTF в дерево сущностей-потомков, когда файл загрузится.
///
/// ```ignore
/// commands.spawn((SceneRoot(server.load("models/helmet.glb")), Transform::default()));
/// ```
#[derive(Component, Debug, Clone)]
#[require(Transform)]
pub struct SceneRoot(pub Handle<Gltf>);

/// Скелетный меш: кости — сущности-узлы той же сцены.
///
/// Скиннинг (деформация вершин костями) — итерация 2. Пока меш рисуется в позе привязки:
/// матрица меша = мировая матрица первой кости × её обратная матрица привязки. По glTF
/// собственный трансформ узла скелетного меша не используется — поворот и масштаб модели
/// (например, «Z вверх» из 3ds Max или сантиметры Mixamo) приходят через кости.
#[derive(Component, Debug, Clone)]
pub struct SkinnedMesh {
    pub joints: Vec<Entity>,
    pub inverse_bind_matrices: Vec<engine_math::Mat4>,
}

/// Метка: сцена уже развёрнута.
#[derive(Component, Debug)]
pub struct SceneSpawned;

pub(crate) fn spawn_gltf_scenes(
    mut commands: Commands,
    roots: Query<(Entity, &SceneRoot), Without<SceneSpawned>>,
    gltfs: Res<Assets<Gltf>>,
    meshes: Res<Assets<Mesh>>,
) {
    for (root, scene) in &roots {
        let Some(gltf) = gltfs.get_loaded(&scene.0) else {
            continue; // ещё грузится
        };
        // Меши — вложенные ресурсы, применяются в том же кадре, что и файл
        if gltf.meshes.iter().any(|m| meshes.get_loaded(m).is_none()) {
            continue;
        }
        let scene_index = gltf.default_scene.unwrap_or(0);
        let Some(nodes) = gltf.scenes.get(scene_index) else {
            commands.entity(root).insert(SceneSpawned);
            continue;
        };
        // Сначала все узлы (запоминаем узел → сущность), потом скелеты: кости могут
        // оказаться в другой ветке иерархии, чем меш
        let mut entities = vec![None; gltf.nodes.len()];
        for &node in nodes {
            spawn_node(&mut commands, gltf, &meshes, node, root, &mut entities);
        }
        for (index, node) in gltf.nodes.iter().enumerate() {
            let (Some(entity), Some(skin)) =
                (entities[index], node.skin.and_then(|s| gltf.skins.get(s)))
            else {
                continue;
            };
            let joints: Vec<Entity> = skin.joints.iter().filter_map(|&j| entities[j]).collect();
            if joints.len() == skin.joints.len() {
                commands.entity(entity).insert(SkinnedMesh {
                    joints,
                    inverse_bind_matrices: skin.inverse_bind_matrices.clone(),
                });
            }
        }
        commands.entity(root).insert(SceneSpawned);
        tracing::debug!("Сцена glTF развёрнута: {} узлов", gltf.nodes.len());
    }
}

fn spawn_node(
    commands: &mut Commands,
    gltf: &Gltf,
    meshes: &Assets<Mesh>,
    index: usize,
    parent: Entity,
    entities: &mut Vec<Option<Entity>>,
) {
    let node = &gltf.nodes[index];
    let transform = Transform {
        translation: node.translation,
        rotation: node.rotation,
        scale: node.scale,
    };
    let mut entity = commands.spawn((transform, ChildOf(parent), Name::new(node.name.clone())));
    if let Some(mesh_index) = node.mesh {
        let mesh = gltf.meshes[mesh_index].clone();
        let materials = meshes
            .get_loaded(&mesh)
            .map(|m| {
                m.primitives
                    .iter()
                    .map(|p| p.material.and_then(|i| gltf.materials.get(i).cloned()))
                    .collect()
            })
            .unwrap_or_default();
        entity.insert(Mesh3d { mesh, materials });
    }
    let id = entity.id();
    entities[index] = Some(id);
    for &child in &node.children {
        spawn_node(commands, gltf, meshes, child, id, entities);
    }
}

/// Мировая матрица скелетных мешей — из первой кости (поза привязки, см. [`SkinnedMesh`]).
/// Выполняется после расчёта мировых трансформов.
pub(crate) fn bind_pose_transforms(
    mut skinned: Query<(&SkinnedMesh, &mut crate::GlobalTransform)>,
    joints: Query<&crate::GlobalTransform, Without<SkinnedMesh>>,
) {
    for (skin, mut global) in &mut skinned {
        let (Some(&joint), Some(inverse_bind)) =
            (skin.joints.first(), skin.inverse_bind_matrices.first())
        else {
            continue;
        };
        if let Ok(joint_global) = joints.get(joint) {
            global.0 = joint_global.0 * *inverse_bind;
        }
    }
}
