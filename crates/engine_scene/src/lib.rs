//! Сцена в ECS (`docs/acrhitecture.md`, раздел 6).
//!
//! - [`Transform`] — положение относительно родителя, [`GlobalTransform`] — в мире
//!   (считается в `PostUpdate`).
//! - Иерархия — компоненты `ChildOf`/`Children` из ECS. Отдельного графа сцены нет.
//! - [`Camera`], [`DirectionalLight`], [`PointLight`], ресурс [`AmbientLight`].
//! - [`SceneRoot`] — разворачивает glTF в дерево сущностей, когда файл загрузится.

mod animation;
mod components;
mod gltf_scene;
mod propagate;

use engine_core::{App, Plugin, Stage};
use engine_ecs::prelude::IntoScheduleConfigs;

pub use animation::{AnimationPlayer, SceneNodes};
pub use components::{
    AmbientLight, Camera, DirectionalLight, GlobalTransform, Mesh3d, PointLight, Transform,
    Visibility,
};
pub use gltf_scene::{SceneRoot, SceneSpawned, SkinnedMesh};
pub use propagate::{is_visible, propagate_transforms};

/// Подключает сцену. Требует `AssetPlugin` (для glTF).
#[derive(Debug, Default)]
pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AmbientLight>()
            .add_systems(Stage::PreUpdate, gltf_scene::spawn_gltf_scenes)
            .add_systems(Stage::FixedUpdate, animation::animate)
            .add_systems(
                Stage::PostUpdate,
                (
                    propagate::propagate_transforms,
                    gltf_scene::bind_pose_transforms,
                )
                    .chain(),
            );
    }
}
