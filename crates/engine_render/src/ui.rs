//! Интерфейс на egui: строится в главном потоке, рисуется рендером поверх сцены.
//!
//! ```ignore
//! fn my_window(ui: Res<Ui>) {
//!     egui::Window::new("Окно").show(ui.ctx(), |ui| { ui.label("Привет"); });
//! }
//! ```
//!
//! - Ввод — из событий платформы (`PlatformInput`), без `egui-winit`.
//! - Геометрия интерфейса — состояние кадра: идёт в снимке через тройной буфер.
//! - Изменения текстур egui (атлас шрифта) терять нельзя — идут командами через очередь.
//!
//! [Rust] egui — интерфейс «немедленного режима»: окно описывается каждый кадр заново
//! вызовами функций, без сохранённого дерева виджетов (как Dear ImGui в C++).

use engine_core::{CVar, PlatformInput, Time, Window};
use engine_ecs::prelude::*;
use engine_platform::{InputEvent, Key, MouseButton, TouchPhase};

use crate::command::RenderCommand;
use crate::frontend::RenderFrontend;

pub static DEBUG_UI: CVar<bool> = CVar::new("debug.ui", false, "Показывать отладочный интерфейс");

/// Контекст egui. Доступен системам как `Res<Ui>`; рисовать — в стадии `Update`.
#[derive(Resource, Clone, Default)]
pub struct Ui {
    ctx: egui::Context,
}

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ui")
    }
}

impl Ui {
    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// Указатель над интерфейсом — камере и игре стоит игнорировать мышь.
    ///
    /// Не `egui_wants_pointer_input`: в egui 0.37 он опирается на границы корневого UI,
    /// которые заполняет только `run_ui`. Мы используем `begin_pass`/`end_pass`, и без этих
    /// границ egui считает весь экран своим (фоновый слой) — камера переставала реагировать.
    pub fn wants_pointer(&self) -> bool {
        let over_window = self
            .ctx
            .input(|i| i.pointer.interact_pos())
            .and_then(|position| self.ctx.layer_id_at(position))
            .is_some_and(|layer| layer.order != egui::Order::Background);
        self.ctx.egui_is_using_pointer() || over_window
    }

    /// Интерфейс ждёт клавиатуру (фокус в текстовом поле).
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }
}

/// Геометрия интерфейса для кадра рендера.
#[derive(Debug, Clone, Default)]
pub(crate) struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub pixels_per_point: f32,
}

/// Состояние ввода между кадрами.
#[derive(Default)]
pub(crate) struct UiInputState {
    pointer: egui::Pos2,
    modifiers: egui::Modifiers,
}

/// Начало прохода egui: ввод за кадр → `begin_pass`. Стадия `PreUpdate`.
pub(crate) fn ui_begin(
    ui: Res<Ui>,
    window: Res<Window>,
    time: Res<Time>,
    mut events: MessageReader<PlatformInput>,
    mut state: Local<UiInputState>,
) {
    #[allow(clippy::cast_possible_truncation)]
    let pixels_per_point = window.scale_factor.max(0.5) as f32;
    #[allow(clippy::cast_precision_loss)]
    let size = egui::vec2(window.size.width as f32, window.size.height as f32) / pixels_per_point;
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        time: Some(time.elapsed().as_secs_f64()),
        ..Default::default()
    };
    if let Some(viewport) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
        viewport.native_pixels_per_point = Some(pixels_per_point);
    }
    for PlatformInput(event) in events.read() {
        translate(*event, pixels_per_point, &mut state, &mut raw.events);
    }
    ui.ctx.begin_pass(raw);
}

fn translate(event: InputEvent, ppp: f32, state: &mut UiInputState, out: &mut Vec<egui::Event>) {
    let point = |x: f32, y: f32| egui::pos2(x / ppp, y / ppp);
    match event {
        InputEvent::CursorMoved { x, y } => {
            state.pointer = point(x, y);
            out.push(egui::Event::PointerMoved(state.pointer));
        }
        InputEvent::MouseButton { button, pressed } => {
            let button = match button {
                MouseButton::Left => egui::PointerButton::Primary,
                MouseButton::Right => egui::PointerButton::Secondary,
                MouseButton::Middle => egui::PointerButton::Middle,
                MouseButton::Other(_) => return,
            };
            out.push(egui::Event::PointerButton {
                pos: state.pointer,
                button,
                pressed,
                modifiers: state.modifiers,
            });
        }
        InputEvent::MouseWheel { lines } => out.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: state.modifiers,
        }),
        // Тачпад прокручивает списки интерфейса в пикселях (точках egui)
        InputEvent::TrackpadScroll { x, y } => out.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(x, y) / ppp,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: state.modifiers,
        }),
        InputEvent::TrackpadPinch { delta } => out.push(egui::Event::Zoom(1.0 + delta)),
        // Касание — как мышь одним пальцем
        InputEvent::Touch { phase, x, y, .. } => {
            state.pointer = point(x, y);
            out.push(egui::Event::PointerMoved(state.pointer));
            let pressed = match phase {
                TouchPhase::Started => true,
                TouchPhase::Ended | TouchPhase::Cancelled => false,
                TouchPhase::Moved => return,
            };
            out.push(egui::Event::PointerButton {
                pos: state.pointer,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: state.modifiers,
            });
            if !pressed {
                out.push(egui::Event::PointerGone);
            }
        }
        InputEvent::Char(c) => out.push(egui::Event::Text(c.to_string())),
        InputEvent::Key { key, pressed } => {
            match key {
                Key::ShiftLeft | Key::ShiftRight => state.modifiers.shift = pressed,
                Key::ControlLeft | Key::ControlRight => {
                    state.modifiers.ctrl = pressed;
                    state.modifiers.command = pressed;
                }
                Key::AltLeft | Key::AltRight => state.modifiers.alt = pressed,
                _ => {}
            }
            let key = match key {
                Key::Enter => egui::Key::Enter,
                Key::Escape => egui::Key::Escape,
                Key::Tab => egui::Key::Tab,
                Key::Backspace => egui::Key::Backspace,
                Key::Left => egui::Key::ArrowLeft,
                Key::Right => egui::Key::ArrowRight,
                Key::Up => egui::Key::ArrowUp,
                Key::Down => egui::Key::ArrowDown,
                Key::A => egui::Key::A,
                Key::C => egui::Key::C,
                Key::V => egui::Key::V,
                Key::X => egui::Key::X,
                _ => return,
            };
            out.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: state.modifiers,
            });
        }
    }
}

/// Конец прохода egui: геометрия — в снимок кадра, текстуры — командой. Стадия `Extract`.
pub(crate) fn ui_end(ui: Res<Ui>, mut frontend: NonSendMut<RenderFrontend>) {
    let _zone = tracing::trace_span!("ui_end").entered();
    let output = ui.ctx.end_pass();
    if !output.textures_delta.set.is_empty() || !output.textures_delta.free.is_empty() {
        frontend.send(RenderCommand::UiTextures(Box::new(
            crate::command::UiTextures(output.textures_delta),
        )));
    }
    let snapshot = frontend.snapshots.write();
    snapshot.ui.pixels_per_point = output.pixels_per_point;
    snapshot.ui.primitives = if DEBUG_UI.get() || !output.shapes.is_empty() {
        ui.ctx.tessellate(output.shapes, output.pixels_per_point)
    } else {
        Vec::new()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Без открытых окон указатель не должен считаться «занятым интерфейсом» —
    /// иначе камера перестаёт реагировать на мышь и тачпад.
    #[test]
    fn pointer_is_free_without_windows() {
        let ui = Ui::default();
        for frame in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(egui::pos2(400.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(5.0, 3.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            ui.ctx.begin_pass(raw);
            let mut output = ui.ctx.end_pass();
            output.textures_delta.clear(); // в тесте текстуры применять некуда
            assert!(
                !ui.wants_pointer(),
                "кадр {frame}: интерфейс захватил указатель без окон"
            );
        }
        // А над окном — захватывает
        for _ in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events: vec![egui::Event::PointerMoved(egui::pos2(60.0, 60.0))],
                ..Default::default()
            };
            ui.ctx.begin_pass(raw);
            egui::Window::new("окно")
                .fixed_pos([20.0, 20.0])
                .fixed_size([200.0, 200.0])
                .show(&ui.ctx, |ui| ui.label("текст"));
            let mut output = ui.ctx.end_pass();
            output.textures_delta.clear();
        }
        assert!(
            ui.wants_pointer(),
            "над окном указатель должен принадлежать интерфейсу"
        );
    }
}
