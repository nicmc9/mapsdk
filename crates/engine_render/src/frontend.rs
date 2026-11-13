use std::sync::Arc;
use std::sync::atomic::Ordering;

use engine_core::sync::mailbox::{MailboxSender, mailbox};
use engine_core::sync::reply::reply;
use engine_core::sync::triple_buffer::{Producer, triple_buffer};
use engine_core::{App, FixedTime, LifecycleHook, Plugin, Stage, Time};
use engine_ecs::prelude::*;
use engine_platform::{Duration, Instant, PlatformWindow};

use crate::Color;
use crate::command::RenderCommand;
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

impl Default for RenderMode {
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self::Inline
        } else {
            Self::Threaded
        }
    }
}

/// Подключает рендер.
#[derive(Debug, Default)]
pub struct RenderPlugin {
    pub mode: RenderMode,
}

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
}

/// Сторона главного потока: отправляет команды и снимки рендеру.
///
/// Хранится как non-send ресурс: работает с ним только главный поток. Это нужно на Web,
/// где объекты `wgpu` нельзя передавать между потоками.
struct RenderFrontend {
    commands: MailboxSender<RenderCommand>,
    snapshots: Producer<FrameSnapshot>,
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

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        let (commands_tx, commands_rx) = mailbox("render");
        let (producer, consumer) = triple_buffer(&FrameSnapshot::default());
        let stats = Arc::new(SharedStats::default());
        let renderer = Renderer::new(commands_rx, consumer, Arc::clone(&stats));

        let backend = match self.mode {
            #[cfg(not(target_arch = "wasm32"))]
            RenderMode::Threaded => Backend::Threaded(Some(spawn_render_thread(renderer))),
            #[cfg(target_arch = "wasm32")]
            RenderMode::Threaded => {
                log::warn!("На Web нет потоков, рендер работает в главном потоке");
                Backend::Inline(Box::new(renderer))
            }
            RenderMode::Inline => Backend::Inline(Box::new(renderer)),
        };
        log::info!("Рендер: режим {:?}", self.mode);

        app.world_mut().insert_non_send(RenderFrontend {
            commands: commands_tx,
            snapshots: producer,
            stats,
            instance: None,
            backend,
        });
        app.init_resource::<ClearColor>()
            .init_resource::<RenderStats>()
            .add_systems(Stage::First, update_stats)
            .add_systems(Stage::Extract, extract_frame)
            .add_systems(Stage::Last, render_inline)
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
            log::info!("Поток рендера запущен");
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
            log::info!("Поток рендера остановлен");
        })
        .expect("не удалось создать поток рендера")
}

impl RenderFrontend {
    fn on_lifecycle(&mut self, event: &LifecycleHook<'_>) {
        match event {
            LifecycleHook::Resumed(window) => self.create_surface(window),
            LifecycleHook::Suspended => self.release_surface(),
            LifecycleHook::Resized { size, .. } => {
                self.commands.send(RenderCommand::Resize(*size));
            }
            LifecycleHook::Exiting => self.shutdown(),
        }
    }

    /// Surface создаётся в главном потоке: на macOS и iOS работать со слоем окна
    /// можно только из него.
    fn create_surface(&mut self, window: &PlatformWindow) {
        let instance = self.instance.get_or_insert_with(|| {
            let mut backends =
                wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::BROWSER_WEBGPU;
            if cfg!(target_os = "android") {
                backends |= wgpu::Backends::GL; // GLES3 для старых устройств (решение D3)
            }
            wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends,
                ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone()))
            })
        });
        // PlatformWindow владеет окном через Arc, поэтому surface может жить сколько угодно
        match instance.create_surface(window.clone()) {
            Ok(surface) => {
                self.commands.send(RenderCommand::SurfaceCreated {
                    instance: instance.clone(),
                    surface,
                    size: window.size(),
                });
            }
            Err(error) => log::error!("Не удалось создать surface: {error}"),
        }
        self.pump_inline();
    }

    /// Уход в фон: ждём, пока рендер отпустит surface. Только после этого
    /// можно вернуть управление ОС — она уничтожит окно.
    fn release_surface(&mut self) {
        let (reply_tx, reply_rx) = reply();
        self.commands
            .send(RenderCommand::SurfaceLost { reply: reply_tx });
        // В режиме Inline рендер — это мы сами: выполняем команду прямо здесь
        self.pump_inline();
        let started = Instant::now();
        if reply_rx.wait(Duration::from_secs(2)).is_some() {
            log::info!("Surface отпущен за {:?}", started.elapsed());
        } else {
            log::error!("Рендер не отпустил surface за 2 с");
        }
    }

    fn shutdown(&mut self) {
        self.commands.send(RenderCommand::Shutdown);
        match &mut self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Threaded(handle) => {
                // [Rust] `take()` забирает значение из Option, оставляя None:
                // join можно вызвать только один раз.
                if let Some(handle) = handle.take()
                    && handle.join().is_err()
                {
                    log::error!("Поток рендера завершился паникой");
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

/// Заполняет снимок кадра и публикует его рендеру.
fn extract_frame(
    clear_color: Res<ClearColor>,
    time: Res<Time>,
    fixed: Res<FixedTime>,
    mut frontend: NonSendMut<RenderFrontend>,
    // [Rust] `Local` — состояние, принадлежащее этой системе (как static-переменная
    // внутри функции, но своя у каждого экземпляра системы и без глобального состояния).
    mut last: Local<Option<(u64, Color)>>,
) {
    // Цвет на предыдущем шаге логики: запоминаем, что было при прошлой публикации,
    // и сдвигаем, только если с тех пор был новый шаг.
    let steps = fixed.steps_total();
    let (prev_steps, prev_color) = last.unwrap_or((steps, clear_color.0));
    let prev_clear_color = if steps == prev_steps {
        // Шагов не было — интервал интерполяции тот же
        frontend.snapshots.write().prev_clear_color
    } else {
        prev_color
    };
    *last = Some((steps, clear_color.0));

    let snapshot = frontend.snapshots.write();
    snapshot.frame = time.frame();
    snapshot.prev_clear_color = prev_clear_color;
    snapshot.clear_color = clear_color.0;
    snapshot.step = fixed.step();
    snapshot.overstep = fixed.overstep_fraction();
    snapshot.published_at = Some(Instant::now());
    frontend.snapshots.publish();
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
    stats.frames_rendered = frontend.stats.frames_rendered.load(Ordering::Relaxed);
    stats.snapshots_skipped = frontend.stats.snapshots_skipped.load(Ordering::Relaxed);
}
