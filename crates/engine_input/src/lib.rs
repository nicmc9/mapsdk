//! Состояние ввода за кадр. Системы читают `Res<Input>`.
//!
//! ```ignore
//! fn camera(input: Res<Input>) {
//!     if input.button_down(MouseButton::Left) { let delta = input.cursor_delta(); }
//!     if input.key_pressed(Key::Space) { /* нажата в этом кадре */ }
//! }
//! ```
//!
//! Маппинг действий («прыжок» → Space или кнопка геймпада) и геймпады — задача P-03, позже.

use std::collections::{HashMap, HashSet};

use engine_core::{App, PlatformInput, Plugin, Stage};
use engine_ecs::prelude::*;
use engine_math::Vec2;

use engine_platform::InputEvent;
pub use engine_platform::{Key, MouseButton, TouchPhase};

#[derive(Debug, Default)]
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Input>()
            .add_systems(Stage::PreUpdate, update_input);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Touch {
    pub position: Vec2,
    pub previous: Vec2,
}

#[derive(Resource, Debug, Default)]
pub struct Input {
    keys_down: HashSet<Key>,
    keys_pressed: HashSet<Key>,
    keys_released: HashSet<Key>,
    buttons_down: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
    cursor: Option<Vec2>,
    cursor_delta: Vec2,
    scroll: f32,
    trackpad_scroll: Vec2,
    trackpad_pinch: f32,
    touches: HashMap<u64, Touch>,
}

impl Input {
    /// Клавиша удерживается.
    pub fn key_down(&self, key: Key) -> bool {
        self.keys_down.contains(&key)
    }
    /// Клавиша нажата в этом кадре.
    pub fn key_pressed(&self, key: Key) -> bool {
        self.keys_pressed.contains(&key)
    }
    pub fn key_released(&self, key: Key) -> bool {
        self.keys_released.contains(&key)
    }
    pub fn button_down(&self, button: MouseButton) -> bool {
        self.buttons_down.contains(&button)
    }
    pub fn button_pressed(&self, button: MouseButton) -> bool {
        self.buttons_pressed.contains(&button)
    }
    /// Положение курсора в физических пикселях окна.
    pub fn cursor(&self) -> Option<Vec2> {
        self.cursor
    }
    /// Сдвиг курсора за кадр.
    pub fn cursor_delta(&self) -> Vec2 {
        self.cursor_delta
    }
    /// Колесо мыши за кадр (строки; от себя — положительная).
    pub fn scroll(&self) -> f32 {
        self.scroll
    }
    /// Прокрутка двумя пальцами по тачпаду за кадр (пиксели).
    pub fn trackpad_scroll(&self) -> Vec2 {
        self.trackpad_scroll
    }
    /// Щипок на тачпаде за кадр (изменение масштаба; раздвинуть пальцы — положительное).
    pub fn trackpad_pinch(&self) -> f32 {
        self.trackpad_pinch
    }
    pub fn touches(&self) -> impl Iterator<Item = &Touch> {
        self.touches.values()
    }
    pub fn touch_count(&self) -> usize {
        self.touches.len()
    }

    /// Перетаскивание одним пальцем: сдвиг за кадр.
    pub fn drag_delta(&self) -> Option<Vec2> {
        let mut touches = self.touches.values();
        match (touches.next(), touches.next()) {
            (Some(touch), None) => Some(touch.position - touch.previous),
            _ => None,
        }
    }

    /// «Щипок» двумя пальцами: изменение расстояния между ними за кадр (пиксели).
    pub fn pinch_delta(&self) -> Option<f32> {
        let mut touches = self.touches.values();
        let (Some(a), Some(b), None) = (touches.next(), touches.next(), touches.next()) else {
            return None;
        };
        Some(a.position.distance(b.position) - a.previous.distance(b.previous))
    }

    fn begin_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.buttons_pressed.clear();
        self.cursor_delta = Vec2::ZERO;
        self.scroll = 0.0;
        self.trackpad_scroll = Vec2::ZERO;
        self.trackpad_pinch = 0.0;
        for touch in self.touches.values_mut() {
            touch.previous = touch.position;
        }
    }

    fn apply(&mut self, event: InputEvent) {
        match event {
            InputEvent::Key { key, pressed: true } => {
                if self.keys_down.insert(key) {
                    self.keys_pressed.insert(key);
                }
            }
            InputEvent::Key {
                key,
                pressed: false,
            } => {
                self.keys_down.remove(&key);
                self.keys_released.insert(key);
            }
            InputEvent::MouseButton {
                button,
                pressed: true,
            } => {
                if self.buttons_down.insert(button) {
                    self.buttons_pressed.insert(button);
                }
            }
            InputEvent::MouseButton {
                button,
                pressed: false,
            } => {
                self.buttons_down.remove(&button);
            }
            InputEvent::CursorMoved { x, y } => {
                let position = Vec2::new(x, y);
                if let Some(previous) = self.cursor {
                    self.cursor_delta += position - previous;
                }
                self.cursor = Some(position);
            }
            InputEvent::MouseWheel { lines } => self.scroll += lines,
            InputEvent::TrackpadScroll { x, y } => self.trackpad_scroll += Vec2::new(x, y),
            InputEvent::TrackpadPinch { delta } => self.trackpad_pinch += delta,
            InputEvent::Char(_) => {} // текст читают текстовые поля (UI)
            InputEvent::Touch { id, phase, x, y } => {
                let position = Vec2::new(x, y);
                match phase {
                    TouchPhase::Started => {
                        self.touches.insert(
                            id,
                            Touch {
                                position,
                                previous: position,
                            },
                        );
                    }
                    TouchPhase::Moved => {
                        if let Some(touch) = self.touches.get_mut(&id) {
                            touch.position = position;
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.touches.remove(&id);
                    }
                }
            }
        }
    }
}

fn update_input(mut input: ResMut<Input>, mut events: MessageReader<PlatformInput>) {
    input.begin_frame();
    for PlatformInput(event) in events.read() {
        input.apply(*event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_only_in_first_frame_and_pinch() {
        let mut input = Input::default();
        input.apply(InputEvent::Key {
            key: Key::W,
            pressed: true,
        });
        assert!(input.key_pressed(Key::W) && input.key_down(Key::W));
        input.begin_frame();
        assert!(!input.key_pressed(Key::W) && input.key_down(Key::W));

        input.apply(InputEvent::Touch {
            id: 1,
            phase: TouchPhase::Started,
            x: 0.0,
            y: 0.0,
        });
        input.apply(InputEvent::Touch {
            id: 2,
            phase: TouchPhase::Started,
            x: 100.0,
            y: 0.0,
        });
        input.begin_frame();
        input.apply(InputEvent::Touch {
            id: 2,
            phase: TouchPhase::Moved,
            x: 150.0,
            y: 0.0,
        });
        assert_eq!(input.pinch_delta(), Some(50.0));
    }
}
