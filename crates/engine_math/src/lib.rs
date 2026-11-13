//! Математика движка.
//!
//! Типы — псевдонимы на `glam` (аналог `using vec3 = glm::vec3;`). Проект использует
//! только имена из этого модуля, поэтому замена библиотеки затронет один файл.
//!
//! [Rust] `glam` хранит `Vec3A`, `Vec4`, `Quat`, `Mat4` в SIMD-регистрах (SSE2/NEON/wasm simd128).
//! `Vec3` — 12 байт без SIMD (удобно в вершинах и GPU-буферах), `Vec3A` — 16 байт с SIMD
//! (быстрее в вычислениях на CPU).

pub type Vec2 = glam::Vec2;
pub type Vec3 = glam::Vec3;
pub type Vec3A = glam::Vec3A;
pub type Vec4 = glam::Vec4;
pub type IVec2 = glam::IVec2;
pub type UVec2 = glam::UVec2;
pub type Quat = glam::Quat;
pub type Mat3 = glam::Mat3;
pub type Mat4 = glam::Mat4;

// Короткие конструкторы: vec3(1.0, 2.0, 3.0)
pub use glam::{vec2, vec3, vec4};

mod geometry;
pub use geometry::{Aabb, Frustum, Plane};

/// Матрицы камеры в соглашениях wgpu: правая система координат, Y вверх, глубина 0..1.
///
/// [Rust/glam] В glam 0.34 эти функции живут в `glam::camera`, разделённые по API:
/// у OpenGL глубина −1..1, у Vulkan Y вниз. wgpu совпадает с вариантом `directx`.
pub mod camera {
    use crate::{Mat4, Vec3};

    /// Матрица вида: камера в `eye` смотрит на `target`.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(eye, target, up)
    }

    /// Перспективная проекция. `fov_y` — вертикальный угол обзора в радианах.
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective(fov_y, aspect, near, far)
    }
}
