//! Логи на `tracing` с выводом, родным для каждой платформы.
//!
//! | Платформа | Куда | Как смотреть |
//! |-----------|------|--------------|
//! | macOS, Linux, iOS-симулятор | stderr | терминал |
//! | Android | logcat | `adb logcat -s engine` |
//! | iOS-устройство | `os_log` | `Console.app`, Xcode |
//! | Web | консоль браузера | инструменты разработчика (F12) |
//!
//! Логи сторонних библиотек, которые пишут через крейт `log` (`wgpu`, `winit`), автоматически
//! перенаправляются в `tracing`.

use std::sync::OnceLock;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, reload};

/// Фильтр по умолчанию. Шумные библиотеки — только предупреждения.
/// Синтаксис: `уровень,модуль=уровень,...` (как переменная `RUST_LOG`).
pub const DEFAULT_LOG_FILTER: &str =
    "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,bevy_ecs=warn,winit=warn";

// Ручка для замены фильтра на лету (из CVar `log.filter` или консоли).
// [Rust] `OnceLock` — глобальная переменная, которая инициализируется один раз и потом
// только читается. Потокобезопасна, аналог `static` + `std::call_once`.
static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// Включает логи. Безопасно вызывать повторно. Фильтр берётся из `RUST_LOG`,
/// а если её нет — [`DEFAULT_LOG_FILTER`].
pub fn init_logging() {
    FILTER.get_or_init(install);
}

/// Меняет фильтр логов на лету. Пример: `"info,engine_render=debug"`.
pub fn set_log_filter(filter: &str) -> Result<(), String> {
    let filter = EnvFilter::try_new(filter).map_err(|e| e.to_string())?;
    let handle = FILTER.get().ok_or("логи ещё не инициализированы")?;
    handle.reload(filter).map_err(|e| e.to_string())
}

fn install() -> reload::Handle<EnvFilter, Registry> {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    let (filter, handle) = reload::Layer::new(filter);

    // [Rust] Подписчик собирается из слоёв (layer), как конвейер: каждый слой получает
    // события и решает, что с ними делать. Фильтр здесь стоит только на выводе логов —
    // профайлер (если включён) видит все зоны, независимо от уровня логов.
    let subscriber = tracing_subscriber::registry().with(output_layer().with_filter(filter));
    #[cfg(feature = "profiling")]
    let subscriber = subscriber.with(crate::profiling::layer());

    // try_init заодно подключает мост `log` → `tracing`
    if let Err(error) = subscriber.try_init() {
        eprintln!("Логи уже настроены другим подписчиком: {error}");
    }
    install_panic_hook();
    handle
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32", target_os = "ios")))]
fn output_layer() -> impl Layer<Registry> + Send + Sync {
    tracing_subscriber::fmt::layer().with_writer(std::io::stderr)
}

#[cfg(all(target_os = "ios", target_abi = "sim"))]
fn output_layer() -> impl Layer<Registry> + Send + Sync {
    tracing_subscriber::fmt::layer().with_writer(std::io::stderr)
}

#[cfg(all(target_os = "ios", not(target_abi = "sim")))]
fn output_layer() -> impl Layer<Registry> + Send + Sync {
    tracing_oslog::OsLogger::new("com.mapsdk.engine", "default")
}

#[cfg(target_os = "android")]
fn output_layer() -> impl Layer<Registry> + Send + Sync {
    use tracing_logcat::{LogcatMakeWriter, LogcatTag};
    let writer = LogcatMakeWriter::new(LogcatTag::Fixed("engine".to_owned()))
        .expect("не удалось открыть logcat");
    // Время, уровень и цвет не нужны: logcat добавляет время и уровень сам, цвета не понимает
    tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_level(false)
        .without_time()
}

#[cfg(target_arch = "wasm32")]
fn output_layer() -> impl Layer<Registry> + Send + Sync {
    // Системного времени в браузере нет (без `without_time` — паника)
    tracing_subscriber::fmt::layer()
        .with_writer(tracing_web::MakeWebConsoleWriter::new())
        .with_ansi(false)
        .without_time()
}

/// Паника (в любом потоке) — в наш лог с местом и именем потока.
///
/// [Rust] Стандартный обработчик печатает панику в stderr. На Android и iOS-устройстве
/// stderr никуда не попадает — поток рендера упал бы молча. Свой обработчик пишет в лог
/// платформы, затем вызывает стандартный (он печатает backtrace при `RUST_BACKTRACE=1`).
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("без имени");
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(ToString::to_string)
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "?".to_owned());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        tracing::error!("ПАНИКА в потоке «{name}»: {message} ({location})");
        default(info);
    }));
}
