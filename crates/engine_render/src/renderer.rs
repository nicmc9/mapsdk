use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use engine_core::sync::mailbox::Mailbox;
use engine_core::sync::triple_buffer::Consumer;
use engine_platform::{Instant, WindowSize};

use crate::command::RenderCommand;
use crate::gpu::{Gpu, GpuInit};
use crate::snapshot::FrameSnapshot;

/// Счётчики, которые рендер пишет, а главный поток читает для статистики.
#[derive(Debug, Default)]
pub(crate) struct SharedStats {
    pub frames_rendered: AtomicU64,
    pub snapshots_skipped: AtomicU64,
}

/// Продолжать работу или завершиться.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    Exit,
}

/// Окно, в которое рисуем.
struct Target {
    surface: wgpu::Surface<'static>,
    size: WindowSize,
    /// `None` — ещё не настроен (нет GPU или окно нулевого размера).
    config: Option<wgpu::SurfaceConfiguration>,
}

/// Рендер: владеет GPU и окном отрисовки. Работает в своём потоке или в главном —
/// код одинаковый, отличается только то, кто вызывает методы.
pub(crate) struct Renderer {
    commands: Mailbox<RenderCommand>,
    snapshots: Consumer<FrameSnapshot>,
    stats: Arc<SharedStats>,
    gpu: Option<Gpu>,
    init: Option<GpuInit>,
    target: Option<Target>,
}

impl Renderer {
    pub fn new(
        commands: Mailbox<RenderCommand>,
        snapshots: Consumer<FrameSnapshot>,
        stats: Arc<SharedStats>,
    ) -> Self {
        Self {
            commands,
            snapshots,
            stats,
            gpu: None,
            init: None,
            target: None,
        }
    }

    /// Можно ли рисовать прямо сейчас.
    pub fn can_render(&self) -> bool {
        self.gpu.is_some() && self.target.as_ref().is_some_and(|t| t.config.is_some())
    }

    /// Нечего делать, пока не придёт команда: нет окна или GPU и инициализация не идёт.
    #[cfg(not(target_arch = "wasm32"))] // нужен только потоку рендера
    pub fn is_idle(&self) -> bool {
        !self.can_render() && self.init.is_none()
    }

    /// Блокирующе ждёт одну команду и выполняет её. Для потока рендера в простое.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_command(&mut self) -> Flow {
        match self.commands.recv() {
            Some(command) => self.handle(command),
            // Все отправители уничтожены — главный поток завершился
            None => Flow::Exit,
        }
    }

    /// Выполняет все накопившиеся команды без ожидания.
    pub fn process_commands(&mut self) -> Flow {
        while let Some(command) = self.commands.try_recv() {
            if self.handle(command) == Flow::Exit {
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    fn handle(&mut self, command: RenderCommand) -> Flow {
        log::debug!("Рендер: команда {command:?}");
        match command {
            RenderCommand::SurfaceCreated {
                instance,
                surface,
                size,
            } => {
                if self.gpu.is_none() && self.init.is_none() {
                    self.init = Some(GpuInit::start(&instance, &surface));
                }
                self.target = Some(Target {
                    surface,
                    size,
                    config: None,
                });
                self.configure();
            }
            RenderCommand::SurfaceLost { reply } => {
                // Surface уничтожается здесь — до ответа главному потоку
                self.target = None;
                reply.send(());
                log::info!("Рендер: surface отпущен");
            }
            RenderCommand::Resize(size) => {
                if let Some(target) = &mut self.target {
                    // При переносе окна между экранами winit присылает и смену размера,
                    // и смену масштаба — второй раз перенастраивать незачем
                    if target.size == size && target.config.is_some() {
                        return Flow::Continue;
                    }
                    target.size = size;
                }
                self.configure();
            }
            RenderCommand::Shutdown => return Flow::Exit,
        }
        Flow::Continue
    }

    /// Проверяет асинхронную инициализацию GPU.
    pub fn poll_init(&mut self) {
        let Some(init) = &mut self.init else { return };
        match init.poll() {
            None => {}
            Some(Ok(gpu)) => {
                let info = gpu.adapter.get_info();
                log::info!(
                    "GPU: {} ({:?}, драйвер {})",
                    info.name,
                    info.backend,
                    info.driver_info
                );
                self.gpu = Some(gpu);
                self.init = None;
                self.configure();
            }
            Some(Err(error)) => {
                log::error!("Не удалось инициализировать GPU: {error}");
                self.init = None;
            }
        }
    }

    /// (Пере)настраивает surface под текущий размер окна.
    fn configure(&mut self) {
        let (Some(gpu), Some(target)) = (&self.gpu, &mut self.target) else {
            return;
        };
        let WindowSize { width, height } = target.size;
        if width == 0 || height == 0 {
            // Свёрнутое окно: настраивать surface нулевого размера нельзя
            target.config = None;
            return;
        }
        let Some(mut config) = target
            .surface
            .get_default_config(&gpu.adapter, width, height)
        else {
            log::error!("Surface несовместим с адаптером");
            return;
        };
        // Fifo = вертикальная синхронизация: get_current_texture ждёт свободный кадр
        // экрана, и это задаёт темп потока рендера. Поддерживается везде.
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        target.surface.configure(&gpu.device, &config);
        // Рендер (снова) готов рисовать: пропуски за время простоя не считаем
        self.snapshots.resync();
        log::info!("Surface: {width}x{height}, {:?}", config.format);
        target.config = Some(config);
    }

    /// Рисует кадр. `false` — кадр не нарисован (не готово, окно скрыто и т.п.).
    pub fn render_frame(&mut self) -> bool {
        if !self.can_render() {
            return false;
        }
        self.snapshots.take();
        self.stats
            .snapshots_skipped
            .store(self.snapshots.skipped(), Ordering::Relaxed);

        // [Rust] Выше проверено, что gpu и target есть, но компилятор об этом не знает.
        // `let ... else` достаёт значения или выходит из функции.
        let (Some(gpu), Some(target)) = (&self.gpu, &self.target) else {
            return false;
        };
        let acquire_started = Instant::now();
        let acquired = target.surface.get_current_texture();
        let waited = acquire_started.elapsed();
        if waited.as_millis() > 100 {
            log::debug!("Рендер: ожидание кадра экрана {waited:?}");
        }
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                log::debug!("Рендер: кадр экрана недоступен (окно скрыто или таймаут)");
                return false;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                log::warn!("Surface устарел, перенастраиваем");
                self.configure();
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("Ошибка валидации при получении кадра");
                return false;
            }
        };

        let clear = self
            .snapshots
            .current()
            .interpolated_clear_color(Instant::now());
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            // [Rust] Блок ограничивает жизнь render pass: он заимствует encoder,
            // и `encoder.finish()` ниже возможен только после выхода из блока.
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear.to_wgpu()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(frame);
        self.stats.frames_rendered.fetch_add(1, Ordering::Relaxed);
        true
    }
}
