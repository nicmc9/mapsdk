use engine_assets::AssetId;
use engine_assets::formats::{Material, Mesh};
use engine_math::{Mat4, Quat, Vec3};
use engine_platform::{Duration, Instant};

use crate::Color;

/// Состояние кадра, которое логика передаёт рендеру через тройной буфер.
///
/// Только то, что меняется каждый кадр. Меши, текстуры и материалы приходят командами,
/// здесь — только их id. Коллекции очищаются через `clear()` — память переиспользуется.
#[derive(Debug, Clone, Default)]
pub(crate) struct FrameSnapshot {
    pub frame: u64,
    pub prev_clear_color: Color,
    pub clear_color: Color,
    pub step: Duration,
    pub published_at: Option<Instant>,
    pub overstep: f32,
    pub camera: Option<CameraSnapshot>,
    pub lights: LightsSnapshot,
    pub draws: Vec<DrawItem>,
    /// Отладочные линии (пары вершин), уже в мировых координатах.
    pub lines: Vec<crate::debug_draw::LineVertex>,
    pub ui: crate::ui::UiFrame,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CameraSnapshot {
    pub prev: Mat4,
    pub curr: Mat4,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LightsSnapshot {
    pub ambient: [f32; 3],
    /// Направление (куда светит) и цвет × интенсивность.
    pub sun: Option<(Vec3, [f32; 3])>,
    /// Позиция, радиус, цвет × интенсивность.
    pub points: Vec<(Vec3, f32, [f32; 3])>,
}

/// Один примитив меша в мире: трансформ на предыдущем и текущем шаге логики.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrawItem {
    pub mesh: AssetId<Mesh>,
    pub primitive: u32,
    pub material: Option<AssetId<Material>>,
    pub prev: Mat4,
    pub curr: Mat4,
}

impl FrameSnapshot {
    /// Коэффициент интерполяции между предыдущим и текущим шагом на момент `now`.
    ///
    /// Рендер показывает мир с задержкой в один шаг: в момент последнего шага
    /// alpha = 0 (показываем предыдущее состояние), через шаг — 1 (текущее).
    pub fn alpha(&self, now: Instant) -> f32 {
        let Some(published_at) = self.published_at else {
            return 1.0;
        };
        if self.step.is_zero() {
            return 1.0;
        }
        let since_publish = now.saturating_duration_since(published_at);
        let alpha = self.overstep + since_publish.as_secs_f32() / self.step.as_secs_f32();
        alpha.clamp(0.0, 1.0)
    }

    pub fn interpolated_clear_color(&self, alpha: f32) -> Color {
        self.prev_clear_color.lerp(self.clear_color, alpha)
    }
}

/// Интерполяция трансформа: позиция и масштаб линейно, поворот — сферически (slerp).
/// Линейная интерполяция матриц целиком искажает поворот (объект «сжимается» по пути).
pub(crate) fn interpolate(prev: &Mat4, curr: &Mat4, alpha: f32) -> Mat4 {
    if alpha >= 1.0 || prev == curr {
        return *curr;
    }
    let (s0, r0, t0): (Vec3, Quat, Vec3) = prev.to_scale_rotation_translation();
    let (s1, r1, t1) = curr.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(
        s0.lerp(s1, alpha),
        r0.slerp(r1, alpha),
        t0.lerp(t1, alpha),
    )
}
