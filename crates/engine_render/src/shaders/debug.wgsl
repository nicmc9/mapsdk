// Отладочные линии: позиция в мире и цвет. Группа 0 — та же, что у PBR (камера).

#include "common.wgsl"

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


@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color.rgb;
    if (frame.counts.y == 1u) {
        color = linear_to_srgb(color);
    }
    return vec4<f32>(color, in.color.a);
}
