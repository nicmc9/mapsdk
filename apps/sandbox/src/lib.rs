//! Тестовое приложение. Здесь же — точки входа для платформ, где их задаёт ОС.

use engine::platform::{LaunchContext, PlatformConfig, PlatformError};

/// Общий запуск для всех платформ.
pub fn run(launch: LaunchContext) -> Result<(), PlatformError> {
    let config = PlatformConfig {
        title: "Sandbox".to_owned(),
        ..PlatformConfig::default() // [Rust] остальные поля — из значения по умолчанию
    };
    engine::platform::run(config, launch)
}

/// Точка входа на Android. Её по имени ищет `android-activity` после загрузки .so,
/// функции `main` на Android нет.
#[cfg(target_os = "android")]
// no_mangle небезопасен: два символа с одним именем — UB при линковке.
// ABI именно Rust, а не C: android-activity объявляет функцию как `extern "Rust"`.
#[allow(unsafe_code, clippy::no_mangle_with_rust_abi)]
#[unsafe(no_mangle)]
fn android_main(app: engine::platform::AndroidApp) {
    if let Err(error) = run(LaunchContext::android(app)) {
        log::error!("Приложение завершилось с ошибкой: {error}");
    }
}

/// Точка входа на Web. `wasm-bindgen` вызывает её сразу после загрузки модуля.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn web_start() {
    if let Err(error) = run(LaunchContext::default()) {
        log::error!("Приложение завершилось с ошибкой: {error}");
    }
}
