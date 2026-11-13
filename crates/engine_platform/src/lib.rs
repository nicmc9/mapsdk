//! Платформенный слой: окно, цикл событий ОС и жизненный цикл приложения.
//!
//! Это нижний слой движка: он ничего не знает о рендере, ресурсах и ECS.
//! Вся работа с `winit` спрятана здесь — его типы не выходят наружу.

// [Rust] `mod` подключает файл как модуль (аналог #include + namespace сразу).
// Без `pub` модуль виден только внутри крейта.
#[cfg(target_os = "android")]
mod android;
mod app;
mod args;
mod config;
mod error;
mod handler;
mod launch;
mod logging;
mod window;

pub mod paths;
pub mod profiling;

// [Rust] `pub use` — реэкспорт: пользователь пишет `engine_platform::PlatformConfig`,
// не зная, в каком файле тип объявлен. Аналог `using` в заголовке namespace.
pub use args::launch_arguments;
pub use config::PlatformConfig;
pub use error::PlatformError;
pub use handler::{NextUpdate, PlatformHandler};
pub use launch::LaunchContext;
pub use logging::{DEFAULT_LOG_FILTER, init_logging, set_log_filter};
pub use window::{PlatformWindow, WindowSize};

/// Время, работающее на всех платформах.
/// [Rust] `std::time::Instant` в браузере паникует: в wasm32-unknown-unknown нет системных
/// часов. `web_time` на native — это просто `std::time`, а на Web использует `performance.now()`.
pub use web_time::{Duration, Instant};

#[cfg(target_os = "android")]
pub use winit::platform::android::activity::AndroidApp;

use winit::event_loop::EventLoop;

/// Создаёт окно и запускает цикл событий ОС. События передаются в `handler`.
///
/// Поведение возврата зависит от платформы:
/// - macOS, Linux, Android — возвращает управление после закрытия окна;
/// - iOS — не возвращает никогда (так устроен `UIApplicationMain`);
/// - Web — возвращает сразу, цикл продолжает работать в браузере.
pub fn run<H: PlatformHandler>(
    config: PlatformConfig,
    launch: LaunchContext,
    handler: H,
) -> Result<(), PlatformError> {
    init_logging();
    tracing::info!("Запуск: {config:?}");

    let event_loop = build_event_loop(launch)?;
    let app = app::PlatformApp::new(config, handler);
    run_event_loop(event_loop, app)
}

#[cfg(target_os = "android")]
fn build_event_loop(launch: LaunchContext) -> Result<EventLoop<()>, PlatformError> {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    // На Android цикл событий привязан к Activity, которую передаёт система.
    let app = launch.into_android_app();
    android::set_app(&app);
    Ok(EventLoop::builder().with_android_app(app).build()?)
}

#[cfg(not(target_os = "android"))]
fn build_event_loop(_launch: LaunchContext) -> Result<EventLoop<()>, PlatformError> {
    // [Rust] `?` — если слева ошибка, вернуть её из функции (с конвертацией типа через From).
    // Замена try/catch + throw: ошибка видна в сигнатуре, проигнорировать её нельзя.
    Ok(EventLoop::new()?)
}

#[cfg(target_arch = "wasm32")]
// Сигнатура общая для всех платформ, поэтому Result оставляем, хотя здесь ошибок нет.
#[allow(clippy::unnecessary_wraps)]
fn run_event_loop<H: PlatformHandler>(
    event_loop: EventLoop<()>,
    app: app::PlatformApp<H>,
) -> Result<(), PlatformError> {
    use winit::platform::web::EventLoopExtWebSys;
    // В браузере нельзя блокировать поток: цикл отдаётся браузеру, функция сразу возвращается.
    event_loop.spawn_app(app);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_event_loop<H: PlatformHandler>(
    event_loop: EventLoop<()>,
    mut app: app::PlatformApp<H>,
) -> Result<(), PlatformError> {
    event_loop.run_app(&mut app)?;
    // Ошибки внутри обработчиков событий нельзя вернуть через `?` (их вызывает winit),
    // поэтому обработчик сохраняет первую ошибку и завершает цикл. Отдаём её здесь.
    app.into_result()
}
