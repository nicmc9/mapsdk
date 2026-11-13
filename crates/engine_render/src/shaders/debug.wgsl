// Отладочные линии: позиция в мире и цвет. Группа 0 — та же, что у PBR (камера).

struct PointLight {
    position_range: vec4<f32>,
    color: vec4<f32>,
};

struct Frame {
    view_proj: mat4x4<f32>,
    camera_position: vec4<f32>,
    ambient: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    counts: vec4<u32>,
    points: array<PointLight, 8>,
};

@group(0) @binding(0) var<uniform> frame: Frame;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = frame.view_proj * vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color.rgb;
    if (frame.counts.y == 1u) {
        color = linear_to_srgb(color);
    }
    return vec4<f32>(color, in.color.a);
}
