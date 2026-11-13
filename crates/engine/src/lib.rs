//! Фасад движка. Проект зависит только от этого крейта.
//!
//! Обычно достаточно `use engine::prelude::*;`. Подсистемы доступны и по отдельности:
//! `engine::ecs`, `engine::math`, `engine::platform`.

pub use engine_core::{
    App, AppLifecycle, FixedTime, MainWindow, Plugin, Stage, Time, Window, WindowResized,
};
pub use engine_ecs as ecs;
pub use engine_math as math;
pub use engine_platform as platform;

pub mod prelude {
    //! Всё, что нужно в большинстве файлов проекта.

    pub use crate::{App, AppLifecycle, FixedTime, Plugin, Stage, Time, Window, WindowResized};
    pub use engine_ecs::prelude::*;
    pub use engine_math::*;
}
