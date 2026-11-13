use winit::application::ApplicationHandler;
#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

use crate::{
    NextUpdate, PlatformConfig, PlatformError, PlatformHandler, PlatformWindow, WindowSize,
};

/// Мост между `winit` и [`PlatformHandler`]: переводит события ОС в вызовы обработчика.
// [Rust] `<H: PlatformHandler>` — обобщённый тип (аналог шаблона). Код генерируется для
// конкретного H, вызовы методов статические, без виртуальной таблицы.
pub(crate) struct PlatformApp<H: PlatformHandler> {
    config: PlatformConfig,
    handler: H,
    window: Option<PlatformWindow>,
    suspended: bool,
    error: Option<PlatformError>,
}

impl<H: PlatformHandler> PlatformApp<H> {
    pub(crate) fn new(config: PlatformConfig, handler: H) -> Self {
        Self {
            config,
            handler,
            window: None,
            suspended: true,
            error: None,
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn into_result(self) -> Result<(), PlatformError> {
        self.error.map_or(Ok(()), Err)
    }

    fn create_window(&self, event_loop: &ActiveEventLoop) -> Result<PlatformWindow, PlatformError> {
        let attributes = Window::default_attributes().with_title(&self.config.title);

        // Размер окна задаём только на desktop. На iOS и Android окно всегда во весь экран,
        // а если задать размер, iOS сделает окно больше экрана.
        // [Rust] `let` с тем же именем «перекрывает» прежнюю переменную (shadowing) —
        // это новая переменная, а не присваивание. Удобно для пошаговой настройки.
        #[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
        let attributes =
            attributes.with_inner_size(LogicalSize::new(self.config.width, self.config.height));

        // В браузере winit создаёт <canvas> и добавляет его в <body>.
        // Размер не задаём: его определяет CSS страницы.
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes.with_append(true)
        };

        Ok(PlatformWindow::new(event_loop.create_window(attributes)?))
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: PlatformError) {
        log::error!("{error}");
        self.error.get_or_insert(error);
        event_loop.exit();
    }
}

impl<H: PlatformHandler> ApplicationHandler for PlatformApp<H> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("Жизненный цикл: resumed");
        if self.window.is_none() {
            match self.create_window(event_loop) {
                Ok(window) => self.window = Some(window),
                Err(error) => return self.fail(event_loop, error),
            }
        }
        self.suspended = false;
        // [Rust] `if let Some(x) = opt` — выполнить блок, только если значение есть.
        if let Some(window) = &self.window {
            self.handler.resumed(window);
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("Жизненный цикл: suspended");
        self.suspended = true;
        self.handler.suspended();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if self.handler.close_requested() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                let scale = self
                    .window
                    .as_ref()
                    .map_or(1.0, PlatformWindow::scale_factor);
                self.handler.resized(
                    WindowSize {
                        width: size.width,
                        height: size.height,
                    },
                    scale,
                );
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(window) = &self.window {
                    self.handler.resized(window.size(), scale_factor);
                }
            }
            _ => {}
        }
    }

    /// Очередь событий ОС разобрана — выполняем кадр и решаем, когда проснуться снова.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.suspended {
            // В фоне кадры не считаем: ждём resumed.
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let control_flow = match self.handler.update() {
            NextUpdate::Immediately => ControlFlow::Poll,
            NextUpdate::At(instant) => ControlFlow::WaitUntil(instant),
            NextUpdate::WaitForEvents => ControlFlow::Wait,
            NextUpdate::Exit => {
                event_loop.exit();
                return;
            }
        };
        event_loop.set_control_flow(control_flow);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("Жизненный цикл: exiting");
        self.handler.exiting();
    }
}
