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
