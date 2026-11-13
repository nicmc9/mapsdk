use engine_ecs::prelude::*;

use crate::handle::short_type_name;
use crate::{Asset, AssetError, AssetId, AssetServer, Handle};

/// Состояние загрузки.
#[derive(Debug, Clone)]
pub enum AssetStatus {
    Loading,
    Ready,
    Failed(AssetError),
}

impl AssetStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

/// Результат обращения к ресурсу: сам ресурс или заглушка + статус.
#[derive(Debug)]
pub struct Resolved<'a, T> {
    pub asset: &'a T,
    pub status: AssetStatus,
}

impl<T> Resolved<'_, T> {
    /// Это настоящий ресурс, а не заглушка.
    pub fn is_ready(&self) -> bool {
        self.status.is_ready()
    }
}

/// События ресурсов типа `T`. Читаются через `MessageReader<AssetEvent<T>>`.
#[derive(Debug, Clone)]
pub enum AssetEvent<T: Asset> {
    Loaded(AssetId<T>),
    Failed(AssetId<T>, AssetError),
    Unloaded(AssetId<T>),
}

// [Rust] Реализация трейта вручную вместо derive: derive(Message) не умеет
// добавлять ограничение `T: Asset` для обобщённого типа.
impl<T: Asset> Message for AssetEvent<T> {}

struct Slot<T> {
    generation: u32,
    status: AssetStatus,
    value: Option<T>,
}

/// Хранилище ресурсов одного типа. Доступно системам как `Res<Assets<T>>`.
#[derive(Resource)]
pub struct Assets<T: Asset> {
    slots: Vec<Option<Slot<T>>>,
    loading_fallback: Option<T>,
    error_fallback: Option<T>,
    server: Option<AssetServer>,
}

impl<T: Asset> Assets<T> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            loading_fallback: T::loading_fallback(),
            error_fallback: T::error_fallback(),
            server: None,
        }
    }

    pub(crate) fn set_server(&mut self, server: AssetServer) {
        self.server = Some(server);
    }

    /// Ресурс или подходящая заглушка. `None` — у типа нет заглушки для текущего состояния.
    pub fn get(&self, handle: &Handle<T>) -> Option<Resolved<'_, T>> {
        self.get_by_id(handle.id())
    }

    pub fn get_by_id(&self, id: AssetId<T>) -> Option<Resolved<'_, T>> {
        match self.slot(id) {
            Some(Slot {
                status: AssetStatus::Ready,
                value: Some(value),
                ..
            }) => Some(Resolved {
                asset: value,
                status: AssetStatus::Ready,
            }),
            Some(Slot {
                status: status @ AssetStatus::Failed(_),
                ..
            }) => self.error_fallback.as_ref().map(|asset| Resolved {
                asset,
                status: status.clone(),
            }),
            // Ещё грузится или результат ещё не применён
            _ => self.loading_fallback.as_ref().map(|asset| Resolved {
                asset,
                status: AssetStatus::Loading,
            }),
        }
    }

    /// Только настоящий ресурс, без заглушек.
    pub fn get_loaded(&self, handle: &Handle<T>) -> Option<&T> {
        self.slot(handle.id()).and_then(|slot| slot.value.as_ref())
    }

    pub fn get_mut(&mut self, handle: &Handle<T>) -> Option<&mut T> {
        let id = handle.id();
        let slot = self.slots.get_mut(id.index as usize)?.as_mut()?;
        (slot.generation == id.generation)
            .then_some(slot.value.as_mut())
            .flatten()
    }

    pub fn status(&self, handle: &Handle<T>) -> AssetStatus {
        self.slot(handle.id())
            .map_or(AssetStatus::Loading, |slot| slot.status.clone())
    }

    /// Добавляет ресурс, созданный в коде (например, сгенерированный меш).
    pub fn add(&mut self, value: T) -> Handle<T> {
        let server = self
            .server
            .clone()
            .expect("Assets<T> не зарегистрирован через init_asset");
        let handle = server.allocate::<T>(None);
        self.set_loaded(handle.id(), value);
        server.set_status(handle.untyped_id(), AssetStatus::Ready);
        handle
    }

    /// Количество загруженных ресурсов.
    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .flatten()
            .filter(|slot| slot.value.is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn slot(&self, id: AssetId<T>) -> Option<&Slot<T>> {
        let slot = self.slots.get(id.index as usize)?.as_ref()?;
        (slot.generation == id.generation).then_some(slot)
    }

    fn slot_entry(&mut self, id: AssetId<T>) -> &mut Option<Slot<T>> {
        let index = id.index as usize;
        if self.slots.len() <= index {
            // [Rust] resize_with заполняет новые элементы результатом замыкания
            self.slots.resize_with(index + 1, || None);
        }
        &mut self.slots[index]
    }

    pub(crate) fn set_loaded(&mut self, id: AssetId<T>, value: T) {
        *self.slot_entry(id) = Some(Slot {
            generation: id.generation,
            status: AssetStatus::Ready,
            value: Some(value),
        });
    }

    pub(crate) fn set_failed(&mut self, id: AssetId<T>, error: AssetError) {
        *self.slot_entry(id) = Some(Slot {
            generation: id.generation,
            status: AssetStatus::Failed(error),
            value: None,
        });
    }

    pub(crate) fn remove(&mut self, id: AssetId<T>) -> Option<T> {
        let entry = self.slot_entry(id);
        if entry
            .as_ref()
            .is_some_and(|slot| slot.generation == id.generation)
        {
            return entry.take().and_then(|slot| slot.value);
        }
        None
    }
}

impl<T: Asset> std::fmt::Debug for Assets<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(&format!("Assets<{}>", short_type_name::<T>()))
            .field("loaded", &self.len())
            .finish_non_exhaustive()
    }
}
