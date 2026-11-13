//! Тестовое приложение: 3D-сцена из моделей glTF, орбитальная камера, статистика.
//!
//! Сцена выбирается CVar `sandbox.scene`: `showcase` (по умолчанию), `sponza`, `helmet`.
//! Скриншот: `--cvar render.screenshot=shot.png --cvar render.screenshot_exit=true`.

mod camera;
mod scenes;

use engine::platform::{LaunchContext, PlatformConfig, PlatformError};
use engine::prelude::*;

/// Общий запуск для всех платформ.
pub fn run(launch: LaunchContext) -> Result<(), PlatformError> {
    let config = PlatformConfig {
        title: "Sandbox".to_owned(),
        ..PlatformConfig::default() // [Rust] остальные поля — из значения по умолчанию
    };
    App::new()
        // Файл встроен в бинарник: так он доступен на всех платформах
        .load_config_toml(include_str!("../sandbox.toml"))
        .add_plugin(NetPlugin)
        .add_plugin(AssetPlugin)
        .add_plugin(ScenePlugin)
        .add_plugin(InputPlugin)
        .add_plugin(RenderPlugin)
        .add_plugin(DebugPlugin)
        .add_plugin(SandboxPlugin)
        .run(config, launch)
}

struct SandboxPlugin;

impl Plugin for SandboxPlugin {
    fn build(&self, app: &mut App) {
        app.register_cvar(&scenes::SCENE)
            .insert_resource(ClearColor(Color::rgb(0.05, 0.06, 0.08)))
            .init_resource::<FrameStats>()
            .add_systems(Stage::Startup, scenes::setup)
            .add_systems(
                Stage::Update,
                (
                    scenes::fit_models,
                    scenes::autoplay,
                    scenes::next_clip,
                    camera::orbit_camera,
                    report_stats,
                ),
            );
    }
}

#[derive(Resource, Default)]
struct FrameStats {
    frames: u32,
    last_report_rendered: u64,
    next_report_secs: f32,
}

fn report_stats(
    time: Res<Time>,
    server: Res<AssetServer>,
    render: Res<RenderStats>,
    net: Res<NetStats>,
    mut stats: ResMut<FrameStats>,
) {
    stats.frames += 1;
    let now = time.elapsed().as_secs_f32();
    if now < stats.next_report_secs {
        return;
    }
    let (loading, ready, failed) = server.counts();
    engine::log::info!(
        "кадры: логика {} ({:.2} мс), рендер {} ({:.2} мс) | примитивов: нарисовано {}, отсечено {} | GPU: мешей {}, текстур {} | ресурсы: {ready} готово, {loading} грузится, {failed} ошибок | сеть: {} КБ",
        stats.frames,
        time.cpu_time().as_secs_f64() * 1000.0,
        render.frames_rendered - stats.last_report_rendered,
        render.frame_cpu_time.as_secs_f64() * 1000.0,
        render.drawn,
        render.culled,
        render.gpu_meshes,
        render.gpu_textures,
        net.bytes_downloaded / 1024,
    );
    stats.frames = 0;
    stats.last_report_rendered = render.frames_rendered;
    stats.next_report_secs = now + 1.0;
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
