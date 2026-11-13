//! Тестовое приложение. Здесь же — точки входа для платформ, где их задаёт ОС.

use engine::platform::{LaunchContext, PlatformConfig, PlatformError};
use engine::prelude::*;

/// Общий запуск для всех платформ.
pub fn run(launch: LaunchContext) -> Result<(), PlatformError> {
    let config = PlatformConfig {
        title: "Sandbox".to_owned(),
        ..PlatformConfig::default() // [Rust] остальные поля — из значения по умолчанию
    };
    App::new().add_plugin(SandboxPlugin).run(config, launch)
}

/// Демонстрация ядра: объекты вращаются на фиксированном шаге,
/// раз в секунду в лог пишется статистика кадров.
struct SandboxPlugin;

impl Plugin for SandboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameStats>()
            .add_systems(Stage::Startup, spawn_objects)
            .add_systems(Stage::FixedUpdate, rotate)
            .add_systems(Stage::Update, (report_stats, log_lifecycle));
    }
}

#[derive(Component, Debug)]
struct Spin {
    angle: f32,
    /// Радиан в секунду
    speed: f32,
}

#[derive(Resource, Default)]
struct FrameStats {
    frames: u32,
    last_report_steps: u64,
    next_report_secs: f32,
}

// [Rust] Система — обычная функция. Её параметры (`Commands`, `Query`, `Res`) bevy_ecs
// заполняет сам, а по их типам понимает, какие системы можно выполнять параллельно.
fn spawn_objects(mut commands: Commands) {
    for i in 0..3u8 {
        commands.spawn(Spin {
            angle: 0.0,
            speed: f32::from(i + 1),
        });
    }
}

fn rotate(fixed: Res<FixedTime>, mut query: Query<&mut Spin>) {
    let dt = fixed.step_secs();
    // [Rust] `&mut Spin` в Query — изменяемый доступ к компоненту каждой сущности
    for mut spin in &mut query {
        spin.angle = (spin.angle + spin.speed * dt) % std::f32::consts::TAU;
    }
}

fn report_stats(
    time: Res<Time>,
    fixed: Res<FixedTime>,
    window: Res<Window>,
    mut stats: ResMut<FrameStats>,
    query: Query<&Spin>,
) {
    stats.frames += 1;
    let now = time.elapsed().as_secs_f32();
    if now < stats.next_report_secs {
        return;
    }
    let angles: Vec<f32> = query.iter().map(|spin| spin.angle).collect();
    log::info!(
        "кадров: {}, шагов логики: {}, окно: {}x{}, углы: {angles:.2?}",
        stats.frames,
        fixed.steps_total() - stats.last_report_steps,
        window.size.width,
        window.size.height,
    );
    stats.frames = 0;
    stats.last_report_steps = fixed.steps_total();
    stats.next_report_secs = now + 1.0;
}

fn log_lifecycle(mut events: MessageReader<AppLifecycle>) {
    for event in events.read() {
        log::info!("Событие приложения: {event:?}");
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
        log::error!("Приложение завершилось с ошибкой: {error}");
    }
}

/// Точка входа на Web. `wasm-bindgen` вызывает её сразу после загрузки модуля.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn web_start() {
    if let Err(error) = run(LaunchContext::default()) {
        log::error!("Приложение завершилось с ошибкой: {error}");
    }
}
