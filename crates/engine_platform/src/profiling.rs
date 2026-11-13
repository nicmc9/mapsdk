//! Профилирование. Зоны — обычные спаны `tracing`:
//!
//! ```ignore
//! let _zone = tracing::trace_span!("upload_textures").entered();
//! ```
//!
//! Без фичи `profiling` функции модуля пустые и компилятор их удаляет, а спаны уровня
//! `trace` отфильтрованы логами — накладных расходов почти нет.
//!
//! С фичей `profiling` на native данные идут в [Tracy](https://github.com/wolfpld/tracy):
//! запустите Tracy GUI и нажмите Connect. Клиент собирает данные, только пока GUI подключён.

/// Конец кадра главного цикла.
pub fn frame_mark() {
    #[cfg(all(feature = "profiling", not(target_arch = "wasm32")))]
    if let Some(client) = tracy_client::Client::running() {
        client.frame_mark();
    }
}

/// Конец кадра рендера — в Tracy отдельная шкала кадров.
pub fn render_frame_mark() {
    #[cfg(all(feature = "profiling", not(target_arch = "wasm32")))]
    if let Some(client) = tracy_client::Client::running() {
        client.secondary_frame_mark(tracy_client::frame_name!("render"));
    }
}

/// Имя текущего потока в профайлере.
pub fn set_thread_name(name: &str) {
    #[cfg(all(feature = "profiling", not(target_arch = "wasm32")))]
    if let Some(client) = tracy_client::Client::running() {
        client.set_thread_name(name);
    }
    let _ = name; // без профайлера параметр не используется
}

#[cfg(all(feature = "profiling", not(target_arch = "wasm32")))]
pub(crate) fn layer() -> tracing_tracy::TracyLayer {
    tracing_tracy::TracyLayer::default()
}

#[cfg(all(feature = "profiling", target_arch = "wasm32"))]
pub(crate) fn layer() -> tracing_subscriber::layer::Identity {
    // TODO(K-04): вкладка Performance браузера через tracing_web::performance_layer
    tracing_subscriber::layer::Identity::new()
}
