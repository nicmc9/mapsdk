use winit::application::ApplicationHandler;
#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

use crate::{
    InputEvent, MouseButton, NextUpdate, PlatformConfig, PlatformError, PlatformHandler,
    PlatformWindow, TouchPhase, WindowSize,
};

/// Мост между `winit` и [`PlatformHandler`]: переводит события ОС в вызовы обработчика.
// [Rust] `<H: PlatformHandler>` — обобщённый тип (аналог шаблона). Код генерируется для
// конкретного H, вызовы методов статические, без виртуальной таблицы.
pub(crate) struct PlatformApp<H: PlatformHandler> {
    config: PlatformConfig,
    handler: H,
    window: Option<PlatformWindow>,
    suspended: bool,
    /// Кадры идут от событий перерисовки экрана, а не от таймера.
    display_driven: bool,
    error: Option<PlatformError>,
}

impl<H: PlatformHandler> PlatformApp<H> {
    pub(crate) fn new(config: PlatformConfig, handler: H) -> Self {
        Self {
            config,
            handler,
            window: None,
            suspended: true,
            display_driven: false,
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

    /// Выполняет кадр и настраивает, когда проснуться в следующий раз.
    fn run_update(&mut self, event_loop: &ActiveEventLoop) {
        let control_flow = match self.handler.update() {
            NextUpdate::Immediately => ControlFlow::Poll,
            NextUpdate::At(instant) => ControlFlow::WaitUntil(instant),
            NextUpdate::WaitForEvents => ControlFlow::Wait,
            NextUpdate::NextDisplayFrame => {
                self.display_driven = true;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                ControlFlow::Wait
            }
            NextUpdate::Exit => {
                event_loop.exit();
                return;
            }
        };
        event_loop.set_control_flow(control_flow);
    }

    /// Клавиша — по физическому коду, текст — отдельными символами: один Shift+A даёт
    /// символ «A» (или «Ф» в другой раскладке).
    fn keyboard_input(&mut self, event: &winit::event::KeyEvent) {
        if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key
            && !event.repeat
        {
            self.handler.input(InputEvent::Key {
                key: crate::input::map_key(code),
                pressed: event.state.is_pressed(),
            });
        }
        if event.state.is_pressed()
            && let Some(text) = &event.text
        {
            for c in text.chars().filter(|c| !c.is_control()) {
                self.handler.input(InputEvent::Char(c));
            }
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: PlatformError) {
        tracing::error!("{error}");
        self.error.get_or_insert(error);
        event_loop.exit();
    }
}

impl<H: PlatformHandler> ApplicationHandler for PlatformApp<H> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        tracing::info!("Жизненный цикл: resumed");
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
            if self.display_driven {
                // После фона цепочка перерисовок прервана — запускаем заново
                window.request_redraw();
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        tracing::info!("Жизненный цикл: suspended");
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
            WindowEvent::RedrawRequested if self.display_driven && !self.suspended => {
                self.run_update(event_loop);
            }
            WindowEvent::KeyboardInput { event, .. } => self.keyboard_input(&event),
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    winit::event::MouseButton::Back => MouseButton::Other(3),
                    winit::event::MouseButton::Forward => MouseButton::Other(4),
                    winit::event::MouseButton::Other(n) => MouseButton::Other(n),
                };
                self.handler.input(InputEvent::MouseButton {
                    button,
                    pressed: state.is_pressed(),
                });
            }
            #[allow(clippy::cast_possible_truncation)]
            WindowEvent::CursorMoved { position, .. } => {
                self.handler.input(InputEvent::CursorMoved {
                    x: position.x as f32,
                    y: position.y as f32,
                });
            }
            #[allow(clippy::cast_possible_truncation)]
            WindowEvent::MouseWheel { delta, .. } => {
                // Колесо мыши шлёт строки, тачпад — пиксели: это разные жесты
                let event = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => {
                        InputEvent::MouseWheel { lines: y }
                    }
                    winit::event::MouseScrollDelta::PixelDelta(p) => InputEvent::TrackpadScroll {
                        x: p.x as f32,
                        y: p.y as f32,
                    },
                };
                self.handler.input(event);
            }
            #[allow(clippy::cast_possible_truncation)]
            WindowEvent::PinchGesture { delta, .. } => {
                self.handler.input(InputEvent::TrackpadPinch {
                    delta: delta as f32,
                });
            }
            #[allow(clippy::cast_possible_truncation)]
            WindowEvent::Touch(touch) => {
                let phase = match touch.phase {
                    winit::event::TouchPhase::Started => TouchPhase::Started,
                    winit::event::TouchPhase::Moved => TouchPhase::Moved,
                    winit::event::TouchPhase::Ended => TouchPhase::Ended,
                    winit::event::TouchPhase::Cancelled => TouchPhase::Cancelled,
                };
                self.handler.input(InputEvent::Touch {
                    id: touch.id,
                    phase,
                    x: touch.location.x as f32,
                    y: touch.location.y as f32,
                });
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
        if self.display_driven {
            // Кадры идут от RedrawRequested, здесь ничего не делаем
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        self.run_update(event_loop);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        tracing::info!("Жизненный цикл: exiting");
        self.handler.exiting();
    }
}
