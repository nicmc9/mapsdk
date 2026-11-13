/// Ошибка загрузки. `Clone` — один результат получают все ожидающие (дедупликация).
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum NetError {
    #[error("превышено время ожидания")]
    Timeout,
    #[error("сервер ответил {0}")]
    Status(u16),
    #[error("ответ больше {0} байт")]
    TooLarge(u64),
    #[error("сетевая ошибка: {0}")]
    Transport(String),
}
