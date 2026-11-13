// PBR (metallic-roughness, как в glTF): BRDF Кука–Торранса с GGX.
// Группы привязок: 0 — кадр (камера, свет), 1 — материал, 2 — объект (смещение в буфере).

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
    // x — число точечных источников, y = 1 — кодировать sRGB вручную (Web)
    counts: vec4<u32>,
    points: array<PointLight, 8>,
};

struct Material {
    base_color: vec4<f32>,
    emissive: vec4<f32>,          // rgb — свечение, w — порог альфы (маска), < 0 — без маски
    // x — металличность, y — шероховатость, z — масштаб нормалей, w = 1 — есть карта нормалей
    params: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,   // обратная транспонированная: верна и при неравномерном масштабе
};

@group(0) @binding(0) var<uniform> frame: Frame;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_color_texture: texture_2d<f32>;
@group(1) @binding(2) var metallic_roughness_texture: texture_2d<f32>;
@group(1) @binding(3) var normal_texture: texture_2d<f32>;
@group(1) @binding(4) var occlusion_texture: texture_2d<f32>;
@group(1) @binding(5) var emissive_texture: texture_2d<f32>;
@group(1) @binding(6) var material_sampler: sampler;

@group(2) @binding(0) var<uniform> object: Object;

// Матрицы костей скелетного меша: мировая матрица кости × обратная матрица привязки.
// Uniform, а не storage: на тире GLES3 storage-буферов в вершинном шейдере может не быть.
struct Joints {
    matrices: array<mat4x4<f32>, 256>,
};
@group(3) @binding(0) var<uniform> joints: Joints;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let world = object.model * vec4<f32>(in.position, 1.0);
    var out: VertexOutput;
    out.clip_position = frame.view_proj * world;
    out.world_position = world.xyz;
    out.world_normal = (object.normal_matrix * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}

// Скиннинг: вершина — взвешенная сумма положений по четырём костям.
// Матрицы костей уже мировые, матрица модели не нужна.
@vertex
fn vs_skinned(
    in: VertexInput,
    @location(3) joint_indices: vec4<u32>,
    @location(4) joint_weights: vec4<f32>,
) -> VertexOutput {
    let skin = joints.matrices[joint_indices.x] * joint_weights.x
        + joints.matrices[joint_indices.y] * joint_weights.y
        + joints.matrices[joint_indices.z] * joint_weights.z
        + joints.matrices[joint_indices.w] * joint_weights.w;
    let world = skin * vec4<f32>(in.position, 1.0);
    var out: VertexOutput;
    out.clip_position = frame.view_proj * world;
    out.world_position = world.xyz;
    // Нормаль — той же матрицей (верно при равномерном масштабе костей)
    out.world_normal = (skin * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}

const PI: f32 = 3.14159265;

// Распределение микрограней GGX
fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Затенение микрограней (Смит, Шлик-GGX)
fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let gv = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let gl = n_dot_l / (n_dot_l * (1.0 - k) + k);
    return gv * gl;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Вклад одного источника света
fn shade(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>,
         albedo: vec3<f32>, metallic: f32, roughness: f32) -> vec3<f32> {
    let h = normalize(v + l);
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 1e-4);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
    let specular = distribution_ggx(max(dot(n, h), 0.0), roughness)
        * geometry_smith(n_dot_v, n_dot_l, roughness) * f
        / max(4.0 * n_dot_v * n_dot_l, 1e-4);
    let diffuse = (vec3<f32>(1.0) - f) * (1.0 - metallic) * albedo / PI;
    return (diffuse + specular) * radiance * n_dot_l;
}

// Нормаль из карты нормалей без касательных в вершинах: базис строится по производным
// позиции и UV в экранном пространстве (метод Christian Schüler, «Followup: Normal Mapping
// Without Precomputed Tangents»).
fn perturb_normal(n: vec3<f32>, position: vec3<f32>, uv: vec2<f32>, mapped: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(position);
    let dp2 = dpdy(position);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let inv_max = inverseSqrt(max(dot(t, t), dot(b, b)) + 1e-12);
    return normalize(mat3x3<f32>(t * inv_max, b * inv_max, n) * mapped);
}

// Тональная компрессия ACES (аппроксимация Narkowicz): HDR → 0..1
fn tonemap_aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Все выборки и производные — в начале, до ветвлений: WGSL требует для них
    // «однородного» потока управления
    let base_sample = textureSample(base_color_texture, material_sampler, in.uv);
    let mr_sample = textureSample(metallic_roughness_texture, material_sampler, in.uv);
    let normal_sample = textureSample(normal_texture, material_sampler, in.uv).xyz;
    let occlusion = textureSample(occlusion_texture, material_sampler, in.uv).r;
    let emissive_sample = textureSample(emissive_texture, material_sampler, in.uv).rgb;

    var n = normalize(in.world_normal);
    if (!front) {
        n = -n; // двусторонний материал: обратная сторона
    }
    let mapped = normalize(vec3<f32>((normal_sample.xy * 2.0 - 1.0) * material.params.z,
                                     normal_sample.z * 2.0 - 1.0));
    let perturbed = perturb_normal(n, in.world_position, in.uv, mapped);
    n = select(n, perturbed, material.params.w > 0.5);

    let base = material.base_color * base_sample;
    if (material.emissive.w >= 0.0 && base.a < material.emissive.w) {
        discard; // альфа-маска (листья, решётки)
    }
    let albedo = base.rgb;
    // В glTF: G — шероховатость, B — металличность
    let roughness = clamp(material.params.y * mr_sample.g, 0.04, 1.0);
    let metallic = clamp(material.params.x * mr_sample.b, 0.0, 1.0);
    let v = normalize(frame.camera_position.xyz - in.world_position);

    var color = vec3<f32>(0.0);
    if (frame.sun_direction.w > 0.5) {
        color += shade(n, v, -normalize(frame.sun_direction.xyz), frame.sun_color.rgb,
                       albedo, metallic, roughness);
    }
    for (var i = 0u; i < min(frame.counts.x, 8u); i++) {
        let light = frame.points[i];
        let to_light = light.position_range.xyz - in.world_position;
        let distance = length(to_light);
        // Плавное затухание до нуля на границе радиуса
        let ratio = distance / max(light.position_range.w, 1e-4);
        let window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
        let attenuation = window * window / (distance * distance + 1.0);
        color += shade(n, v, to_light / max(distance, 1e-4), light.color.rgb * attenuation,
                       albedo, metallic, roughness);
    }
    color += frame.ambient.rgb * albedo * occlusion;
    color += material.emissive.rgb * emissive_sample;

    var out_color = tonemap_aces(color);
    if (frame.counts.y == 1u) {
        out_color = linear_to_srgb(out_color);
    }
    return vec4<f32>(out_color, base.a);
}
