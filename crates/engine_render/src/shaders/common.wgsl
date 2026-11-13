// Общее для всех шейдеров движка: данные кадра (камера, свет) и кодирование sRGB.
// Подключается через `#include "common.wgsl"`.

struct PointLight {
    position_range: vec4<f32>,   // xyz — позиция, w — радиус действия
    color: vec4<f32>,            // rgb — цвет × интенсивность
};

struct Frame {
    view_proj: mat4x4<f32>,
    camera_position: vec4<f32>,
    ambient: vec4<f32>,           // rgb — цвет × интенсивность
    sun_direction: vec4<f32>,     // xyz — куда светит солнце, w = 1 — солнце есть
    sun_color: vec4<f32>,
    // x — число точечных источников, y = 1 — кодировать sRGB вручную (Web, GLES)
    counts: vec4<u32>,
    points: array<PointLight, 8>,
};

@group(0) @binding(0) var<uniform> frame: Frame;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}
