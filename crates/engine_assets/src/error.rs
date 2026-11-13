use std::sync::Arc;

/// Ошибка загрузки ресурса. Клонируется дёшево: одна ошибка может храниться в статусе,
/// уйти в событие и в лог.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AssetError {
    #[error("файл не найден: {0}")]
    NotFound(String),
    #[error("ошибка чтения {path}: {reason}")]
    Io { path: String, reason: String },
    #[error("сеть не подключена (нужен NetPlugin)")]
    NoNetwork,
    #[error("сеть: {0}")]
    Net(#[from] engine_net::NetError),
    #[error("нет загрузчика для «{0}»")]
    NoLoader(String),
    #[error("загрузчик {loader} даёт {produced}, а запрошен {requested}")]
    WrongType {
        loader: &'static str,
        produced: &'static str,
        requested: &'static str,
    },
    #[error("ошибка формата {path}: {reason}")]
    Format { path: String, reason: Arc<str> },
    #[error("зависимость {dependency}: {source}")]
    Dependency {
        dependency: String,
        source: Box<AssetError>,
    },
}

impl AssetError {
    pub fn format(path: &impl std::fmt::Display, reason: impl std::fmt::Display) -> Self {
        Self::Format {
            path: path.to_string(),
            reason: reason.to_string().into(),
        }
    }
}
