//! Переменные конфигурации ядра.

use std::borrow::Cow;

use engine_ecs::prelude::*;
use engine_platform::Duration;

use crate::{CVar, CVarFlags, CVarString, FixedTime};

pub static SIM_HZ: CVar<f64> = CVar::new("sim.hz", 60.0, "Частота шага логики, Гц");

pub static SIM_MAX_STEPS: CVar<u32> = CVar::new(
    "sim.max_steps",
    5,
    "Максимум шагов логики за кадр (защита от «спирали смерти»)",
);

pub static LOG_FILTER: CVar<CVarString> = CVar::new(
    "log.filter",
    Cow::Borrowed(engine_platform::DEFAULT_LOG_FILTER),
    "Фильтр логов: «уровень,модуль=уровень» (как RUST_LOG)",
)
// Меняется на лету: удобно включить debug для одного модуля прямо из консоли
.with_flags(CVarFlags::NONE);

/// Применяет изменения переменных ядра. Выполняется в начале каждого кадра —
/// стоимость: три атомарных чтения.
pub(crate) fn apply_core_settings(
    mut fixed: ResMut<FixedTime>,
    // Версии, применённые в прошлый раз. 0 — переменная ни разу не менялась.
    mut applied: Local<[u64; 3]>,
) {
    let versions = [
        SIM_HZ.version(),
        SIM_MAX_STEPS.version(),
        LOG_FILTER.version(),
    ];
    if versions[0] != applied[0] || versions[1] != applied[1] {
        let hz = SIM_HZ.get();
        if hz.is_finite() && hz > 0.0 {
            fixed.configure(
                Duration::from_secs_f64(1.0 / hz),
                SIM_MAX_STEPS.get().max(1),
            );
            tracing::info!("Логика: {hz} Гц, до {} шагов за кадр", SIM_MAX_STEPS.get());
        } else {
            tracing::error!("sim.hz = {hz}: нужно положительное число");
        }
    }
    // Фильтр логов применяем, только если его меняли: иначе затёрли бы RUST_LOG
    if versions[2] != applied[2] {
        let filter = LOG_FILTER.get();
        match engine_platform::set_log_filter(&filter) {
            Ok(()) => tracing::info!("Фильтр логов: {filter}"),
            Err(error) => tracing::error!("log.filter = «{filter}»: {error}"),
        }
    }
    *applied = versions;
}
