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
    pub fn wants_pointer(&self) -> bool {
        self.ctx.egui_wants_pointer_input() || self.ctx.is_pointer_over_egui()
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
        InputEvent::MouseWheel { delta } => out.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, delta),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: state.modifiers,
        }),
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
