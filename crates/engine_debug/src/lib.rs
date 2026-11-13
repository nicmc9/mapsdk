//! Отладочный интерфейс (R-15).
//!
//! | Клавиша | Действие |
//! |---------|----------|
//! | `` ` `` или F4 | показать / скрыть интерфейс (CVar `debug.ui`) |
//! | F1 | сетка (`debug.grid`) |
//! | F2 | границы мешей (`debug.bounds`) |
//! | F3 | оси объектов (`debug.axes`) |
//!
//! Окна: «Статистика» (кадры, рендер, ресурсы, сеть) и «Переменные» (редактор CVar).

use std::collections::HashMap;

use engine_assets::AssetServer;
use engine_core::{App, CVarSource, CVars, Plugin, Stage, Time};
use engine_ecs::prelude::*;
use engine_input::{Input, Key};
use engine_net::NetStats;
use engine_render::egui;
use engine_render::{DEBUG_AXES, DEBUG_BOUNDS, DEBUG_GRID, DEBUG_UI, RenderStats, Ui};

/// Подключает отладочный интерфейс. Требует `RenderPlugin`, `InputPlugin`, `NetPlugin`.
#[derive(Debug, Default)]
pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameRates>().add_systems(
            Stage::Update,
            (hotkeys, measure_rates, stats_window, cvars_window),
        );
    }
}

/// Частоты кадров, усреднённые за секунду.
#[derive(Resource, Debug, Default)]
struct FrameRates {
    logic_fps: f32,
    render_fps: f32,
    frames: u32,
    rendered_at_start: u64,
    window_start: f32,
}

fn hotkeys(input: Res<Input>, ui: Res<Ui>) {
    // Пока в текстовом поле курсор — клавиши принадлежат ему
    if ui.wants_keyboard() {
        return;
    }
    for (key, cvar) in [
        (Key::Backquote, &DEBUG_UI),
        // На Mac с ISO-клавиатурой macOS путает коды клавиш ` и § — F4 как запасной вариант
        (Key::F4, &DEBUG_UI),
        (Key::F1, &DEBUG_GRID),
        (Key::F2, &DEBUG_BOUNDS),
        (Key::F3, &DEBUG_AXES),
    ] {
        if input.key_pressed(key) {
            cvar.set(!cvar.get());
        }
    }
}

fn measure_rates(time: Res<Time>, render: Res<RenderStats>, mut rates: ResMut<FrameRates>) {
    rates.frames += 1;
    let now = time.elapsed().as_secs_f32();
    let elapsed = now - rates.window_start;
    if elapsed >= 1.0 {
        #[allow(clippy::cast_precision_loss)]
        {
            rates.logic_fps = rates.frames as f32 / elapsed;
            rates.render_fps = (render.frames_rendered - rates.rendered_at_start) as f32 / elapsed;
        }
        rates.frames = 0;
        rates.rendered_at_start = render.frames_rendered;
        rates.window_start = now;
    }
}

fn stats_window(
    ui: Res<Ui>,
    time: Res<Time>,
    rates: Res<FrameRates>,
    render: Res<RenderStats>,
    net: Res<NetStats>,
    server: Res<AssetServer>,
) {
    if !DEBUG_UI.get() {
        return;
    }
    egui::Window::new("Статистика")
        .default_pos([12.0, 12.0])
        .default_width(300.0)
        .show(ui.ctx(), |ui| {
            egui::Grid::new("stats").striped(true).show(ui, |ui| {
                let mut row = |name: &str, value: String| {
                    ui.label(name);
                    ui.label(value);
                    ui.end_row();
                };
                row(
                    "Логика",
                    format!("{:.0} к/с, {:.2} мс", rates.logic_fps, ms(time.cpu_time())),
                );
                row(
                    "Рендер",
                    format!(
                        "{:.0} к/с, {:.2} мс",
                        rates.render_fps,
                        ms(render.frame_cpu_time)
                    ),
                );
                row(
                    "Примитивы",
                    format!("{} нарисовано, {} отсечено", render.drawn, render.culled),
                );
                row(
                    "Видеопамять",
                    format!(
                        "{} мешей, {} текстур",
                        render.gpu_meshes, render.gpu_textures
                    ),
                );
                row("Пропуски снимков", render.snapshots_skipped.to_string());
                let (loading, ready, failed) = server.counts();
                row(
                    "Ресурсы",
                    format!("{ready} готово, {loading} грузится, {failed} ошибок"),
                );
                row(
                    "Сеть",
                    format!(
                        "{} запросов, {} КБ, кеш {}, ошибок {}",
                        net.requests,
                        net.bytes_downloaded / 1024,
                        net.cache_hits,
                        net.errors
                    ),
                );
            });
            let failed = server.failed();
            if !failed.is_empty() {
                ui.separator();
                ui.collapsing(
                    format!("Ошибки загрузки ({})", failed.len()),
                    |ui| {
                        for (path, error) in failed {
                            ui.colored_label(egui::Color32::LIGHT_RED, path)
                                .on_hover_text(error.to_string());
                        }
                    },
                );
            }
            ui.separator();
            ui.label("` или F4 — интерфейс, F1 — сетка, F2 — границы, F3 — оси");
        });
}

fn ms(duration: engine_core::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Редактор CVar: значение применяется по Enter, как из консоли (с проверкой флагов).
fn cvars_window(
    ui: Res<Ui>,
    mut cvars: ResMut<CVars>,
    mut filter: Local<String>,
    // Текст полей ввода: пока поле редактируется, значение не перезаписывается текущим
    mut edits: Local<HashMap<&'static str, String>>,
    mut errors: Local<HashMap<&'static str, String>>,
) {
    if !DEBUG_UI.get() {
        return;
    }
    let mut apply = Vec::new();
    egui::Window::new("Переменные")
        .default_pos([12.0, 320.0])
        .default_width(420.0)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Поиск:");
                ui.text_edit_singleline(&mut *filter);
            });
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    egui::Grid::new("cvars")
                        .striped(true)
                        .num_columns(2)
                        .show(ui, |ui| {
                            for cvar in cvars.iter().filter(|c| c.name().contains(filter.as_str()))
                            {
                                let name = cvar.name();
                                ui.label(name).on_hover_text(format!(
                                    "{} ({})",
                                    cvar.description(),
                                    cvar.type_name()
                                ));
                                let text = edits.entry(name).or_insert_with(|| cvar.value_text());
                                let response = ui.text_edit_singleline(text);
                                if response.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    apply.push((name, text.clone()));
                                } else if !response.has_focus() {
                                    // Значение могли поменять другие системы — показываем актуальное
                                    *text = cvar.value_text();
                                }
                                ui.end_row();
                                if let Some(error) = errors.get(name) {
                                    ui.label("");
                                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                                    ui.end_row();
                                }
                            }
                        });
                });
        });
    for (name, text) in apply {
        match cvars.set(name, &text, CVarSource::Console) {
            Ok(()) => {
                errors.remove(name);
            }
            Err(error) => {
                errors.insert(name, error.to_string());
            }
        }
    }
}
