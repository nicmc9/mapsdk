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

/// Синхронное событие платформы для плагинов, которым нельзя ждать следующего кадра.
///
/// В отличие от сообщения [`AppLifecycle`], хук вызывается сразу, внутри обработчика ОС.
/// Пример: на Android после `Suspended` окно уничтожается немедленно, и рендер должен
/// отпустить surface до возврата из хука.
#[derive(Debug, Clone, Copy)]
pub enum LifecycleHook<'a> {
    Resumed(&'a engine_platform::PlatformWindow),
    Suspended,
    Resized { size: WindowSize, scale_factor: f64 },
    Exiting,
}

/// Событие ввода от платформы. Читается через `MessageReader<PlatformInput>`;
/// удобное состояние (нажатые клавиши и т.д.) собирает `engine_input`.
///
/// [Rust] Обёртка нужна из-за правила сироты (orphan rule): трейт `Message` (из `bevy_ecs`)
/// нельзя реализовать для `InputEvent` (из `engine_platform`) в третьем крейте —
/// хотя бы трейт или тип должен быть «своим». Своя обёртка решает это.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlatformInput(pub engine_platform::InputEvent);

/// Управление приложением из систем.
#[derive(Resource, Debug, Default)]
pub struct AppControl {
    exit_requested: bool,
}

impl AppControl {
    /// Завершить приложение после текущего кадра.
    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }

    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }
}
