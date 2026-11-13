//! Тестовое приложение. Здесь же — точки входа для платформ, где их задаёт ОС.

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
