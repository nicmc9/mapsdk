//! Временное логирование для итерации 0. В итерации 1 заменяется на `tracing` (задача K-03).

use std::sync::Once;

/// Подключает вывод логов платформы. Безопасно вызывать повторно.
pub fn init_logging() {
    // [Rust] `Once` — потокобезопасный «выполнить ровно один раз», аналог std::call_once.
    static INIT: Once = Once::new();
    INIT.call_once(init_platform_logger);
}

#[cfg(target_os = "android")]
fn init_platform_logger() {
    // Вывод в logcat: `adb logcat -s engine`
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("engine"),
    );
}

#[cfg(target_arch = "wasm32")]
fn init_platform_logger() {
    // Паники выводятся в консоль браузера, иначе там будет только "unreachable executed".
    console_error_panic_hook::set_once();
    // [Rust] `let _ =` — явный отказ от результата. Логгер может быть уже установлен — это не ошибка.
    let _ = console_log::init_with_level(log::Level::Info);
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn init_platform_logger() {
    // Уровень задаётся переменной окружения: RUST_LOG=debug cargo run
    let env = env_logger::Env::default().default_filter_or("info");
    let _ = env_logger::Builder::from_env(env).try_init();
}
