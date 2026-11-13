use engine_ecs::prelude::*;

use crate::{GlobalTransform, Transform, Visibility};

/// Мировые матрицы: от корней иерархии к листьям. Скрытый родитель скрывает потомков.
pub(crate) fn propagate_transforms(
    mut roots: Query<
        (Entity, &Transform, &mut GlobalTransform, Option<&Children>),
        Without<ChildOf>,
    >,
    mut nodes: Query<(&Transform, &mut GlobalTransform, Option<&Children>), With<ChildOf>>,
) {
    let _zone = tracing::trace_span!("propagate_transforms").entered();
    for (_, transform, mut global, children) in &mut roots {
        let matrix = transform.to_matrix();
        global.0 = matrix;
        if let Some(children) = children {
            for &child in children {
                propagate_recursive(child, &matrix, &mut nodes);
            }
        }
    }
}

// [Rust] Здесь Query передаётся по `&mut` в рекурсию. Borrow checker разрешает это, потому что
// в каждый момент времени изменяется ровно одна сущность — компоненты не пересекаются.
fn propagate_recursive(
    entity: Entity,
    parent: &engine_math::Mat4,
    nodes: &mut Query<(&Transform, &mut GlobalTransform, Option<&Children>), With<ChildOf>>,
) {
    let (matrix, children) = {
        let Ok((transform, mut global, children)) = nodes.get_mut(entity) else {
            return;
        };
        let matrix = *parent * transform.to_matrix();
        global.0 = matrix;
        // Копируем список детей, чтобы отпустить заимствование nodes перед рекурсией
        (matrix, children.map(|c| c.to_vec()))
    };
    for child in children.into_iter().flatten() {
        propagate_recursive(child, &matrix, nodes);
    }
}

/// Видимость с учётом предков — для рендера.
pub fn is_visible(entity: Entity, visibility: &Query<(&Visibility, Option<&ChildOf>)>) -> bool {
    let mut current = Some(entity);
    while let Some(entity) = current {
        let Ok((visible, parent)) = visibility.get(entity) else {
            return true;
        };
        if !visible.0 {
            return false;
        }
        current = parent.map(ChildOf::parent);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::{App, Stage};
    use engine_math::Vec3;

    #[test]
    fn children_follow_parents() {
        let mut app = App::new();
        app.add_systems(Stage::PostUpdate, propagate_transforms);
        let parent = app
            .world_mut()
            .spawn(Transform::from_xyz(10.0, 0.0, 0.0))
            .id();
        let child = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 5.0, 0.0), ChildOf(parent)))
            .id();
        let grandchild = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 0.0, 1.0), ChildOf(child)))
            .id();
        app.update();
        let world = app.world();
        let position = |e| world.get::<GlobalTransform>(e).unwrap().translation();
        assert_eq!(position(child), Vec3::new(10.0, 5.0, 0.0));
        assert_eq!(position(grandchild), Vec3::new(10.0, 5.0, 1.0));
    }
}
