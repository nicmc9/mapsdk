//! Проигрыватель анимаций (R2-05, R2-07): клипы glTF двигают узлы-кости сцены.
//!
//! ```ignore
//! player.crossfade(gltf.animations[1].clone(), 0.3); // плавный переход за 0.3 с
//! ```
//!
//! Время идёт на фиксированном шаге логики — анимация детерминирована, как физика.

use engine_assets::formats::{AnimationClip, Sample};
use engine_assets::{Assets, Handle};
use engine_core::FixedTime;
use engine_ecs::prelude::*;

use crate::Transform;

/// Сущности узлов развёрнутой сцены glTF: индекс узла → сущность.
/// Добавляется на корень `SceneRoot` при развёртывании.
#[derive(Component, Debug, Clone)]
pub struct SceneNodes(pub Vec<Option<Entity>>);

#[derive(Debug, Clone)]
struct Playing {
    clip: Handle<AnimationClip>,
    time: f32,
}

/// Проигрыватель на корне сцены. Требует `SceneNodes` (появляется после развёртывания).
#[derive(Component, Debug, Clone)]
pub struct AnimationPlayer {
    current: Option<Playing>,
    /// Предыдущий клип во время перехода: доигрывает с убывающим весом.
    fading: Option<(Playing, f32)>,
    fade_duration: f32,
    fade_elapsed: f32,
    pub speed: f32,
    pub paused: bool,
    pub looping: bool,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            current: None,
            fading: None,
            fade_duration: 0.0,
            fade_elapsed: 0.0,
            speed: 1.0,
            paused: false,
            looping: true,
        }
    }
}

impl AnimationPlayer {
    /// Сразу переключиться на клип.
    pub fn play(&mut self, clip: Handle<AnimationClip>) -> &mut Self {
        self.current = Some(Playing { clip, time: 0.0 });
        self.fading = None;
        self
    }

    /// Плавно перейти на клип за `duration` секунд.
    pub fn crossfade(&mut self, clip: Handle<AnimationClip>, duration: f32) -> &mut Self {
        if self.current.as_ref().is_some_and(|c| c.clip == clip) {
            return self;
        }
        self.fading = self.current.take().map(|playing| (playing, 1.0));
        self.current = Some(Playing { clip, time: 0.0 });
        self.fade_duration = duration.max(1e-3);
        self.fade_elapsed = 0.0;
        self
    }

    pub fn current_clip(&self) -> Option<&Handle<AnimationClip>> {
        self.current.as_ref().map(|c| &c.clip)
    }
}

pub(crate) fn animate(
    fixed: Res<FixedTime>,
    clips: Res<Assets<AnimationClip>>,
    mut players: Query<(&mut AnimationPlayer, &SceneNodes)>,
    mut transforms: Query<&mut Transform>,
) {
    let _zone = tracing::trace_span!("animate").entered();
    for (mut player, nodes) in &mut players {
        if player.paused {
            continue;
        }
        let dt = fixed.step_secs() * player.speed;
        let looping = player.looping;

        // Переход: вес нового клипа растёт от 0 до 1
        let mut weight = 1.0;
        if player.fading.is_some() {
            player.fade_elapsed += dt;
            weight = (player.fade_elapsed / player.fade_duration).min(1.0);
            if weight >= 1.0 {
                player.fading = None;
            }
        }
        // [Rust] `as_mut()` + `map` — меняем время внутри Option, не вынимая значение
        if let Some((fading, _)) = player.fading.as_mut()
            && let Some(clip) = clips.get_loaded(&fading.clip)
        {
            fading.time = advance(fading.time, dt, clip.duration, looping);
            apply(clip, fading.time, 1.0, nodes, &mut transforms);
        }
        if let Some(current) = player.current.as_mut()
            && let Some(clip) = clips.get_loaded(&current.clip)
        {
            current.time = advance(current.time, dt, clip.duration, looping);
            apply(clip, current.time, weight, nodes, &mut transforms);
        }
    }
}

fn advance(time: f32, dt: f32, duration: f32, looping: bool) -> f32 {
    let next = time + dt;
    if duration <= 0.0 {
        0.0
    } else if looping {
        next.rem_euclid(duration)
    } else {
        next.min(duration)
    }
}

/// Записывает значения каналов в трансформы узлов. `weight < 1` — смешивание
/// с текущим значением (его уже записал предыдущий клип).
fn apply(
    clip: &AnimationClip,
    time: f32,
    weight: f32,
    nodes: &SceneNodes,
    transforms: &mut Query<&mut Transform>,
) {
    for channel in &clip.channels {
        let Some(Some(entity)) = nodes.0.get(channel.node) else {
            continue;
        };
        let (Ok(mut transform), Some(sample)) = (transforms.get_mut(*entity), channel.sample(time))
        else {
            continue;
        };
        match sample {
            Sample::Translation(v) => transform.translation = transform.translation.lerp(v, weight),
            Sample::Rotation(q) => transform.rotation = transform.rotation.slerp(q, weight),
            Sample::Scale(v) => transform.scale = transform.scale.lerp(v, weight),
        }
    }
}
