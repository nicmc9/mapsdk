//! Орбитальная камера: мышь (левая кнопка — вращение, колесо — зум),
//! касания (палец — вращение, щипок — зум), клавиши Q/E — зум.

use engine::math::{Quat, Vec3};
use engine::prelude::*;

#[derive(Component)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl OrbitCamera {
    pub fn new(target: Vec3, distance: f32) -> Self {
        Self {
            target,
            distance,
            yaw: 0.35,
            pitch: -0.25,
        }
    }
}

pub fn orbit_camera(
    input: Res<Input>,
    ui: Res<Ui>,
    time: Res<Time>,
    mut cameras: Query<(&mut OrbitCamera, &mut Transform)>,
) {
    const ROTATE: f32 = 0.006;
    // Мышь над отладочным интерфейсом — камеру не трогаем
    let pointer_free = !ui.wants_pointer();
    for (mut orbit, mut transform) in &mut cameras {
        let mut rotate = engine::math::Vec2::ZERO;
        if pointer_free && input.button_down(MouseButton::Left) {
            rotate += input.cursor_delta();
        }
        if let Some(drag) = input.drag_delta().filter(|_| pointer_free) {
            rotate += drag;
        }
        orbit.yaw -= rotate.x * ROTATE;
        orbit.pitch = (orbit.pitch - rotate.y * ROTATE).clamp(-1.5, 1.5);

        let mut zoom = if pointer_free {
            input.scroll() * 0.1
        } else {
            0.0
        };
        if let Some(pinch) = input.pinch_delta() {
            zoom += pinch * 0.005;
        }
        if input.key_down(Key::Q) {
            zoom += time.delta_secs();
        }
        if input.key_down(Key::E) {
            zoom -= time.delta_secs();
        }
        orbit.distance = (orbit.distance * (1.0 - zoom)).clamp(0.3, 200.0);

        let rotation = Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch);
        transform.translation = orbit.target + rotation * Vec3::new(0.0, 0.0, orbit.distance);
        transform.rotation = rotation;
    }
}
