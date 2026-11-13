//! Рендер движка на `wgpu`.
//!
//! Схема (подробно — `docs/acrhitecture.md`, разделы 3–5, 11):
//! - главный поток в стадии `Extract` записывает снимок кадра в тройной буфер, а загруженные
//!   ресурсы (меши, текстуры, материалы) отправляет командами;
//! - рендер (в своём потоке или в главном — [`RenderMode`]) забирает свежий снимок,
//!   интерполирует трансформы, отсекает невидимое и рисует PBR;
//! - цель — окно или текстура в памяти (`render.headless`), кадр можно сохранить в PNG.
//!
//! Типы `wgpu` наружу не выходят.

mod color;
mod command;
mod debug_draw;
mod extract;
mod frontend;
mod gpu;
mod graph;
mod renderer;
mod scene;
mod screenshot;
mod snapshot;
mod ui;

pub use color::Color;
pub use command::Screenshot;
pub use debug_draw::{DEBUG_AXES, DEBUG_BOUNDS, DEBUG_GRID, DebugDraw};
/// egui для своих окон: `engine::render::egui::Window::new(..)`.
pub use egui;
pub use frontend::{
    ClearColor, RENDER_BACKEND, RENDER_HEADLESS, RENDER_MODE, RenderMode, RenderPlugin,
    RenderStats, SCREENSHOT_EXIT, SCREENSHOT_FRAME, SCREENSHOT_PATH, ScreenshotReceiver,
    request_screenshot,
};
pub use ui::{DEBUG_UI, Ui};

/// Сохраняет снимок в PNG (только native — на Web файловой системы нет).
pub fn save_png(shot: &Screenshot, path: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (shot, path);
        Err("на Web сохранение файлов не поддерживается".to_owned())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        image::save_buffer(
            path,
            &shot.data,
            shot.width,
            shot.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())
    }
}
