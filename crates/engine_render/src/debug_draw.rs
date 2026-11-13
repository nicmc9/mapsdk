//! Отладочная отрисовка линиями (R-14).
//!
//! ```ignore
//! fn show(mut debug: ResMut<DebugDraw>) {
//!     debug.line(Vec3::ZERO, Vec3::Y, Color::rgb(0.0, 1.0, 0.0));
//!     debug.aabb(&bounds, Color::WHITE);
//! }
//! ```
//!
//! Немедленный режим: линии живут один кадр — добавляйте их в каждом кадре (`Stage::Update`).
//! Встроенные режимы — CVar `debug.grid`, `debug.bounds`, `debug.axes`.

use bytemuck::{Pod, Zeroable};
use engine_assets::Assets;
use engine_assets::formats::Mesh;
use engine_core::CVar;
use engine_ecs::prelude::*;
use engine_math::{Aabb, Mat4, Vec3};
use engine_scene::{GlobalTransform, Mesh3d};

use crate::Color;

pub static DEBUG_GRID: CVar<bool> = CVar::new("debug.grid", false, "Сетка на плоскости y = 0");
pub static DEBUG_BOUNDS: CVar<bool> =
    CVar::new("debug.bounds", false, "Границы (AABB) всех мешей в мире");
pub static DEBUG_AXES: CVar<bool> = CVar::new(
    "debug.axes",
    false,
    "Оси объектов с мешами: X — красная, Y — зелёная, Z — синяя",
);

/// Вершина линии для GPU.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub(crate) struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// Список линий текущего кадра.
#[derive(Resource, Debug, Default)]
pub struct DebugDraw {
    pub(crate) vertices: Vec<LineVertex>,
}

impl DebugDraw {
    pub fn line(&mut self, from: Vec3, to: Vec3, color: Color) {
        let color = [color.r, color.g, color.b, color.a];
        self.vertices.push(LineVertex {
            position: from.to_array(),
            color,
        });
        self.vertices.push(LineVertex {
            position: to.to_array(),
            color,
        });
    }

    /// Рёбра параллелепипеда — 12 линий.
    pub fn aabb(&mut self, aabb: &Aabb, color: Color) {
        if aabb.is_empty() {
            return;
        }
        let (a, b) = (aabb.min, aabb.max);
        let corner = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { b.x } else { a.x },
                if y { b.y } else { a.y },
                if z { b.z } else { a.z },
            )
        };
        for (p, q) in [
            // Рёбра вдоль X, Y и Z
            ((false, false, false), (true, false, false)),
            ((false, true, false), (true, true, false)),
            ((false, false, true), (true, false, true)),
            ((false, true, true), (true, true, true)),
            ((false, false, false), (false, true, false)),
            ((true, false, false), (true, true, false)),
            ((false, false, true), (false, true, true)),
            ((true, false, true), (true, true, true)),
            ((false, false, false), (false, false, true)),
            ((true, false, false), (true, false, true)),
            ((false, true, false), (false, true, true)),
            ((true, true, false), (true, true, true)),
        ] {
            self.line(corner(p.0, p.1, p.2), corner(q.0, q.1, q.2), color);
        }
    }

    /// Оси системы координат объекта длиной `size`.
    pub fn axes(&mut self, transform: &Mat4, size: f32) {
        let origin = transform.transform_point3(Vec3::ZERO);
        for (axis, color) in [
            (Vec3::X, Color::rgb(1.0, 0.2, 0.2)),
            (Vec3::Y, Color::rgb(0.2, 1.0, 0.2)),
            (Vec3::Z, Color::rgb(0.3, 0.5, 1.0)),
        ] {
            let end = origin + transform.transform_vector3(axis).normalize_or_zero() * size;
            self.line(origin, end, color);
        }
    }

    /// Квадратная сетка на плоскости y = 0 с центром в начале координат.
    pub fn grid(&mut self, half_size: i32, step: f32, color: Color) {
        #[allow(clippy::cast_precision_loss)]
        let extent = half_size as f32 * step;
        for i in -half_size..=half_size {
            #[allow(clippy::cast_precision_loss)]
            let offset = i as f32 * step;
            // Центральные линии ярче
            let line_color = if i == 0 {
                Color::rgb(0.6, 0.6, 0.6)
            } else {
                color
            };
            self.line(
                Vec3::new(offset, 0.0, -extent),
                Vec3::new(offset, 0.0, extent),
                line_color,
            );
            self.line(
                Vec3::new(-extent, 0.0, offset),
                Vec3::new(extent, 0.0, offset),
                line_color,
            );
        }
    }

    pub fn len(&self) -> usize {
        self.vertices.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

/// Встроенные режимы отладки (CVar `debug.*`). Стадия `PostUpdate`, после трансформов.
pub(crate) fn builtin_debug_draw(
    mut debug: ResMut<DebugDraw>,
    objects: Query<(&Mesh3d, &GlobalTransform)>,
    meshes: Res<Assets<Mesh>>,
) {
    if DEBUG_GRID.get() {
        debug.grid(10, 1.0, Color::rgb(0.25, 0.25, 0.25));
    }
    let (bounds, axes) = (DEBUG_BOUNDS.get(), DEBUG_AXES.get());
    if !bounds && !axes {
        return;
    }
    for (mesh3d, global) in &objects {
        if axes {
            debug.axes(&global.0, 0.3);
        }
        if bounds && let Some(mesh) = meshes.get_loaded(&mesh3d.mesh) {
            for primitive in &mesh.primitives {
                debug.aabb(
                    &primitive.bounds.transformed(&global.0),
                    Color::rgb(1.0, 0.8, 0.1),
                );
            }
        }
    }
}
