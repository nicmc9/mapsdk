/// Параметры окна при запуске.
///
/// В итерации 1 значения будут приходить из `CVar` (файл, командная строка, консоль).
// [Rust] `derive` генерирует реализации трейтов автоматически:
// Debug — печать через {:?}, Clone — явное копирование через .clone().
#[derive(Debug, Clone)]
pub struct PlatformConfig {
    pub title: String,
    /// Логический размер (в точках, без учёта DPI). Только desktop: на iOS, Android и Web
    /// окно занимает экран или canvas.
    pub width: u32,
    pub height: u32,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            title: "Engine".to_owned(),
            width: 1280,
            height: 720,
        }
    }
}
