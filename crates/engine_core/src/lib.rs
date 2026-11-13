//! Ядро движка: [`App`], плагины, стадии кадра, время с фиксированным шагом.

mod app;
mod plugin;
mod runner;
mod stage;
mod time;
mod window;

pub use app::App;
pub use plugin::Plugin;
pub use stage::Stage;
pub use time::{FixedTime, Time};
pub use window::{AppLifecycle, MainWindow, Window, WindowResized};
