//! Параметры запуска в виде пар «имя = значение».
//!
//! - desktop: `sandbox --cvar sim.hz=30 --cvar render.mode=inline`
//! - Web: `http://localhost:8000/?sim.hz=30&render.mode=inline`
//! - Android, iOS: пока нет (будут из intent / launch arguments).

/// Возвращает пары `(имя, значение)` в порядке появления.
pub fn launch_arguments() -> Vec<(String, String)> {
    platform_arguments()
        .into_iter()
        .filter_map(|pair| {
            // [Rust] `split_once` делит строку по первому вхождению и возвращает Option
            let (name, value) = pair.split_once('=')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_arguments() -> Vec<String> {
    // Берём только значения после `--cvar`/`-c`, остальное не наше (например, аргументы
    // тестового раннера cargo test)
    let mut result = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--cvar" || arg == "-c" {
            if let Some(value) = args.next() {
                result.push(value);
            }
        } else if let Some(value) = arg.strip_prefix("--cvar=") {
            result.push(value.to_owned());
        }
    }
    result
}

#[cfg(target_arch = "wasm32")]
fn platform_arguments() -> Vec<String> {
    // location.search = "?sim.hz=30&render.mode=inline"
    let search = js_sys::eval("window.location.search")
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let pair = pair.replace('+', " ");
            js_sys::decode_uri_component(&pair)
                .ok()
                .and_then(|decoded| decoded.as_string())
                .unwrap_or(pair)
        })
        .collect()
}
