//! Сцены sandbox.

use std::borrow::Cow;

use engine::CVarString;
use engine::assets::formats::Mesh;
use engine::math::{Aabb, Vec3};
use engine::prelude::*;

use crate::camera::OrbitCamera;

pub static SCENE: CVar<CVarString> = CVar::new(
    "sandbox.scene",
    Cow::Borrowed("showcase"),
    "Сцена: showcase, sponza, helmet",
);

/// Вписать модель: высота и точка, куда поставить основание.
#[derive(Component)]
pub struct FitTo {
    pub height: f32,
    pub position: Vec3,
}

pub fn setup(mut commands: Commands, server: Res<AssetServer>, mut ambient: ResMut<AmbientLight>) {
    let scene = SCENE.get();
    engine::log::info!("Сцена: {scene}");
    let mut model = |path: &str, height: f32, position: Vec3| {
        commands.spawn((
            SceneRoot(server.load(path)),
            FitTo { height, position },
            Transform::default(),
        ));
    };
    let (target, distance) = match scene.as_ref() {
        "sponza" => {
            model("models/khronos/Sponza/Sponza.gltf", 12.0, Vec3::ZERO);
            ambient.intensity = 0.3;
            (Vec3::new(0.0, 2.0, 0.0), 6.0)
        }
        "helmet" => {
            model(
                "models/khronos/DamagedHelmet/DamagedHelmet.glb",
                2.0,
                Vec3::ZERO,
            );
            (Vec3::new(0.0, 1.0, 0.0), 4.0)
        }
        _ => {
            model(
                "models/khronos/DamagedHelmet/DamagedHelmet.glb",
                1.6,
                Vec3::new(0.0, 0.0, 0.0),
            );
            model("models/orc/orc.glb", 2.0, Vec3::new(-2.2, 0.0, 0.0));
            model("models/khronos/Fox/Fox.glb", 1.0, Vec3::new(2.2, 0.0, 0.0));
            model(
                "https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models/Duck/glTF-Binary/Duck.glb",
                0.8,
                Vec3::new(1.2, 0.0, 1.6),
            );
            model(
                "models/khronos/BoxTextured/BoxTextured.glb",
                0.6,
                Vec3::new(-1.2, 0.0, 1.6),
            );
            (Vec3::new(0.0, 0.9, 0.0), 6.5)
        }
    };

    commands.spawn((
        Camera::default(),
        OrbitCamera::new(target, distance),
        Transform::default(),
    ));
    commands.spawn((
        DirectionalLight::default(),
        Transform::from_xyz(4.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        PointLight {
            color: [1.0, 0.6, 0.3],
            intensity: 30.0,
            range: 12.0,
        },
        Transform::from_xyz(-3.0, 2.5, 2.0),
    ));
    commands.spawn((
        PointLight {
            color: [0.3, 0.6, 1.0],
            intensity: 30.0,
            range: 12.0,
        },
        Transform::from_xyz(3.0, 2.0, 3.0),
    ));
}

/// Когда сцена модели развернулась, считает её границы и масштабирует корень так,
/// чтобы модель стояла основанием на точке `position` и имела высоту `height`.
pub fn fit_models(
    mut commands: Commands,
    mut roots: Query<(Entity, &FitTo, &mut Transform, Ref<SceneSpawned>)>,
    children: Query<&Children>,
    objects: Query<(&Mesh3d, &GlobalTransform)>,
    meshes: Res<Assets<Mesh>>,
) {
    for (root, fit, mut transform, spawned) in &mut roots {
        // [Rust] `Ref<T>` — доступ на чтение с флагами изменений. В кадре, когда сцену
        // развернули, мировые трансформы потомков ещё не посчитаны — ждём следующий.
        if spawned.is_added() {
            continue;
        }
        // Границы в мире при текущем (единичном) трансформе корня
        let mut bounds = Aabb::EMPTY;
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok((mesh3d, global)) = objects.get(entity)
                && let Some(mesh) = meshes.get_loaded(&mesh3d.mesh)
            {
                for primitive in &mesh.primitives {
                    bounds = bounds.union(&primitive.bounds.transformed(&global.0));
                }
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        if bounds.is_empty() {
            continue;
        }
        let size = bounds.max - bounds.min;
        let scale = fit.height / size.y.max(1e-4);
        let base = Vec3::new(bounds.center().x, bounds.min.y, bounds.center().z);
        transform.scale = Vec3::splat(scale);
        transform.translation = fit.position - base * scale;
        engine::log::info!(
            "Модель вписана: размер {:.2?} → масштаб {scale:.4}",
            size.to_array()
        );
        commands.entity(root).remove::<FitTo>();
    }
}
