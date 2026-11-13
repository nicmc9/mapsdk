use winit::error::{EventLoopError, OsError};

/// Ошибки платформенного слоя.
// [Rust] `thiserror` генерирует реализацию std::error::Error и текст сообщения.
// `#[from]` разрешает автоматическую конвертацию исходной ошибки через `?`.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("ошибка цикла событий: {0}")]
    EventLoop(#[from] EventLoopError),

    #[error("не удалось создать окно: {0}")]
    CreateWindow(#[from] OsError),
}
