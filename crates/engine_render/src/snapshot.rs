use engine_platform::{Duration, Instant};

use crate::Color;

/// Состояние кадра, которое логика передаёт рендеру через тройной буфер.
///
/// Здесь только то, что меняется каждый кадр. Постоянные объекты (меши, текстуры)
/// будут создаваться командами, а в снимке — только их id.
#[derive(Debug, Clone, Default)]
pub(crate) struct FrameSnapshot {
    /// Номер кадра логики (для отладки и статистики).
    pub frame: u64,
    /// Цвет фона на предыдущем и текущем шаге логики — для интерполяции.
    pub prev_clear_color: Color,
    pub clear_color: Color,
    /// Длительность шага логики.
    pub step: Duration,
    /// Когда снимок опубликован и насколько реальное время на тот момент ушло вперёд
    /// от последнего шага (доля шага). `None` — снимок ещё ни разу не публиковался.
    pub published_at: Option<Instant>,
    pub overstep: f32,
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

    pub fn interpolated_clear_color(&self, now: Instant) -> Color {
        self.prev_clear_color
            .lerp(self.clear_color, self.alpha(now))
    }
}
