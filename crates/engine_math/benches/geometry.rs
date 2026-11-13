//! Отсечение: 10 000 объектов — AABB в мир и проверка по фрустуму, как в кадре рендера.

use criterion::{Criterion, criterion_group, criterion_main};
use engine_math::{Aabb, Frustum, Mat4, Quat, Vec3, camera};
use std::hint::black_box;

fn culling(c: &mut Criterion) {
    let frustum = Frustum::from_view_projection(
        &(camera::perspective(1.0, 16.0 / 9.0, 0.1, 500.0)
            * camera::look_at(Vec3::new(0.0, 5.0, 30.0), Vec3::ZERO, Vec3::Y)),
    );
    let local = Aabb {
        min: Vec3::splat(-0.5),
        max: Vec3::splat(0.5),
    };
    // Сетка 100×100 объектов с поворотом — половина вне поля зрения
    #[allow(clippy::cast_precision_loss)]
    let objects: Vec<Mat4> = (0..10_000)
        .map(|i| {
            let (x, z) = ((i % 100) as f32 - 50.0, (i / 100) as f32 - 50.0);
            Mat4::from_rotation_translation(
                Quat::from_rotation_y(i as f32),
                Vec3::new(x * 2.0, 0.0, z * 2.0),
            )
        })
        .collect();
    c.bench_function("отсечение 10k AABB", |b| {
        // [Rust] black_box не даёт оптимизатору выбросить «ненужные» вычисления
        b.iter(|| {
            objects
                .iter()
                .filter(|m| frustum.intersects_aabb(&local.transformed(black_box(m))))
                .count()
        });
    });
}

criterion_group!(benches, culling);
criterion_main!(benches);
