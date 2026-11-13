//! Ядро движка: [`App`], плагины, стадии кадра, время с фиксированным шагом.

mod app;
mod cvar;
mod plugin;
mod runner;
mod settings;
mod stage;
mod time;
mod window;

pub mod sync;
pub mod tasks;

pub use app::App;
pub use cvar::{
    AnyCVar, CHEATS, CVar, CVarError, CVarFlags, CVarSource, CVarString, CVarValue, CVars,
};
pub use plugin::Plugin;
pub use settings::{LOG_FILTER, SIM_HZ, SIM_MAX_STEPS};
pub use stage::Stage;
pub use time::{FixedTime, Time};
pub use window::{AppLifecycle, LifecycleHook, MainWindow, Window, WindowResized};
