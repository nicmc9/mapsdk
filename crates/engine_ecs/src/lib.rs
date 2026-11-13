//! ECS движка. Реализация — `bevy_ecs`, но проект импортирует только `engine::ecs`.
//!
//! Наружу выставлен ограниченный набор типов: при замене ECS нужно будет
//! переписать этот модуль, а не весь код проекта.

pub mod prelude {
    //! Всё необходимое для систем и компонентов: `use engine::ecs::prelude::*;`

    pub use bevy_ecs::prelude::{
        // Фильтры запросов
        Added,
        // Сущности, компоненты, ресурсы
        Bundle,
        Changed,
        // Иерархия
        ChildOf,
        Children,
        // Системы и их параметры
        Commands,
        Component,
        Entity,
        // Порядок выполнения систем
        IntoScheduleConfigs,
        Local,
        // Сообщения между системами (двойная буферизация, живут 2 кадра)
        Message,
        MessageReader,
        MessageWriter,
        Or,
        Query,
        Res,
        ResMut,
        Resource,
        Single,
        SystemSet,
        With,
        Without,
        World,
    };

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
