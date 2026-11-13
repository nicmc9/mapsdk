use engine_ecs::prelude::*;
use engine_platform::WindowSize;

/// Состояние окна. Доступно системам как `Res<Window>`.
#[derive(Resource, Debug, Default)]
pub struct Window {
    pub size: WindowSize,
    pub scale_factor: f64,
}

/// Окно изменило размер (физические пиксели).
#[derive(Message, Debug, Clone, Copy)]
pub struct WindowResized {
    pub size: WindowSize,
    pub scale_factor: f64,
}

/// Смена состояния приложения.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppLifecycle {
    /// Приложение активно (в том числе после возврата из фона).
    Resumed,
    /// Приложение ушло в фон.
    Suspended,
}

/// Главное окно — нужно рендеру для создания GPU-surface.
/// Появляется в мире после первого `resumed`.
#[derive(Resource, Debug, Clone)]
pub struct MainWindow(pub engine_platform::PlatformWindow);
