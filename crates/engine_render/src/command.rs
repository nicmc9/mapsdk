use engine_core::sync::reply::ReplySender;
use engine_platform::WindowSize;

/// Команды рендеру. Отправляются из главного потока, разбираются в начале кадра рендера.
// [Rust] Варианты enum несут данные (как std::variant с именованными полями).
// `match` по команде обязан обработать все варианты — забыть новый нельзя.
pub(crate) enum RenderCommand {
    /// Окно доступно: surface создан в главном потоке (на macOS/iOS иначе нельзя).
    /// `instance` приходит с первым surface — из него рендер создаёт адаптер и устройство.
    SurfaceCreated {
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        size: WindowSize,
    },
    /// Окно скоро будет уничтожено (уход в фон). Рендер обязан отпустить surface
    /// и ответить — до ответа главный поток не вернёт управление ОС.
    SurfaceLost {
        reply: ReplySender<()>,
    },
    Resize(WindowSize),
    /// Завершить работу рендера.
    Shutdown,
}

// Ручной Debug: wgpu-объекты печатать бессмысленно
impl std::fmt::Debug for RenderCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SurfaceCreated { size, .. } => write!(f, "SurfaceCreated({size:?})"),
            Self::SurfaceLost { .. } => write!(f, "SurfaceLost"),
            Self::Resize(size) => write!(f, "Resize({size:?})"),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}
