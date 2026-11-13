//! Рендер движка на `wgpu`.
//!
//! Схема (подробно — `docs/acrhitecture.md`, разделы 3–5):
//! - главный поток в стадии `Extract` записывает [`FrameSnapshot`] в тройной буфер;
//! - рендер (в своём потоке или в главном — [`RenderMode`]) забирает свежий снимок и рисует;
//! - создание/потеря окна приходят в рендер командами через почтовый ящик.
//!
//! Типы `wgpu` наружу не выходят.

mod color;
mod command;
mod frontend;
mod gpu;
mod renderer;
mod snapshot;

pub use color::Color;
pub use frontend::{ClearColor, RenderMode, RenderPlugin, RenderStats};
