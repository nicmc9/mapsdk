//! Точка входа для macOS, Linux и iOS.
//! На Android и Web исполняемый файл не используется — там вход в lib.rs.

use std::process::ExitCode;

#[cfg(not(target_os = "android"))]
fn main() -> ExitCode {
    match sandbox::run(engine::platform::LaunchContext::default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            engine::log::error!("Приложение завершилось с ошибкой: {error}");
            ExitCode::FAILURE
        }
    }
}

// Заглушка, чтобы `cargo clippy --all-targets` проходил под Android.
#[cfg(target_os = "android")]
fn main() -> ExitCode {
    ExitCode::SUCCESS
}
