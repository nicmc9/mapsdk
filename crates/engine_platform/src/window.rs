use std::sync::Arc;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::Window;

/// Физический размер окна в пикселях.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

/// Окно приложения. Дёшево копируется: все копии указывают на одно окно.
///
/// Реализует `HasWindowHandle`/`HasDisplayHandle` — по ним рендер создаёт GPU-surface,
/// не зная, что внутри `winit`.
// [Rust] `Arc` — атомарный счётчик ссылок, аналог `std::shared_ptr`. `clone()` только
// увеличивает счётчик. Arc нужен, потому что окно будет использовать и поток рендера.
#[derive(Debug, Clone)]
pub struct PlatformWindow {
    inner: Arc<Window>,
}

impl PlatformWindow {
    pub(crate) fn new(window: Window) -> Self {
        Self {
            inner: Arc::new(window),
        }
    }

    pub fn size(&self) -> WindowSize {
        let size = self.inner.inner_size();
        WindowSize {
            width: size.width,
            height: size.height,
        }
    }

    pub fn scale_factor(&self) -> f64 {
        self.inner.scale_factor()
    }

    /// Попросить ОС прислать событие перерисовки (на Web — в следующем requestAnimationFrame).
    pub fn request_redraw(&self) {
        self.inner.request_redraw();
    }
}

// [Rust] Реализация чужого трейта для своего типа — так тип «подключается» к внешнему API.
// Аналог наследования от интерфейса, но может быть добавлена отдельно от объявления типа.
impl HasWindowHandle for PlatformWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.inner.window_handle()
    }
}

impl HasDisplayHandle for PlatformWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.inner.display_handle()
    }
}
