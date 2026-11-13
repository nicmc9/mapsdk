use winit::application::ApplicationHandler;
#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::{PlatformConfig, PlatformError};

/// Обработчик событий ОС.
///
/// `winit` вызывает его методы из цикла событий — это аналог виртуальных
/// методов-колбэков, только через трейт `ApplicationHandler`.
#[derive(Debug)]
pub(crate) struct PlatformApp {
    config: PlatformConfig,
    // [Rust] `Option<T>` — значение может отсутствовать (аналог std::optional).
    // Окно создаётся не сразу, а в `resumed`, поэтому до этого здесь `None`.
    window: Option<Window>,
    error: Option<PlatformError>,
}

impl PlatformApp {
    pub(crate) fn new(config: PlatformConfig) -> Self {
        Self {
            config,
            window: None,
            error: None,
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn into_result(self) -> Result<(), PlatformError> {
        // [Rust] `map_or` — если ошибки нет, Ok(()), иначе Err(ошибка).
        self.error.map_or(Ok(()), Err)
    }

    fn create_window(&self, event_loop: &ActiveEventLoop) -> Result<Window, PlatformError> {
        let attributes = Window::default_attributes().with_title(&self.config.title);

        // [Rust] `let` с тем же именем «перекрывает» прежнюю переменную (shadowing) —
        // это новая переменная, а не присваивание. Удобно для пошаговой настройки.
        // Размер окна задаём только на desktop. На iOS и Android окно всегда во весь экран,
        // а если задать размер, iOS сделает окно больше экрана.
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

        Ok(event_loop.create_window(attributes)?)
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: PlatformError) {
        log::error!("{error}");
        self.error.get_or_insert(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for PlatformApp {
    /// Приложение стало активным. На Android и iOS вызывается при каждом
    /// возврате из фона; только после этого можно работать с окном.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("Жизненный цикл: resumed");
        if self.window.is_some() {
            // Окно уже есть. Пересоздание GPU-surface после фона — задача TH-10.
            return;
        }
        match self.create_window(event_loop) {
            Ok(window) => {
                log::info!(
                    "Окно создано: {:?}, scale factor {}",
                    window.inner_size(),
                    window.scale_factor()
                );
                self.window = Some(window);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    /// Приложение ушло в фон. На Android после этого native-окно уничтожается,
    /// и рисовать в него нельзя до следующего `resumed`.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("Жизненный цикл: suspended");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // [Rust] `match` обязан покрыть все варианты; `_ => {}` — «остальное игнорируем».
        match event {
            WindowEvent::CloseRequested => {
                log::info!("Закрытие окна");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => log::info!("Размер окна: {size:?}"),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                log::info!("Scale factor: {scale_factor}");
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("Жизненный цикл: exiting");
    }
}
