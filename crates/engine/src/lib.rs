//! Фасад движка. Проект зависит только от этого крейта.
//!
//! Обычно достаточно `use engine::prelude::*;`. Подсистемы доступны и по отдельности:
//! `engine::ecs`, `engine::math`, `engine::platform`, `engine::render`.

pub use engine_assets as assets;
pub use engine_core::{
    AnyCVar, App, AppLifecycle, CHEATS, CVar, CVarError, CVarFlags, CVarSource, CVarString,
    CVarValue, CVars, FixedTime, LOG_FILTER, MainWindow, Plugin, SIM_HZ, SIM_MAX_STEPS, Stage,
    Time, Window, WindowResized,
};
pub use engine_ecs as ecs;
pub use engine_math as math;
pub use engine_net as net;
pub use engine_platform as platform;
pub use engine_render as render;

/// Логи и зоны профайлера: `engine::log::info!("...")`, `engine::log::trace_span!("zone")`.
///
/// [Rust] Макросы реэкспортируются как обычные элементы. Внутри они ссылаются на свой крейт
/// через `$crate`, поэтому работают, даже если проект не зависит от `tracing` напрямую.
pub mod log {
    pub use tracing::{debug, debug_span, error, info, info_span, trace, trace_span, warn};
}

pub mod prelude {
    //! Всё, что нужно в большинстве файлов проекта.

    pub use crate::{
        App, AppLifecycle, CVar, CVarFlags, CVars, FixedTime, Plugin, Stage, Time, Window,
        WindowResized,
    };
    pub use engine_assets::{
        AssetAppExt, AssetEvent, AssetPlugin, AssetServer, AssetStatus, Assets, Handle,
    };
    pub use engine_ecs::prelude::*;
    pub use engine_math::*;
    pub use engine_net::{NetPlugin, NetStats};
    pub use engine_render::{ClearColor, Color, RenderPlugin, RenderStats};
}
