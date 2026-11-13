use engine_platform::{Instant, NextUpdate, PlatformHandler, PlatformWindow, WindowSize};

use crate::window::MainWindow;
use crate::{App, AppLifecycle, Time, Window, WindowResized};

/// Связывает цикл событий платформы с [`App`]: события ОС превращаются в ресурсы
/// и сообщения ECS, а кадры выполняются с темпом фиксированного шага.
#[derive(Debug)]
pub(crate) struct AppRunner {
    app: App,
    /// Когда выполнять следующий кадр. `None` — сразу.
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
    }

    fn suspended(&mut self) {
        self.app.world_mut().write_message(AppLifecycle::Suspended);
    }

    fn resized(&mut self, size: WindowSize, scale_factor: f64) {
        self.set_window_state(size, scale_factor);
    }

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
        let next = self.app.next_update_time(now);
        self.next_frame = Some(next);
        NextUpdate::At(next)
    }

    fn exiting(&mut self) {
        log::info!("Остановка приложения");
    }
}
