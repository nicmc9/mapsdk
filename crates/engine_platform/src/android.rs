//! Глобальный доступ к Activity: она нужна для путей и assets, а приходит только в `run`.

use std::sync::OnceLock;

use crate::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();

pub(crate) fn set_app(app: &AndroidApp) {
    // [Rust] AndroidApp дёшево клонируется (внутри — счётчик ссылок)
    let _ = APP.set(app.clone());
}

pub(crate) fn app() -> Option<&'static AndroidApp> {
    APP.get()
}
