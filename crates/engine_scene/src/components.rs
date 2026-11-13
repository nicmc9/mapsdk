use engine_assets::Handle;
use engine_assets::formats::{Material, Mesh};
use engine_ecs::prelude::*;
use engine_math::{Mat4, Quat, Vec3, camera};

/// Положение, поворот и масштаб относительно родителя (или мира, если родителя нет).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
// [Rust] `#[require(...)]` — при добавлении Transform ECS сам добавит GlobalTransform и
// Visibility со значениями по умолчанию. Не нужно помнить о «парных» компонентах.
#[require(GlobalTransform, Visibility)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn from_xyz(x: f32, y: f32, z: f32) -> Self {
        Self::from_translation(Vec3::new(x, y, z))
    }

    /// Повернуть так, чтобы «вперёд» (−Z) смотрело на `target`.
    #[must_use]
    pub fn looking_at(mut self, target: Vec3, up: Vec3) -> Self {
        // Матрица вида переводит мир в камеру; поворот объекта — обратный к ней
        let view = camera::look_at(self.translation, target, up);
        self.rotation = Quat::from_mat4(&view.inverse());
        self
    }

    #[must_use]
    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Направление «вперёд» (−Z), как у камеры и направленного света.
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Итоговая матрица в мировых координатах. Считается системой — не меняйте вручную.
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct GlobalTransform(pub Mat4);

impl GlobalTransform {
    pub fn translation(&self) -> Vec3 {
        self.0.w_axis.truncate()
    }
    pub fn forward(&self) -> Vec3 {
        (-self.0.z_axis.truncate()).normalize_or_zero()
    }
}

/// Видимость. Скрытый объект не рисуется вместе с потомками.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility(pub bool);

impl Default for Visibility {
    fn default() -> Self {
        Self(true)
    }
}

/// Меш с материалами — по одному на примитив (часть меша). `None` — материал по умолчанию.
#[derive(Component, Debug, Clone)]
#[require(Transform)]
pub struct Mesh3d {
    pub mesh: Handle<Mesh>,
    pub materials: Vec<Option<Handle<Material>>>,
}

/// Перспективная камера. Смотрит вдоль −Z своего `Transform`.
#[derive(Component, Debug, Clone, Copy)]
#[require(Transform)]
pub struct Camera {
    /// Вертикальный угол обзора, радианы.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    /// Камеры с бо́льшим номером рисуются поверх (R-06: несколько камер — позже).
    pub order: i32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov_y: 50f32.to_radians(),
            near: 0.05,
            far: 2000.0,
            order: 0,
        }
    }
}

impl Camera {
    pub fn projection(&self, aspect: f32) -> Mat4 {
        camera::perspective(self.fov_y, aspect.max(1e-4), self.near, self.far)
    }
}

/// Направленный свет (солнце). Светит вдоль −Z своего `Transform`.
#[derive(Component, Debug, Clone, Copy)]
#[require(Transform)]
pub struct DirectionalLight {
    /// Линейный цвет.
    pub color: [f32; 3],
    /// Освещённость (условные единицы: 1 — «пасмурный день», 3–5 — яркое солнце).
    pub intensity: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            color: [1.0, 0.98, 0.95],
            intensity: 3.0,
        }
    }
}

/// Точечный свет.
#[derive(Component, Debug, Clone, Copy)]
#[require(Transform)]
pub struct PointLight {
    pub color: [f32; 3],
    pub intensity: f32,
    /// Дальше этого расстояния свет не действует.
    pub range: f32,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            color: [1.0; 3],
            intensity: 10.0,
            range: 20.0,
        }
    }
}

/// Рассеянный свет — одинаковый со всех сторон.
#[derive(Resource, Debug, Clone, Copy)]
pub struct AmbientLight {
    pub color: [f32; 3],
    pub intensity: f32,
}

impl Default for AmbientLight {
    fn default() -> Self {
        Self {
            color: [0.6, 0.7, 1.0],
            intensity: 0.15,
        }
    }
}
