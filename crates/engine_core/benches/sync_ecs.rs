//! Тройной буфер (снимок кадра на 1000 объектов) и обход ECS (100 000 сущностей).

use criterion::{Criterion, criterion_group, criterion_main};
use engine_core::sync::triple_buffer::triple_buffer;
use engine_core::{App, Stage};
use engine_ecs::prelude::*;

#[derive(Clone, Default)]
struct Snapshot {
    items: Vec<[f32; 16]>,
}

fn snapshot_exchange(c: &mut Criterion) {
    let (mut producer, mut consumer) = triple_buffer(&Snapshot::default());
    c.bench_function(
        "снимок 1000 матриц: запись + publish + take",
        |b| {
            b.iter(|| {
                let slot = producer.write();
                slot.items.clear();
                slot.items.extend((0..1000).map(|_| [1.0; 16]));
                producer.publish();
                consumer.take();
                std::hint::black_box(consumer.current().items.len())
            });
        },
    );
}

#[derive(Component)]
struct Spin {
    angle: f32,
    speed: f32,
}

fn rotate(mut query: Query<&mut Spin>) {
    for mut spin in &mut query {
        spin.angle += spin.speed * 0.016;
    }
}

fn ecs_iteration(c: &mut Criterion) {
    let mut app = App::new();
    app.add_systems(Stage::Update, rotate);
    #[allow(clippy::cast_precision_loss)]
    for i in 0..100_000 {
        app.world_mut().spawn(Spin {
            angle: 0.0,
            speed: i as f32,
        });
    }
    app.update();
    c.bench_function(
        "ECS: кадр с обходом 100k сущностей",
        |b| b.iter(|| app.update()),
    );
}

criterion_group!(benches, snapshot_exchange, ecs_iteration);
criterion_main!(benches);
