//! Загрузка на Web через `fetch` браузера. Кешем управляет браузер (HTTP-кеш).

use std::sync::atomic::Ordering;

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::{Bytes, Counters, NetError, Settings, is_retryable_status};

pub(crate) async fn fetch(
    url: &str,
    settings: &Settings,
    counters: &Counters,
) -> Result<Bytes, NetError> {
    let mut last_error = NetError::Transport("нет попыток".to_owned());
    for attempt in 0..=settings.retries {
        if attempt > 0 {
            counters.retries.fetch_add(1, Ordering::Relaxed);
            sleep(Settings::backoff(attempt)).await;
        }
        match request(url, settings).await {
            Ok(bytes) => {
                counters
                    .bytes_downloaded
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                return Ok(bytes);
            }
            Err(error) => {
                let retry = match &error {
                    NetError::Status(code) => is_retryable_status(*code),
                    NetError::Timeout | NetError::Transport(_) => true,
                    NetError::TooLarge(_) => false,
                };
                last_error = error;
                if !retry {
                    break;
                }
            }
        }
    }
    Err(last_error)
}

async fn request(url: &str, settings: &Settings) -> Result<Bytes, NetError> {
    let window = web_sys::window().ok_or_else(|| NetError::Transport("нет window".to_owned()))?;
    let init = web_sys::RequestInit::new();
    // AbortSignal.timeout отменит запрос по таймауту (поддерживается современными браузерами)
    let timeout_ms = u32::try_from(settings.timeout.as_millis()).unwrap_or(u32::MAX);
    init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(timeout_ms)));

    let response = JsFuture::from(window.fetch_with_str_and_init(url, &init))
        .await
        .map_err(js_error)?;
    let response: web_sys::Response = response
        .dyn_into()
        .map_err(|_| NetError::Transport("неожиданный ответ fetch".to_owned()))?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(NetError::Status(status));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    if bytes.len() as u64 > settings.max_response_bytes {
        return Err(NetError::TooLarge(settings.max_response_bytes));
    }
    Ok(bytes.into())
}

fn js_error(value: wasm_bindgen::JsValue) -> NetError {
    // Отмена по AbortSignal.timeout приходит как DOMException с именем TimeoutError
    if let Some(exception) = value.dyn_ref::<js_sys::Error>() {
        let name: String = exception.name().into();
        if name == "TimeoutError" {
            return NetError::Timeout;
        }
        return NetError::Transport(String::from(exception.message()));
    }
    NetError::Transport(format!("{value:?}"))
}

/// Асинхронная пауза через setTimeout (блокировать поток в браузере нельзя).
async fn sleep(duration: std::time::Duration) {
    let ms = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}
