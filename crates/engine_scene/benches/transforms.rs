//! Мировые трансформы: 100 корней × 100 потомков = 10 100 сущностей.

use criterion::{Criterion, criterion_group, criterion_main};
use engine_core::{App, Stage};
use engine_ecs::prelude::*;
use engine_scene::{Transform, propagate_transforms};

fn propagate(c: &mut Criterion) {
    let mut app = App::new();
    app.add_systems(Stage::PostUpdate, propagate_transforms);
    #[allow(clippy::cast_precision_loss)]
    for r in 0..100 {
        let root = app
            .world_mut()
            .spawn(Transform::from_xyz(r as f32, 0.0, 0.0))
            .id();
        for c in 0..100 {
            app.world_mut()
                .spawn((Transform::from_xyz(0.0, c as f32, 0.0), ChildOf(root)));
        }
    }
    app.update();
    c.bench_function(
        "трансформы 10k сущностей в иерархии",
        |b| b.iter(|| app.update()),
    );
}

criterion_group!(benches, propagate);
criterion_main!(benches);
