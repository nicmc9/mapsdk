//! Геометрия для отсечения и границ: AABB, плоскость, фрустум.

use crate::{Mat4, Vec3, Vec4};

/// Ограничивающий параллелепипед, выровненный по осям (axis-aligned bounding box).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// «Пустой» AABB: объединение с любой точкой даст эту точку.
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    /// Границы набора точек. Для пустого набора — [`Aabb::EMPTY`].
    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        points.into_iter().fold(Self::EMPTY, |aabb, point| Self {
            min: aabb.min.min(point),
            max: aabb.max.max(point),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Половина размеров.
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// AABB после преобразования: охватывает повёрнутый параллелепипед.
    /// Метод Арво: без перебора восьми углов.
    #[must_use]
    pub fn transformed(&self, matrix: &Mat4) -> Self {
        if self.is_empty() {
            return *self;
        }
        let center = matrix.transform_point3(self.center());
        let half = self.half_extents();
        // Модули элементов матрицы поворота-масштаба × половины размеров
        let extent = matrix.x_axis.truncate().abs() * half.x
            + matrix.y_axis.truncate().abs() * half.y
            + matrix.z_axis.truncate().abs() * half.z;
        Self {
            min: center - extent,
            max: center + extent,
        }
    }
}

/// Плоскость `normal · p + d = 0`; положительная сторона — куда смотрит нормаль.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub d: f32,
}

impl Plane {
    fn from_vec4(v: Vec4) -> Self {
        let length = v.truncate().length();
        Self {
            normal: v.truncate() / length,
            d: v.w / length,
        }
    }

    pub fn distance(&self, point: Vec3) -> f32 {
        self.normal.dot(point) + self.d
    }
}

/// Пирамида видимости камеры: 6 плоскостей с нормалями внутрь.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Из матрицы «проекция × вид» (глубина 0..1, как в wgpu/Vulkan/Metal).
    /// Метод Грибба–Хартманна: плоскости — суммы и разности строк матрицы.
    pub fn from_view_projection(view_projection: &Mat4) -> Self {
        let m = view_projection.transpose(); // строки исходной матрицы
        let (r0, r1, r2, r3) = (m.x_axis, m.y_axis, m.z_axis, m.w_axis);
        Self {
            planes: [
                Plane::from_vec4(r3 + r0), // левая
                Plane::from_vec4(r3 - r0), // правая
                Plane::from_vec4(r3 + r1), // нижняя
                Plane::from_vec4(r3 - r1), // верхняя
                Plane::from_vec4(r2),      // ближняя (глубина 0)
                Plane::from_vec4(r3 - r2), // дальняя
            ],
        }
    }

    /// `false` — AABB точно вне фрустума. `true` — виден или пересекает границу
    /// (консервативно: редкие объекты за углом фрустума тоже вернут `true`).
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        let center = aabb.center();
        let half = aabb.half_extents();
        self.planes.iter().all(|plane| {
            // «Радиус» AABB вдоль нормали плоскости
            let radius = half.dot(plane.normal.abs());
            plane.distance(center) >= -radius
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_from_points_and_transform() {
        let aabb = Aabb::from_points([Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 3.0)]);
        assert_eq!(aabb.center(), Vec3::new(0.0, 1.0, 1.5));
        let moved = aabb.transformed(&Mat4::from_translation(Vec3::X * 10.0));
        assert_eq!(moved.min, Vec3::new(9.0, 0.0, 0.0));
        // Поворот на 90° вокруг Y: размеры по X и Z меняются местами
        let rotated = Aabb::from_points([Vec3::ZERO, Vec3::new(4.0, 1.0, 2.0)])
            .transformed(&Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2));
        let size = rotated.max - rotated.min;
        assert!((size - Vec3::new(2.0, 1.0, 4.0)).abs().max_element() < 1e-5);
        assert!(Aabb::EMPTY.is_empty());
    }

    #[test]
    fn frustum_culls_objects_behind_and_aside() {
        let view = crate::camera::look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let projection = crate::camera::perspective(1.0, 1.0, 0.1, 100.0);
        let frustum = Frustum::from_view_projection(&(projection * view));
        let unit = |center: Vec3| Aabb {
            min: center - Vec3::splat(0.5),
            max: center + Vec3::splat(0.5),
        };
        assert!(frustum.intersects_aabb(&unit(Vec3::ZERO)), "перед камерой");
        assert!(
            !frustum.intersects_aabb(&unit(Vec3::new(0.0, 0.0, 10.0))),
            "за камерой"
        );
        assert!(
            !frustum.intersects_aabb(&unit(Vec3::new(50.0, 0.0, 0.0))),
            "сбоку"
        );
        assert!(
            !frustum.intersects_aabb(&unit(Vec3::new(0.0, 0.0, -200.0))),
            "дальше far"
        );
        assert!(
            frustum.intersects_aabb(&unit(Vec3::new(2.4, 0.0, 0.0))),
            "на границе"
        );
    }
}
