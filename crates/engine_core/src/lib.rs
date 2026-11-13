//! Ядро движка: [`App`], плагины, стадии кадра, время с фиксированным шагом.

mod app;
mod plugin;
mod runner;
mod stage;
mod time;
mod window;

pub mod sync;

pub use app::App;
pub use plugin::Plugin;
pub use stage::Stage;
pub use time::{FixedTime, Time};
pub use window::{AppLifecycle, LifecycleHook, MainWindow, Window, WindowResized};
