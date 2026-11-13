use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use engine_core::sync::mailbox::{MailboxSender, mailbox};
use engine_core::sync::reply::{ReplyReceiver, reply};
use engine_core::sync::triple_buffer::{Producer, triple_buffer};
use engine_core::{
    App, AppControl, CVar, CVarFlags, CVarString, CVarValue, LifecycleHook, Plugin, Stage, Time,
};
use engine_ecs::prelude::*;
use engine_platform::{Duration, Instant, PlatformWindow, WindowSize};

use crate::Color;
use crate::command::{RenderCommand, Screenshot};
use crate::renderer::{Renderer, SharedStats};
use crate::snapshot::FrameSnapshot;

/// Где выполняется рендер.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// В отдельном потоке. По умолчанию на native.
    Threaded,
    /// В главном потоке после логики. Единственный вариант на Web, на native — для отладки.
    Inline,
}

impl RenderMode {
    /// Режим по умолчанию для платформы.
    // [Rust] `cfg!(...)` — условие на этапе компиляции, но в виде обычного bool:
    // обе ветки должны компилироваться, лишняя выбрасывается оптимизатором.
    pub const PLATFORM_DEFAULT: Self = if cfg!(target_arch = "wasm32") {
        Self::Inline
    } else {
        Self::Threaded
    };
}

impl Default for RenderMode {
    fn default() -> Self {
        Self::PLATFORM_DEFAULT
    }
}

// Чтобы режим можно было хранить в CVar: разбор из строки и обратно
impl CVarValue for RenderMode {
    const TYPE_NAME: &'static str = "threaded | inline";
    fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "threaded" => Ok(Self::Threaded),
            "inline" => Ok(Self::Inline),
            other => Err(format!("ожидается threaded или inline, получено «{other}»")),
        }
    }
    fn format(&self) -> String {
        match self {
            Self::Threaded => "threaded",
            Self::Inline => "inline",
        }
        .to_owned()
    }
}

/// Режим рендера. Читается при добавлении `RenderPlugin`.
pub static RENDER_MODE: CVar<RenderMode> = CVar::new(
    "render.mode",
    RenderMode::PLATFORM_DEFAULT,
    "Где выполняется рендер: threaded (свой поток) или inline (главный поток)",
)
.with_flags(CVarFlags::STARTUP);

pub static RENDER_BACKEND: CVar<CVarString> = CVar::new(
    "render.backend",
    Cow::Borrowed("auto"),
    "Графический API: auto, vulkan, metal, gl, webgpu",
)
.with_flags(CVarFlags::STARTUP);

pub static RENDER_HEADLESS: CVar<bool> = CVar::new(
    "render.headless",
    false,
    "Рисовать без окна в текстуру (тесты, серверный рендер)",
)
.with_flags(CVarFlags::STARTUP);

pub static RENDER_HEADLESS_WIDTH: CVar<u32> =
    CVar::new("render.headless_width", 1280, "Ширина кадра без окна")
        .with_flags(CVarFlags::STARTUP);
pub static RENDER_HEADLESS_HEIGHT: CVar<u32> =
    CVar::new("render.headless_height", 720, "Высота кадра без окна")
        .with_flags(CVarFlags::STARTUP);

pub static SCREENSHOT_PATH: CVar<CVarString> = CVar::new(
    "render.screenshot",
    Cow::Borrowed(""),
    "Сохранить кадр в PNG по этому пути (пусто — не сохранять)",
);
pub static SCREENSHOT_FRAME: CVar<u64> = CVar::new(
    "render.screenshot_frame",
    120,
    "На каком кадре логики делать снимок",
);
pub static SCREENSHOT_EXIT: CVar<bool> = CVar::new(
    "render.screenshot_exit",
    false,
    "Выйти после сохранения снимка",
);

/// Подключает рендер. Требует `ScenePlugin` (а он — `AssetPlugin`).
#[derive(Debug, Default)]
pub struct RenderPlugin;

/// Цвет фона. Меняйте в `FixedUpdate` — рендер плавно интерполирует между шагами.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ClearColor(pub Color);

/// Статистика рендера. Обновляется в начале кадра логики.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct RenderStats {
    /// Всего нарисовано кадров.
    pub frames_rendered: u64,
    /// Снимков, которые рендер не успел увидеть (перезаписаны более новыми).
    pub snapshots_skipped: u64,
    /// Время работы последнего кадра рендера на CPU (без ожидания кадра экрана).
    pub frame_cpu_time: Duration,
    /// Нарисовано и отсечено примитивов в последнем кадре.
    pub drawn: u64,
    pub culled: u64,
    /// Ресурсов в видеопамяти.
    pub gpu_meshes: u64,
    pub gpu_textures: u64,
}

/// Сторона главного потока: отправляет команды и снимки рендеру.
///
/// Хранится как non-send ресурс: работает с ним только главный поток. Это нужно на Web,
/// где объекты `wgpu` нельзя передавать между потоками.
pub(crate) struct RenderFrontend {
    commands: MailboxSender<RenderCommand>,
    pub(crate) snapshots: Producer<FrameSnapshot>,
    stats: Arc<SharedStats>,
    /// Создаётся при первом появлении окна: на Android для GLES нужен display окна.
    instance: Option<wgpu::Instance>,
    backend: Backend,
}

enum Backend {
    // [Rust] Вариант существует только на native: на wasm его нет даже в типе,
    // и случайно включить поток там невозможно.
    #[cfg(not(target_arch = "wasm32"))]
    Threaded(Option<std::thread::JoinHandle<()>>),
    Inline(Box<Renderer>),
}

/// Ожидание скриншота: опрашивайте `try_get()` в следующих кадрах.
pub type ScreenshotReceiver = ReplyReceiver<Result<Screenshot, String>>;

/// Запрашивает снимок ближайшего кадра рендера.
pub fn request_screenshot(world: &mut World) -> ScreenshotReceiver {
    let (tx, rx) = reply();
    world
        .non_send_mut::<RenderFrontend>()
        .send(RenderCommand::Screenshot { reply: tx });
    rx
}

fn backends() -> wgpu::Backends {
    match RENDER_BACKEND.get().as_ref() {
        "vulkan" => wgpu::Backends::VULKAN,
        "metal" => wgpu::Backends::METAL,
        "gl" => wgpu::Backends::GL,
        "webgpu" => wgpu::Backends::BROWSER_WEBGPU,
        other => {
            if other != "auto" {
                tracing::warn!("render.backend = «{other}» неизвестен, используется auto");
            }
            let mut backends =
                wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::BROWSER_WEBGPU;
            if cfg!(target_os = "android") {
                backends |= wgpu::Backends::GL; // GLES3 для старых устройств (решение D3)
            }
            backends
        }
    }
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        assert!(
            app.is_plugin_added::<engine_scene::ScenePlugin>(),
            "RenderPlugin требует ScenePlugin: добавьте его раньше"
        );
        app.register_cvar(&RENDER_MODE)
            .register_cvar(&RENDER_BACKEND)
            .register_cvar(&RENDER_HEADLESS)
            .register_cvar(&RENDER_HEADLESS_WIDTH)
            .register_cvar(&RENDER_HEADLESS_HEIGHT)
            .register_cvar(&SCREENSHOT_PATH)
            .register_cvar(&SCREENSHOT_FRAME)
            .register_cvar(&SCREENSHOT_EXIT)
            .register_cvar(&crate::debug_draw::DEBUG_GRID)
            .register_cvar(&crate::debug_draw::DEBUG_BOUNDS)
            .register_cvar(&crate::debug_draw::DEBUG_AXES)
            .register_cvar(&crate::ui::DEBUG_UI);
        let headless = RENDER_HEADLESS.get();
        // Без окна нет цикла событий платформы — кадры рисует главный поток
        let mode = if headless {
            RenderMode::Inline
        } else {
            RENDER_MODE.get()
        };
        let (commands_tx, commands_rx) = mailbox("render");
        let (producer, consumer) = triple_buffer(&FrameSnapshot::default());
        let stats = Arc::new(SharedStats::default());
        let renderer = Renderer::new(commands_rx, consumer, Arc::clone(&stats));

        let backend = match mode {
            #[cfg(not(target_arch = "wasm32"))]
            RenderMode::Threaded => Backend::Threaded(Some(spawn_render_thread(renderer))),
            #[cfg(target_arch = "wasm32")]
            RenderMode::Threaded => {
                tracing::warn!("На Web нет потоков, рендер работает в главном потоке");
                Backend::Inline(Box::new(renderer))
            }
            RenderMode::Inline => Backend::Inline(Box::new(renderer)),
        };
        tracing::info!(
            "Рендер: режим {mode:?}{}",
            if headless { ", без окна" } else { "" }
        );

        let frontend = RenderFrontend {
            commands: commands_tx,
            snapshots: producer,
            stats,
            instance: None,
            backend,
        };
        if headless {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: backends(),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            frontend.send(RenderCommand::InitHeadless {
                instance,
                size: WindowSize {
                    width: RENDER_HEADLESS_WIDTH.get(),
                    height: RENDER_HEADLESS_HEIGHT.get(),
                },
            });
        }
        app.world_mut().insert_non_send(frontend);
        app.init_resource::<ClearColor>()
            .init_resource::<RenderStats>()
            .init_resource::<crate::DebugDraw>()
            .init_resource::<crate::Ui>()
            .add_systems(Stage::PreUpdate, crate::ui::ui_begin)
            .add_systems(Stage::PostUpdate, crate::debug_draw::builtin_debug_draw)
            .add_systems(Stage::First, update_stats)
            .add_systems(
                Stage::Extract,
                (
                    crate::ui::ui_end,
                    crate::extract::sync_assets,
                    crate::extract::extract_frame,
                )
                    .chain(),
            )
            .add_systems(Stage::Last, (render_inline, save_screenshot))
            .add_lifecycle_hook(|world, event| {
                world.non_send_mut::<RenderFrontend>().on_lifecycle(event);
            });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_render_thread(mut renderer: Renderer) -> std::thread::JoinHandle<()> {
    use crate::renderer::Flow;

    std::thread::Builder::new()
        .name("render".to_owned())
        // [Rust] `move` — замыкание забирает renderer себе (перемещение в новый поток).
        // Компилятор проверяет, что Renderer: Send, иначе не соберётся.
        .spawn(move || {
            engine_platform::profiling::set_thread_name("render");
            tracing::info!("Поток рендера запущен");
            loop {
                if renderer.is_idle() && renderer.wait_command() == Flow::Exit {
                    break;
                }
                if renderer.process_commands() == Flow::Exit {
                    break;
                }
                renderer.poll_init();
                if !renderer.render_frame() {
                    // Нечего рисовать (окно скрыто, идёт инициализация): не крутим процессор
                    std::thread::sleep(Duration::from_millis(4));
                }
            }
            tracing::info!("Поток рендера остановлен");
        })
        .expect("не удалось создать поток рендера")
}

impl RenderFrontend {
    pub(crate) fn send(&self, command: RenderCommand) {
        self.commands.send(command);
    }

    fn on_lifecycle(&mut self, event: &LifecycleHook<'_>) {
        match event {
            LifecycleHook::Resumed(window) => self.create_surface(window),
            LifecycleHook::Suspended => self.release_surface(),
            LifecycleHook::Resized { size, .. } => self.send(RenderCommand::Resize(*size)),
            LifecycleHook::Exiting => self.shutdown(),
        }
    }

    /// Surface создаётся в главном потоке: на macOS и iOS работать со слоем окна
    /// можно только из него.
    fn create_surface(&mut self, window: &PlatformWindow) {
        let instance = self.instance.get_or_insert_with(|| {
            wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: backends(),
                ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone()))
            })
        });
        // PlatformWindow владеет окном через Arc, поэтому surface может жить сколько угодно
        match instance.create_surface(window.clone()) {
            Ok(surface) => {
                let command = RenderCommand::SurfaceCreated {
                    instance: instance.clone(),
                    surface,
                    size: window.size(),
                };
                self.send(command);
            }
            Err(error) => tracing::error!("Не удалось создать surface: {error}"),
        }
        self.pump_inline();
    }

    /// Уход в фон: ждём, пока рендер отпустит surface. Только после этого
    /// можно вернуть управление ОС — она уничтожит окно.
    fn release_surface(&mut self) {
        let (reply_tx, reply_rx) = reply();
        self.send(RenderCommand::SurfaceLost { reply: reply_tx });
        // В режиме Inline рендер — это мы сами: выполняем команду прямо здесь
        self.pump_inline();
        let started = Instant::now();
        if reply_rx.wait(Duration::from_secs(2)).is_some() {
            tracing::info!("Surface отпущен за {:?}", started.elapsed());
        } else {
            tracing::error!("Рендер не отпустил surface за 2 с");
        }
    }

    fn shutdown(&mut self) {
        self.send(RenderCommand::Shutdown);
        match &mut self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Threaded(handle) => {
                // [Rust] `take()` забирает значение из Option, оставляя None:
                // join можно вызвать только один раз.
                if let Some(handle) = handle.take()
                    && handle.join().is_err()
                {
                    tracing::error!("Поток рендера завершился паникой");
                }
            }
            Backend::Inline(_) => {}
        }
    }

    /// В режиме Inline сразу выполняет накопившиеся команды.
    // На Web у Backend единственный вариант Inline, и `if let` всегда срабатывает
    #[cfg_attr(target_arch = "wasm32", allow(irrefutable_let_patterns))]
    fn pump_inline(&mut self) {
        if let Backend::Inline(renderer) = &mut self.backend {
            renderer.process_commands();
            renderer.poll_init();
        }
    }
}

#[cfg_attr(target_arch = "wasm32", allow(irrefutable_let_patterns))]
fn render_inline(mut frontend: NonSendMut<RenderFrontend>) {
    if let Backend::Inline(renderer) = &mut frontend.backend {
        renderer.process_commands();
        renderer.poll_init();
        renderer.render_frame();
    }
}

fn update_stats(frontend: NonSend<RenderFrontend>, mut stats: ResMut<RenderStats>) {
    let s = &frontend.stats;
    stats.frames_rendered = s.frames_rendered.load(Ordering::Relaxed);
    stats.snapshots_skipped = s.snapshots_skipped.load(Ordering::Relaxed);
    stats.frame_cpu_time = Duration::from_nanos(s.frame_cpu_nanos.load(Ordering::Relaxed));
    stats.drawn = s.drawn.load(Ordering::Relaxed);
    stats.culled = s.culled.load(Ordering::Relaxed);
    stats.gpu_meshes = s.gpu_meshes.load(Ordering::Relaxed);
    stats.gpu_textures = s.gpu_textures.load(Ordering::Relaxed);
}

/// CVar `render.screenshot`: на кадре `render.screenshot_frame` снимает кадр и сохраняет PNG.
fn save_screenshot(world: &mut World, mut pending: Local<Option<ScreenshotReceiver>>) {
    let path = SCREENSHOT_PATH.get();
    if path.is_empty() {
        return;
    }
    if pending.is_none() && world.resource::<Time>().frame() == SCREENSHOT_FRAME.get() {
        *pending = Some(request_screenshot(world));
        return;
    }
    let Some(result) = pending.as_ref().and_then(ReplyReceiver::try_get) else {
        return;
    };
    *pending = None;
    match result.and_then(|shot| crate::save_png(&shot, &path)) {
        Ok(()) => tracing::info!("Скриншот сохранён: {path}"),
        Err(error) => tracing::error!("Скриншот не сохранён: {error}"),
    }
    if SCREENSHOT_EXIT.get() {
        world.resource_mut::<AppControl>().request_exit();
    }
}
