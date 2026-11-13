use engine_ecs::prelude::*;
use engine_platform::{Duration, Instant};

/// Время кадра (переменный шаг). Доступно системам как `Res<Time>`.
#[derive(Resource, Debug)]
pub struct Time {
    startup: Instant,
    last_update: Option<Instant>,
    delta: Duration,
    elapsed: Duration,
    frame: u64,
}

impl Time {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            startup: now,
            last_update: None,
            delta: Duration::ZERO,
            elapsed: Duration::ZERO,
            frame: 0,
        }
    }

    pub(crate) fn update(&mut self, now: Instant) {
        // Первый кадр — нулевой delta, а не время с момента создания App
        // [Rust] `saturating_*` — арифметика без переполнения: вместо паники или «заворота» результат
        // упирается в 0. Часы могут вернуть время раньше прошлого (монотонные часы не должны,
        // но некоторые платформы грешат), поэтому вычитаем безопасно.
        self.delta = self
            .last_update
            .map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
        self.last_update = Some(now);
        self.elapsed = now.saturating_duration_since(self.startup);
        self.frame += 1;
    }

    /// Следующий кадр получит нулевой `delta`. Вызывается при возврате из фона:
    /// время в фоне не должно попадать в логику, иначе она попытается его «догнать».
    pub(crate) fn skip_gap(&mut self) {
        self.last_update = None;
    }

    /// Время с прошлого кадра.
    pub fn delta(&self) -> Duration {
        self.delta
    }

    pub fn delta_secs(&self) -> f32 {
        self.delta.as_secs_f32()
    }

    /// Время с запуска.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Номер кадра, начиная с 1.
    pub fn frame(&self) -> u64 {
        self.frame
    }
}

/// Фиксированный шаг логики. Доступен системам как `Res<FixedTime>`.
///
/// Реальное время копится в аккумуляторе и «расходуется» шагами по `step`.
/// Остаток (`overstep`) показывает, насколько реальное время ушло вперёд от последнего
/// шага — по нему рендер интерполирует положение объектов.
#[derive(Resource, Debug)]
pub struct FixedTime {
    step: Duration,
    accumulator: Duration,
    /// Время логики: шагов выполнено × step.
    elapsed: Duration,
    steps_total: u64,
    /// Ограничение шагов за кадр. Защита от «спирали смерти»: если кадр долгий, логика
    /// пытается догнать время, кадр становится ещё дольше, и так по нарастающей.
    max_steps_per_frame: u32,
    /// Сколько реального времени отброшено из-за ограничения (для статистики).
    dropped: Duration,
}

impl FixedTime {
    pub const DEFAULT_HZ: f64 = 60.0;
    pub const DEFAULT_MAX_STEPS: u32 = 5;

    pub fn from_hz(hz: f64) -> Self {
        Self::new(Duration::from_secs_f64(1.0 / hz), Self::DEFAULT_MAX_STEPS)
    }

    pub fn new(step: Duration, max_steps_per_frame: u32) -> Self {
        assert!(!step.is_zero(), "шаг логики не может быть нулевым");
        assert!(
            max_steps_per_frame > 0,
            "должен быть хотя бы один шаг за кадр"
        );
        Self {
            step,
            accumulator: Duration::ZERO,
            elapsed: Duration::ZERO,
            steps_total: 0,
            max_steps_per_frame,
            dropped: Duration::ZERO,
        }
    }

    /// Добавить реальное время кадра. Возвращает, сколько шагов логики выполнить.
    pub(crate) fn accumulate(&mut self, real_delta: Duration) -> u32 {
        self.accumulator += real_delta;
        // [Rust] Деление Duration на Duration в std нет, поэтому считаем через u128 наносекунд.
        let available = self.accumulator.as_nanos() / self.step.as_nanos();
        let steps = u32::try_from(available).unwrap_or(u32::MAX);
        if steps > self.max_steps_per_frame {
            // Лишнее время отбрасываем: логика замедлится, но не уйдёт в «спираль смерти»
            let keep = self.step * self.max_steps_per_frame;
            let excess = self.accumulator.saturating_sub(keep);
            self.dropped += excess;
            self.accumulator = keep;
            log::warn!("Логика не успевает: отброшено {excess:?}");
            return self.max_steps_per_frame;
        }
        steps
    }

    /// Учесть выполненный шаг.
    pub(crate) fn consume_step(&mut self) {
        self.accumulator -= self.step;
        self.elapsed += self.step;
        self.steps_total += 1;
    }

    /// Сколько осталось до следующего шага.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn until_next_step(&self) -> Duration {
        self.step.saturating_sub(self.accumulator)
    }

    pub fn step(&self) -> Duration {
        self.step
    }

    pub fn step_secs(&self) -> f32 {
        self.step.as_secs_f32()
    }

    /// Время логики (сумма выполненных шагов).
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn steps_total(&self) -> u64 {
        self.steps_total
    }

    /// Доля шага, на которую реальное время ушло вперёд от последнего шага: `[0, 1)`.
    /// Коэффициент интерполяции для рендера.
    // Значение в [0, 1), потеря точности при переходе к f32 не важна
    #[allow(clippy::cast_possible_truncation)]
    pub fn overstep_fraction(&self) -> f32 {
        (self.accumulator.as_secs_f64() / self.step.as_secs_f64()) as f32
    }

    pub fn dropped(&self) -> Duration {
        self.dropped
    }
}

impl Default for FixedTime {
    fn default() -> Self {
        Self::from_hz(Self::DEFAULT_HZ)
    }
}

// [Rust] Тесты живут рядом с кодом. `#[cfg(test)]` — модуль компилируется только
// при `cargo test`, в обычную сборку не попадает.
#[cfg(test)]
mod tests {
    use super::*;

    const STEP: Duration = Duration::from_millis(10);

    fn run(fixed: &mut FixedTime, delta: Duration) -> u32 {
        let steps = fixed.accumulate(delta);
        for _ in 0..steps {
            fixed.consume_step();
        }
        steps
    }

    #[test]
    fn steps_match_real_time() {
        let mut fixed = FixedTime::new(STEP, 5);
        assert_eq!(run(&mut fixed, Duration::from_millis(25)), 2);
        // Остаток 5 мс переходит в следующий кадр
        assert_eq!(run(&mut fixed, Duration::from_millis(5)), 1);
        assert_eq!(fixed.steps_total(), 3);
        assert_eq!(fixed.elapsed(), STEP * 3);
    }

    #[test]
    fn no_step_when_frame_is_shorter_than_step() {
        let mut fixed = FixedTime::new(STEP, 5);
        assert_eq!(run(&mut fixed, Duration::from_millis(4)), 0);
        assert!((fixed.overstep_fraction() - 0.4).abs() < 1e-6);
        assert_eq!(fixed.until_next_step(), Duration::from_millis(6));
    }

    #[test]
    fn long_frame_does_not_cause_death_spiral() {
        let mut fixed = FixedTime::new(STEP, 5);
        // Кадр в 1 секунду (например, отладчик или выход из фона)
        assert_eq!(run(&mut fixed, Duration::from_secs(1)), 5);
        assert_eq!(fixed.dropped(), Duration::from_millis(950));
        // После просадки логика сразу возвращается к нормальному темпу
        assert_eq!(run(&mut fixed, STEP), 1);
    }

    #[test]
    fn stable_over_many_uneven_frames() {
        let mut fixed = FixedTime::new(STEP, 5);
        let mut total = Duration::ZERO;
        for i in 0..1000 {
            let delta = Duration::from_micros(7_000 + (i % 7) * 1_000); // 7..13 мс
            total += delta;
            run(&mut fixed, delta);
        }
        // Время логики отстаёт от реального не больше чем на один шаг
        let lag = total.saturating_sub(fixed.elapsed());
        assert!(lag < STEP, "отставание {lag:?}");
    }
}
