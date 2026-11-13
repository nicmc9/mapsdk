use engine_platform::{Instant, NextUpdate, PlatformHandler, PlatformWindow, WindowSize};

use crate::window::MainWindow;
use crate::{App, AppLifecycle, LifecycleHook, Time, Window, WindowResized};

/// Связывает цикл событий платформы с [`App`]: события ОС превращаются в ресурсы
/// и сообщения ECS, а кадры выполняются с темпом фиксированного шага.
#[derive(Debug)]
pub(crate) struct AppRunner {
    app: App,
    /// Когда выполнять следующий кадр. `None` — сразу. На Web не используется.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    next_frame: Option<Instant>,
}

impl AppRunner {
    pub(crate) fn new(app: App) -> Self {
        Self {
            app,
            next_frame: None,
        }
    }

    fn set_window_state(&mut self, size: WindowSize, scale_factor: f64) {
        let world = self.app.world_mut();
        let mut window = world.resource_mut::<Window>();
        window.size = size;
        window.scale_factor = scale_factor;
        world.write_message(WindowResized { size, scale_factor });
    }
}

impl PlatformHandler for AppRunner {
    fn resumed(&mut self, window: &PlatformWindow) {
        self.next_frame = None; // после фона — сразу кадр
        self.app.world_mut().resource_mut::<Time>().skip_gap();
        self.app.set_main_window(MainWindow(window.clone()));
        self.set_window_state(window.size(), window.scale_factor());
        self.app.world_mut().write_message(AppLifecycle::Resumed);
        self.app
            .run_lifecycle_hooks(&LifecycleHook::Resumed(window));
    }

    fn suspended(&mut self) {
        self.app.world_mut().write_message(AppLifecycle::Suspended);
        self.app.run_lifecycle_hooks(&LifecycleHook::Suspended);
    }

    fn resized(&mut self, size: WindowSize, scale_factor: f64) {
        self.set_window_state(size, scale_factor);
        self.app
            .run_lifecycle_hooks(&LifecycleHook::Resized { size, scale_factor });
    }

    /// На Web кадр выполняется на каждое обновление экрана: рендер там работает в главном
    /// потоке, и так он рисует с частотой экрана, а логика копит шаги как обычно.
    #[cfg(target_arch = "wasm32")]
    fn update(&mut self) -> NextUpdate {
        self.app.update_at(Instant::now());
        engine_platform::profiling::frame_mark();
        NextUpdate::NextDisplayFrame
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn update(&mut self) -> NextUpdate {
        // Цикл ОС просыпается не только по нашему таймеру, но и на каждое событие
        // (движение мыши, перерисовка). Кадр выполняем только когда подошло время,
        // иначе частота кадров зависела бы от активности пользователя.
        let now = Instant::now();
        if let Some(next) = self.next_frame
            && now < next
        {
            return NextUpdate::At(next);
        }
        self.app.update_at(now);
        engine_platform::profiling::frame_mark();
        let next = self.app.next_update_time(now);
        self.next_frame = Some(next);
        NextUpdate::At(next)
    }

    fn exiting(&mut self) {
        tracing::info!("Остановка приложения");
        self.app.run_lifecycle_hooks(&LifecycleHook::Exiting);
    }
}
