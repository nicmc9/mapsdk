use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use engine_core::sync::mailbox::Mailbox;
use engine_core::sync::reply::ReplySender;
use engine_core::sync::triple_buffer::Consumer;
use engine_platform::{Instant, WindowSize};

use crate::command::{RenderCommand, Screenshot};
use crate::gpu::{Gpu, GpuInit};
use crate::graph::{PassKind, RenderGraph, TextureDesc};
use crate::scene::{DEPTH_FORMAT, SceneRenderer};
use crate::snapshot::FrameSnapshot;

/// Счётчики, которые рендер пишет, а главный поток читает для статистики.
#[derive(Debug, Default)]
pub(crate) struct SharedStats {
    pub frames_rendered: AtomicU64,
    pub snapshots_skipped: AtomicU64,
    pub frame_cpu_nanos: AtomicU64,
    pub drawn: AtomicU64,
    pub culled: AtomicU64,
    pub gpu_meshes: AtomicU64,
    pub gpu_textures: AtomicU64,
}

/// Продолжать работу или завершиться.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    Exit,
}

/// Куда рисуем: окно или текстура в памяти (offscreen).
enum TargetKind {
    Surface {
        surface: wgpu::Surface<'static>,
        /// `None` — ещё не настроен (нет GPU или окно нулевого размера).
        config: Option<wgpu::SurfaceConfiguration>,
    },
    Offscreen {
        texture: Option<wgpu::Texture>,
    },
}

struct Target {
    kind: TargetKind,
    size: WindowSize,
    /// Формат, в котором рисуют пайплайны (для окна — sRGB-вариант, если есть).
    view_format: Option<wgpu::TextureFormat>,
}

/// Рендер: владеет GPU и целью отрисовки. Работает в своём потоке или в главном —
/// код одинаковый, отличается только то, кто вызывает методы.
pub(crate) struct Renderer {
    commands: Mailbox<RenderCommand>,
    snapshots: Consumer<FrameSnapshot>,
    stats: Arc<SharedStats>,
    gpu: Option<Gpu>,
    init: Option<GpuInit>,
    target: Option<Target>,
    scene: Option<SceneRenderer>,
    /// Ресурсы, пришедшие до готовности GPU, — применятся после инициализации.
    pending: Vec<RenderCommand>,
    screenshots: Vec<ReplySender<Result<Screenshot, String>>>,
    ui: Option<egui_wgpu::Renderer>,
    /// Временные текстуры render graph, переиспользуемые между кадрами.
    pool: crate::graph::TexturePool,
    /// Текстуры интерфейса, которые нужно освободить после отрисовки кадра.
    ui_free: Vec<egui::TextureId>,
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
            scene: None,
            pending: Vec::new(),
            screenshots: Vec::new(),
            ui: None,
            ui_free: Vec::new(),
            pool: crate::graph::TexturePool::default(),
        }
    }

    /// Можно ли рисовать прямо сейчас.
    pub fn can_render(&self) -> bool {
        self.gpu.is_some()
            && self.scene.is_some()
            && self
                .target
                .as_ref()
                .is_some_and(|t| t.view_format.is_some())
    }

    /// Нечего делать, пока не придёт команда: нет окна или GPU и инициализация не идёт.
    #[cfg(not(target_arch = "wasm32"))] // нужен только потоку рендера
    pub fn is_idle(&self) -> bool {
        !self.can_render() && self.init.is_none()
    }

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
        tracing::trace!("Рендер: команда {command:?}");
        match command {
            RenderCommand::SurfaceCreated {
                instance,
                surface,
                size,
            } => {
                if self.gpu.is_none() && self.init.is_none() {
                    self.init = Some(GpuInit::start(&instance, Some(&surface)));
                }
                self.target = Some(Target {
                    kind: TargetKind::Surface {
                        surface,
                        config: None,
                    },
                    size,
                    view_format: None,
                });
                self.configure();
            }
            RenderCommand::InitHeadless { instance, size } => {
                self.init = Some(GpuInit::start(&instance, None));
                self.target = Some(Target {
                    kind: TargetKind::Offscreen { texture: None },
                    size,
                    view_format: None,
                });
            }
            RenderCommand::SurfaceLost { reply } => {
                // Surface уничтожается здесь — до ответа главному потоку
                self.target = None;
                reply.send(());
                tracing::info!("Рендер: surface отпущен");
            }
            RenderCommand::Resize(size) => {
                if let Some(target) = &mut self.target {
                    // При переносе окна между экранами winit присылает и смену размера,
                    // и смену масштаба — второй раз перенастраивать незачем
                    if target.size == size && target.view_format.is_some() {
                        return Flow::Continue;
                    }
                    target.size = size;
                }
                self.configure();
            }
            RenderCommand::Screenshot { reply } => self.screenshots.push(reply),
            RenderCommand::Shutdown => return Flow::Exit,
            resource => self.apply_resource(resource),
        }
        Flow::Continue
    }

    fn apply_resource(&mut self, command: RenderCommand) {
        let (Some(gpu), Some(scene)) = (&self.gpu, &mut self.scene) else {
            self.pending.push(command);
            return;
        };
        match command {
            RenderCommand::UploadMesh(id, mesh) => scene.upload_mesh(&gpu.device, id, &mesh),
            RenderCommand::UploadImage(id, image) => {
                scene.upload_image(&gpu.device, &gpu.queue, id, &image);
            }
            RenderCommand::UploadMaterial(id, data) => scene.upload_material(&gpu.device, id, data),
            RenderCommand::RemoveMesh(id) => scene.remove_mesh(id),
            RenderCommand::RemoveImage(id) => scene.remove_image(id),
            RenderCommand::RemoveMaterial(id) => scene.remove_material(id),
            RenderCommand::UiTextures(delta) => {
                let delta = &delta.0;
                if let Some(ui) = &mut self.ui {
                    for (id, images) in &delta.set {
                        for image in images {
                            ui.update_texture(&gpu.device, &gpu.queue, *id, image);
                        }
                    }
                }
                // Обёртка UiTextures пометит изменения обработанными при уничтожении
                self.ui_free.extend(delta.free.iter().copied());
            }
            _ => {}
        }
        let (meshes, textures, _) = scene.counts();
        self.stats
            .gpu_meshes
            .store(meshes as u64, Ordering::Relaxed);
        self.stats
            .gpu_textures
            .store(textures as u64, Ordering::Relaxed);
    }

    /// Проверяет асинхронную инициализацию GPU.
    pub fn poll_init(&mut self) {
        let Some(init) = &mut self.init else { return };
        match init.poll() {
            None => {}
            Some(Ok(gpu)) => {
                let info = gpu.adapter.get_info();
                tracing::info!(
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
                tracing::error!("Не удалось инициализировать GPU: {error}");
                self.init = None;
            }
        }
    }

    /// (Пере)настраивает цель отрисовки под текущий размер.
    fn configure(&mut self) {
        let (Some(gpu), Some(target)) = (&self.gpu, &mut self.target) else {
            return;
        };
        let WindowSize { width, height } = target.size;
        if width == 0 || height == 0 {
            // Свёрнутое окно: настраивать цель нулевого размера нельзя
            target.view_format = None;
            return;
        }
        let view_format = match &mut target.kind {
            TargetKind::Surface { surface, config } => {
                let Some(mut new_config) = surface.get_default_config(&gpu.adapter, width, height)
                else {
                    tracing::error!("Surface несовместим с адаптером");
                    return;
                };
                // Fifo = вертикальная синхронизация: get_current_texture ждёт свободный кадр
                // экрана, и это задаёт темп потока рендера. Поддерживается везде.
                new_config.present_mode = wgpu::PresentMode::Fifo;
                new_config.desired_maximum_frame_latency = 2;
                let capabilities = surface.get_capabilities(&gpu.adapter);
                if capabilities.usages.contains(wgpu::TextureUsages::COPY_SRC) {
                    new_config.usage |= wgpu::TextureUsages::COPY_SRC; // для скриншотов
                }
                // На Web формат окна не sRGB. Рисуем через sRGB-«вид» той же текстуры, если
                // GPU умеет; иначе шейдер кодирует sRGB сам (флаг в uniform)
                let srgb = new_config.format.add_srgb_suffix();
                let downlevel = gpu.adapter.get_downlevel_capabilities();
                let format = if srgb != new_config.format
                    && downlevel
                        .flags
                        .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS)
                {
                    new_config.view_formats.push(srgb);
                    srgb
                } else {
                    new_config.format
                };
                surface.configure(&gpu.device, &new_config);
                tracing::info!(
                    "Surface: {width}x{height}, {:?} (рисуем в {format:?})",
                    new_config.format
                );
                *config = Some(new_config);
                format
            }
            TargetKind::Offscreen { texture } => {
                let format = wgpu::TextureFormat::Rgba8UnormSrgb;
                *texture = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("offscreen"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                }));
                tracing::info!("Offscreen: {width}x{height}, {format:?}");
                format
            }
        };
        target.view_format = Some(view_format);
        // Временные текстуры графа (глубина и т.п.) старого размера больше не нужны
        self.pool.retain_size((width, height));

        if self.scene.is_none() {
            self.scene = Some(SceneRenderer::new(&gpu.device, &gpu.queue, view_format));
            self.ui = Some(egui_wgpu::Renderer::new(
                &gpu.device,
                view_format,
                egui_wgpu::RendererOptions::default(),
            ));
            // Ресурсы, пришедшие до готовности GPU
            for command in std::mem::take(&mut self.pending) {
                self.apply_resource(command);
            }
        }
        // Рендер (снова) готов рисовать: пропуски за время простоя не считаем
        self.snapshots.resync();
    }

    /// После отправки кадра: освобождение текстур интерфейса и статистика.
    fn finish_frame(&mut self, work_started: Instant, draw: crate::scene::DrawStats) {
        if let Some(ui) = &mut self.ui {
            for id in self.ui_free.drain(..) {
                ui.free_texture(&id);
            }
        }
        let work = u64::try_from(work_started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.stats.frame_cpu_nanos.store(work, Ordering::Relaxed);
        self.stats
            .drawn
            .store(u64::from(draw.drawn), Ordering::Relaxed);
        self.stats
            .culled
            .store(u64::from(draw.culled), Ordering::Relaxed);
        self.stats.frames_rendered.fetch_add(1, Ordering::Relaxed);
    }

    /// Рисует кадр. `false` — кадр не нарисован (не готово, окно скрыто и т.п.).
    pub fn render_frame(&mut self) -> bool {
        if !self.can_render() {
            return false;
        }
        let _zone = tracing::trace_span!("render_frame").entered();
        self.snapshots.take();
        self.stats
            .snapshots_skipped
            .store(self.snapshots.skipped(), Ordering::Relaxed);

        // [Rust] Выше проверено, что всё есть, но компилятор об этом не знает:
        // `let ... else` достаёт значения или выходит из функции.
        let (Some(gpu), Some(target), Some(scene)) = (&self.gpu, &self.target, &mut self.scene)
        else {
            return false;
        };
        let Some(view_format) = target.view_format else {
            return false;
        };

        // Цель: кадр окна или offscreen-текстура
        let acquire_started = Instant::now();
        let surface_frame = match &target.kind {
            TargetKind::Surface { surface, .. } => match acquire(surface) {
                Acquired::Frame(frame) => Some(frame),
                Acquired::Skip => return false,
                Acquired::Reconfigure => {
                    self.configure();
                    return false;
                }
            },
            TargetKind::Offscreen { .. } => None,
        };
        let waited = acquire_started.elapsed();
        if waited.as_millis() > 100 {
            tracing::debug!("Рендер: ожидание кадра экрана {waited:?}");
        }
        let work_started = Instant::now();
        let texture = match (&surface_frame, &target.kind) {
            (Some(frame), _) => &frame.texture,
            (
                None,
                TargetKind::Offscreen {
                    texture: Some(texture),
                },
            ) => texture,
            _ => return false,
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(view_format),
            ..Default::default()
        });

        let snapshot = self.snapshots.current();
        let alpha = snapshot.alpha(Instant::now());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let inputs = FrameInputs {
            gpu,
            view: &view,
            size: target.size,
            snapshot,
            alpha,
        };
        let (draw, ui_commands) = execute_graph(
            &inputs,
            &mut encoder,
            scene,
            self.ui.as_mut(),
            &mut self.pool,
        );

        // Скриншот: копия кадра в буфер, читаемый процессором
        let readback = (!self.screenshots.is_empty())
            .then(|| crate::screenshot::copy_to_buffer(&gpu.device, &mut encoder, texture));
        {
            let _zone = tracing::trace_span!("submit_present").entered();
            gpu.queue
                .submit(ui_commands.into_iter().chain([encoder.finish()]));
            if let Some(readback) = readback {
                let result = readback.read(&gpu.device);
                for reply in self.screenshots.drain(..) {
                    reply.send(result.clone());
                }
            }
            if let Some(frame) = surface_frame {
                gpu.queue.present(frame);
            }
        }
        self.finish_frame(work_started, draw);
        engine_platform::profiling::render_frame_mark();
        true
    }
}

enum Acquired {
    Frame(wgpu::SurfaceTexture),
    Skip,
    Reconfigure,
}

/// Следующий кадр окна. Здесь поток ждёт свободный кадр экрана (vsync).
fn acquire(surface: &wgpu::Surface<'static>) -> Acquired {
    let acquired = {
        let _zone = tracing::trace_span!("acquire_frame").entered();
        surface.get_current_texture()
    };
    match acquired {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Acquired::Frame(frame),
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
            tracing::debug!("Рендер: кадр экрана недоступен (окно скрыто или таймаут)");
            Acquired::Skip
        }
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            tracing::warn!("Surface устарел, перенастраиваем");
            Acquired::Reconfigure
        }
        wgpu::CurrentSurfaceTexture::Validation => {
            tracing::error!("Ошибка валидации при получении кадра");
            Acquired::Skip
        }
    }
}

/// Интерфейс поверх сцены. Возвращает командные буферы egui (загрузка вершин),
/// которые нужно отправить раньше основного.
fn render_ui(
    ui: &mut egui_wgpu::Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    size: WindowSize,
    frame: &crate::ui::UiFrame,
) -> Vec<wgpu::CommandBuffer> {
    if frame.primitives.is_empty() {
        return Vec::new();
    }
    let _zone = tracing::trace_span!("render_ui").entered();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [size.width, size.height],
        pixels_per_point: frame.pixels_per_point,
    };
    let commands = ui.update_buffers(device, queue, encoder, &frame.primitives, &screen);
    let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("ui"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load, // поверх сцены
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    // [Rust] egui-wgpu требует проход с временем жизни 'static. `forget_lifetime` снимает
    // привязку прохода к encoder — wgpu сам проверит порядок во время выполнения.
    let mut pass = pass.forget_lifetime();
    ui.render(&mut pass, &frame.primitives, &screen);
    commands
}

/// Всё, что нужно проходам кадра, кроме изменяемых частей рендера.
struct FrameInputs<'a> {
    gpu: &'a Gpu,
    view: &'a wgpu::TextureView,
    size: WindowSize,
    snapshot: &'a FrameSnapshot,
    alpha: f32,
}

/// Строит граф кадра и выполняет проходы по порядку. Сюда добавляются новые проходы
/// (тени, постэффекты): ресурс + проход в графе + ветка в `match`.
fn execute_graph(
    frame: &FrameInputs<'_>,
    encoder: &mut wgpu::CommandEncoder,
    scene: &mut SceneRenderer,
    mut ui: Option<&mut egui_wgpu::Renderer>,
    pool: &mut crate::graph::TexturePool,
) -> (crate::scene::DrawStats, Vec<wgpu::CommandBuffer>) {
    let gpu = frame.gpu;
    // Сцена пишет в цель и глубину, интерфейс дорисовывает поверх
    let mut graph = RenderGraph::default();
    let color = graph.import("target");
    let depth_id = graph.transient(
        "depth",
        TextureDesc {
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        },
    );
    graph.add_pass(PassKind::Scene, &[], &[color, depth_id]);
    graph.add_pass(PassKind::Ui, &[], &[color]);
    let order = graph.compile(color).unwrap_or_else(|error| {
        tracing::error!("Render graph: {error}");
        Vec::new()
    });
    let size = (frame.size.width, frame.size.height);
    let transients: std::collections::HashMap<_, _> = graph
        .transients()
        .map(|(id, name, desc)| (id, pool.get(&gpu.device, name, desc, size)))
        .collect();

    let mut draw = crate::scene::DrawStats::default();
    let mut ui_commands = Vec::new();
    for pass in order {
        let _zone = tracing::trace_span!("pass", ?pass).entered();
        match pass {
            PassKind::Scene => {
                let target = crate::scene::FrameTarget {
                    color: frame.view,
                    depth: &transients[&depth_id],
                    size,
                };
                draw = scene.render(
                    &gpu.device,
                    &gpu.queue,
                    encoder,
                    &target,
                    frame.snapshot,
                    frame.alpha,
                );
            }
            PassKind::Ui => {
                if let Some(ui) = ui.as_deref_mut() {
                    ui_commands = render_ui(
                        ui,
                        &gpu.device,
                        &gpu.queue,
                        encoder,
                        frame.view,
                        frame.size,
                        &frame.snapshot.ui,
                    );
                }
            }
        }
    }
    (draw, ui_commands)
}
