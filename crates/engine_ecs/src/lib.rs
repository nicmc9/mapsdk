//! ECS движка. Реализация — `bevy_ecs`, но проект импортирует только `engine::ecs`.
//!
//! Наружу выставлен ограниченный набор типов: при замене ECS нужно будет
//! переписать этот модуль, а не весь код проекта.

pub mod prelude {
    //! Всё необходимое для систем и компонентов: `use engine::ecs::prelude::*;`

    // Сущности, компоненты, ресурсы
    pub use bevy_ecs::prelude::{Bundle, Component, Entity, Resource, World};
    // Параметры систем
    pub use bevy_ecs::prelude::{Commands, Local, Query, Ref, Res, ResMut, Single};
    // [Rust] Методы трейта (`is_added`, `is_changed`) доступны, только если трейт импортирован
    pub use bevy_ecs::prelude::{DetectChanges, DetectChangesMut};
    // Ресурсы, доступные только из главного потока (объекты ОС, wgpu на Web)
    pub use bevy_ecs::prelude::{NonSend, NonSendMut};
    // Фильтры запросов
    pub use bevy_ecs::prelude::{Added, Changed, Or, With, Without};
    // Иерархия
    pub use bevy_ecs::prelude::{ChildOf, Children, Name};
    // Сообщения между системами (двойная буферизация, живут 2 кадра)
    pub use bevy_ecs::prelude::{Message, MessageReader, MessageWriter};
    // Порядок выполнения систем
    pub use bevy_ecs::prelude::{IntoScheduleConfigs, SystemSet};

    // [Rust] Обход ограничения derive-макросов bevy_ecs. `#[derive(Component)]` генерирует
    // код с путём `bevy_ecs::...` и ищет крейт `bevy_ecs` в Cargo.toml проекта. Проект
    // зависит только от `engine`, поэтому имя `bevy_ecs` приходит в область видимости
    // через этот реэкспорт вместе с `use engine::ecs::prelude::*`.
    #[doc(hidden)]
    pub use bevy_ecs;
}

pub use bevy_ecs::schedule::Schedules;
pub use bevy_ecs::schedule::{Schedule, ScheduleLabel};
pub use bevy_ecs::system::ScheduleSystem;

use bevy_ecs::message::{Message, MessageRegistry};
use bevy_ecs::world::World;

/// Регистрирует тип сообщения: создаёт его двойной буфер в мире.
pub fn register_message<M: Message>(world: &mut World) {
    MessageRegistry::register_message::<M>(world);
}

/// Система обмена буферов сообщений. Должна выполняться раз в кадр:
/// сообщение живёт два обмена, после чего удаляется.
pub use bevy_ecs::message::message_update_system;
