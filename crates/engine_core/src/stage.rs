use engine_ecs::ScheduleLabel;
// Нужен для derive(ScheduleLabel): см. комментарий в engine_ecs::prelude
#[allow(unused_imports)]
use engine_ecs::prelude::*;

/// Стадии кадра в порядке выполнения. Каждая стадия — отдельное расписание систем.
///
/// ```text
/// Startup (один раз)
/// First → PreUpdate → FixedUpdate × N → Update → PostUpdate → Extract → Last
/// ```
///
/// `FixedUpdate` выполняется 0, 1 или несколько раз за кадр — столько, сколько шагов
/// логики накопилось (см. [`FixedTime`](crate::FixedTime)). Всё, что должно двигаться
/// плавно (трансформы, физика), меняется только здесь, иначе интерполяция в рендере сломается.
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Один раз перед первым кадром.
    Startup,
    /// Служебная: время, обмен буферов сообщений.
    First,
    /// Ввод, события платформы.
    PreUpdate,
    /// Логика с фиксированным шагом: физика, движение, игровые правила.
    FixedUpdate,
    /// Логика с частотой кадров: UI, камера, реакция на ввод.
    Update,
    /// Распространение трансформов, подготовка к извлечению.
    PostUpdate,
    /// Заполнение снимка кадра для рендера.
    Extract,
    /// Служебная: конец кадра.
    Last,
}

impl Stage {
    /// Стадии, выполняемые каждый кадр (без `Startup` и `FixedUpdate` — у них свои правила).
    pub(crate) const PER_FRAME_BEFORE_FIXED: [Stage; 2] = [Stage::First, Stage::PreUpdate];
    pub(crate) const PER_FRAME_AFTER_FIXED: [Stage; 4] = [
        Stage::Update,
        Stage::PostUpdate,
        Stage::Extract,
        Stage::Last,
    ];
    pub(crate) const ALL: [Stage; 8] = [
        Stage::Startup,
        Stage::First,
        Stage::PreUpdate,
        Stage::FixedUpdate,
        Stage::Update,
        Stage::PostUpdate,
        Stage::Extract,
        Stage::Last,
    ];
}
