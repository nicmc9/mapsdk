//! Тестовое приложение. Здесь же — точки входа для платформ, где их задаёт ОС.

use engine::assets::formats::{Gltf, Image, Mesh};
use engine::platform::{LaunchContext, PlatformConfig, PlatformError};
use engine::prelude::*;

/// Общий запуск для всех платформ.
pub fn run(launch: LaunchContext) -> Result<(), PlatformError> {
    let config = PlatformConfig {
        title: "Sandbox".to_owned(),
        ..PlatformConfig::default() // [Rust] остальные поля — из значения по умолчанию
    };
    App::new()
        // Файл встроен в бинарник: так он доступен на всех платформах. Чтение с диска
        // и из сети появится вместе с VFS (задача A-01).
        .load_config_toml(include_str!("../sandbox.toml"))
        .add_plugin(RenderPlugin)
        .add_plugin(NetPlugin)
        .add_plugin(AssetPlugin)
        .add_plugin(AssetsDemoPlugin)
        .add_plugin(SandboxPlugin)
        .run(config, launch)
}

/// Демонстрация ядра и рендера: объекты вращаются на фиксированном шаге, цвет фона
/// следует за первым объектом. Раз в секунду в лог пишется статистика кадров.
struct SandboxPlugin;

impl Plugin for SandboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameStats>()
            .add_systems(Stage::Startup, spawn_objects)
            .add_systems(Stage::FixedUpdate, (rotate, tint_background.after(rotate)))
            .add_systems(Stage::Update, (report_stats, log_lifecycle));
    }
}

#[derive(Component, Debug)]
struct Spin {
    angle: f32,
    /// Радиан в секунду
    speed: f32,
}

/// Метка: по этому объекту окрашивается фон.
// [Rust] Компонент без полей (zero-sized type) не занимает памяти — это просто тег,
// по которому Query находит нужные сущности.
#[derive(Component)]
struct TintsBackground;

#[derive(Resource, Default)]
struct FrameStats {
    frames: u32,
    last_report_steps: u64,
    last_report_rendered: u64,
    next_report_secs: f32,
}

// [Rust] Система — обычная функция. Её параметры (`Commands`, `Query`, `Res`) bevy_ecs
// заполняет сам, а по их типам понимает, какие системы можно выполнять параллельно.
fn spawn_objects(mut commands: Commands) {
    // [Rust] Кортеж компонентов — сущность получает их все сразу
    commands.spawn((
        Spin {
            angle: 0.0,
            speed: 1.0,
        },
        TintsBackground,
    ));
    for speed in [2.0, 3.0] {
        commands.spawn(Spin { angle: 0.0, speed });
    }
}

fn rotate(fixed: Res<FixedTime>, mut query: Query<&mut Spin>) {
    let dt = fixed.step_secs();
    // [Rust] `&mut Spin` в Query — изменяемый доступ к компоненту каждой сущности
    for mut spin in &mut query {
        spin.angle = (spin.angle + spin.speed * dt) % std::f32::consts::TAU;
    }
}

/// Цвет фона — оттенок по углу первого объекта: полный круг цветов за 2π/скорость секунд.
fn tint_background(
    query: Query<&Spin, With<TintsBackground>>,
    mut clear_color: ResMut<ClearColor>,
) {
    // [Rust] `single()` — ровно одна подходящая сущность, иначе Err
    if let Ok(spin) = query.single() {
        let hue = spin.angle / std::f32::consts::TAU;
        // Приглушаем цвет: смешиваем с чёрным
        clear_color.0 = Color::hue(hue).lerp(Color::BLACK, 0.5);
    }
}

fn report_stats(
    time: Res<Time>,
    fixed: Res<FixedTime>,
    window: Res<Window>,
    render: Res<RenderStats>,
    mut stats: ResMut<FrameStats>,
    query: Query<&Spin>,
) {
    stats.frames += 1;
    let now = time.elapsed().as_secs_f32();
    if now < stats.next_report_secs {
        return;
    }
    let angles: Vec<f32> = query.iter().map(|spin| spin.angle).collect();
    engine::log::info!(
        "кадров логики: {} ({:.2} мс), шагов: {}, кадров рендера: {} ({:.2} мс), пропущено снимков: {}, окно: {}x{}, углы: {angles:.2?}",
        stats.frames,
        time.cpu_time().as_secs_f64() * 1000.0,
        fixed.steps_total() - stats.last_report_steps,
        render.frames_rendered - stats.last_report_rendered,
        render.frame_cpu_time.as_secs_f64() * 1000.0,
        render.snapshots_skipped,
        window.size.width,
        window.size.height,
    );
    stats.frames = 0;
    stats.last_report_steps = fixed.steps_total();
    stats.last_report_rendered = render.frames_rendered;
    stats.next_report_secs = now + 1.0;
}

fn log_lifecycle(mut events: MessageReader<AppLifecycle>) {
    for event in events.read() {
        engine::log::info!("Событие приложения: {event:?}");
    }
}

/// Демонстрация ресурсов: модели с диска, по сети, вложенный ресурс, ошибка.
struct AssetsDemoPlugin;

impl Plugin for AssetsDemoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Stage::Startup, load_models)
            .add_systems(Stage::Update, report_models);
    }
}

#[derive(Resource)]
struct Models {
    gltf: Vec<(&'static str, Handle<Gltf>)>,
    fox_mesh: Handle<Mesh>,
    missing: Handle<Image>,
    all_ready_logged: bool,
}

const MODELS: &[&str] = &[
    "models/khronos/Box/Box.glb",
    "models/khronos/BoxTextured/BoxTextured.glb",
    "models/khronos/DamagedHelmet/DamagedHelmet.glb",
    "models/khronos/CesiumMan/CesiumMan.glb",
    "models/khronos/Fox/Fox.glb",
    // Не в git (лицензии): есть, только если скачаны локально — иначе ошибка «не найден»
    "models/khronos/Sponza/Sponza.gltf",
    "models/orc/orc.glb",
    // По сети: кеш, повторы, дедупликация
    "https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models/Duck/glTF-Binary/Duck.glb",
];

fn load_models(server: Res<AssetServer>, mut commands: Commands) {
    let gltf = MODELS
        .iter()
        .map(|path| (*path, server.load(path)))
        .collect();
    commands.insert_resource(Models {
        gltf,
        // Вложенный ресурс по метке: файл загрузится один раз для обоих запросов
        fox_mesh: server.load("models/khronos/Fox/Fox.glb#Mesh0"),
        missing: server.load("textures/does_not_exist.png"),
        all_ready_logged: false,
    });
}

fn report_models(
    time: Res<Time>,
    server: Res<AssetServer>,
    mut models: ResMut<Models>,
    gltfs: Res<Assets<Gltf>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    mut events: MessageReader<AssetEvent<Gltf>>,
) {
    for event in events.read() {
        let AssetEvent::Loaded(id) = event else {
            continue;
        };
        let Some((path, handle)) = models.gltf.iter().find(|(_, h)| h.id() == *id) else {
            continue;
        };
        let Some(gltf) = gltfs.get_loaded(handle) else {
            continue;
        };
        let (vertices, triangles) = gltf
            .meshes
            .iter()
            .filter_map(|mesh| meshes.get_loaded(mesh))
            .fold((0, 0), |(v, t), mesh| {
                (v + mesh.vertex_count(), t + mesh.triangle_count())
            });
        let name = path.rsplit('/').next().unwrap_or(path);
        engine::log::info!(
            "{name}: {:.2} с, мешей {}, вершин {vertices}, треугольников {triangles}, материалов {}, текстур {}, костей {}, анимаций {:?}",
            time.elapsed().as_secs_f32(),
            gltf.meshes.len(),
            gltf.materials.len(),
            gltf.images.len(),
            gltf.skins.iter().map(|s| s.joints.len()).sum::<usize>(),
            gltf.animations,
        );
    }

    if models.all_ready_logged {
        return;
    }
    // Ждём, пока всё завершится (успешно или с ошибкой), включая внешние текстуры
    let pending = models.gltf.iter().any(|(_, h)| {
        matches!(
            server.is_loaded_with_dependencies(h.untyped_id()),
            Ok(false)
        )
    });
    if pending {
        return;
    }
    models.all_ready_logged = true;
    let (loading, ready, failed) = server.counts();
    engine::log::info!(
        "Ресурсы: загружено {ready}, ошибок {failed}, в процессе {loading} — за {:.2} с",
        time.elapsed().as_secs_f32()
    );
    let fox = meshes.get_loaded(&models.fox_mesh).map(Mesh::vertex_count);
    engine::log::info!("Fox.glb#Mesh0: вершин {fox:?}");
    if let Some(resolved) = images.get(&models.missing) {
        engine::log::info!(
            "Несуществующая текстура: заглушка {}x{}, статус {:?}",
            resolved.asset.width,
            resolved.asset.height,
            resolved.status
        );
    }
}

/// Точка входа на Android. Её по имени ищет `android-activity` после загрузки .so,
/// функции `main` на Android нет.
#[cfg(target_os = "android")]
// no_mangle небезопасен: два символа с одним именем — UB при линковке.
// ABI именно Rust, а не C: android-activity объявляет функцию как `extern "Rust"`.
#[allow(unsafe_code, clippy::no_mangle_with_rust_abi)]
#[unsafe(no_mangle)]
fn android_main(app: engine::platform::AndroidApp) {
    if let Err(error) = run(LaunchContext::android(app)) {
        engine::log::error!("Приложение завершилось с ошибкой: {error}");
    }
}

/// Точка входа на Web. `wasm-bindgen` вызывает её сразу после загрузки модуля.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn web_start() {
    if let Err(error) = run(LaunchContext::default()) {
        engine::log::error!("Приложение завершилось с ошибкой: {error}");
    }
}
