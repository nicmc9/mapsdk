use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};

use crossbeam_channel::{Receiver, Sender};
use engine_core::tasks;
use engine_ecs::prelude::*;
use engine_net::Net;
use tracing::Instrument;

use crate::handle::{StrongRef, short_type_name};
use crate::loader::{AnyAsset, ErasedLoader, Labeled, LoadContext};
use crate::source::{self, Bytes};
use crate::{
    ASSETS_STRICT, Asset, AssetError, AssetEvent, AssetId, AssetLoader, AssetPath, AssetStatus,
    Assets, Handle, UntypedAssetId,
};

/// Сервер ресурсов: выдаёт хэндлы, запускает загрузки, применяет результаты.
/// Дёшево клонируется (внутри `Arc`) — его копия есть у каждой задачи загрузки.
#[derive(Clone, Resource)]
pub struct AssetServer {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    loaders: RwLock<Vec<Arc<dyn ErasedLoader>>>,
    types: RwLock<HashMap<TypeId, TypeOps>>,
    /// Готовые загрузки → система `apply_loads`.
    results: (Sender<LoadResult>, Receiver<LoadResult>),
    /// Исчез последний сильный хэндл → система `apply_loads` выгрузит ресурс.
    drops: (Sender<UntypedAssetId>, Receiver<UntypedAssetId>),
    net: RwLock<Option<Net>>,
    root: RwLock<String>,
}

#[derive(Default)]
struct State {
    /// Для каждого типа — свободные индексы и поколения.
    allocators: HashMap<TypeId, Allocator>,
    by_path: HashMap<(AssetPath, TypeId), UntypedAssetId>,
    infos: HashMap<UntypedAssetId, Info>,
}

struct Info {
    path: Option<AssetPath>,
    status: AssetStatus,
    /// Слабая ссылка на владение: по ней повторный `load` того же пути отдаёт тот же ресурс.
    owner: Weak<StrongRef>,
    dependencies: Vec<UntypedAssetId>,
}

#[derive(Default)]
struct Allocator {
    generations: Vec<u32>,
    free: Vec<u32>,
}

impl Allocator {
    fn allocate(&mut self) -> (u32, u32) {
        if let Some(index) = self.free.pop() {
            (index, self.generations[index as usize])
        } else {
            let index = u32::try_from(self.generations.len()).expect("слишком много ресурсов");
            self.generations.push(0);
            (index, 0)
        }
    }

    fn free(&mut self, index: u32) {
        // Новое поколение: старые id с этим индексом больше ничего не найдут
        self.generations[index as usize] += 1;
        self.free.push(index);
    }
}

/// Операции над ресурсами конкретного типа, сохранённые как указатели на функции.
// [Rust] `fn(...)` — обычный указатель на функцию. `apply_loaded::<T>` — отдельная функция
// для каждого T (мономорфизация шаблона), и сервер вызывает её, не зная T.
#[derive(Clone, Copy)]
struct TypeOps {
    name: &'static str,
    apply_loaded: fn(&mut World, UntypedAssetId, AnyAsset),
    apply_failed: fn(&mut World, UntypedAssetId, AssetError),
    remove: fn(&mut World, UntypedAssetId),
}

struct LoadResult {
    id: UntypedAssetId,
    result: Result<AnyAsset, AssetError>,
    labeled: Vec<Labeled>,
    dependencies: Vec<UntypedAssetId>,
}

impl std::fmt::Debug for AssetServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        f.debug_struct("AssetServer")
            .field("assets", &state.infos.len())
            .finish_non_exhaustive()
    }
}

impl AssetServer {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::default(),
                loaders: RwLock::default(),
                types: RwLock::default(),
                results: crossbeam_channel::unbounded(),
                drops: crossbeam_channel::unbounded(),
                net: RwLock::default(),
                root: RwLock::new(String::new()),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Загружает ресурс. Хэндл возвращается сразу; повторный вызов с тем же путём и
    /// типом вернёт тот же ресурс.
    pub fn load<T: Asset>(&self, path: &str) -> Handle<T> {
        let path = AssetPath::new(path);
        let key = (path.clone(), TypeId::of::<T>());
        let mut state = self.lock();
        if let Some(&id) = state.by_path.get(&key) {
            let info = state.infos.get_mut(&id).expect("путь без записи");
            let owner = info.owner.upgrade().unwrap_or_else(|| {
                // Все хэндлы уже уничтожены, но выгрузка ещё не прошла — возрождаем владение
                let owner = Arc::new(self.strong_ref(id));
                info.owner = Arc::downgrade(&owner);
                owner
            });
            return Handle::strong(typed(id), owner);
        }
        drop(state);

        let handle = self.allocate::<T>(Some(path.clone()));
        if path.label().is_some() {
            // Вложенный ресурс: грузим файл целиком, метку заполнит его загрузчик
            self.load_parent_of(&path, handle.untyped_id());
        } else {
            self.spawn_load(handle.untyped_id(), path, Some(TypeId::of::<T>()));
        }
        handle
    }

    /// Состояние ресурса.
    pub fn status(&self, id: UntypedAssetId) -> AssetStatus {
        self.lock()
            .infos
            .get(&id)
            .map_or(AssetStatus::Loading, |info| info.status.clone())
    }

    /// Загружен сам ресурс и все его зависимости (рекурсивно).
    /// `Err` — что-то упало, в ошибке — что именно.
    pub fn is_loaded_with_dependencies(&self, id: UntypedAssetId) -> Result<bool, AssetError> {
        let state = self.lock();
        let mut stack = vec![id];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(info) = state.infos.get(&id) else {
                return Ok(false);
            };
            match &info.status {
                AssetStatus::Loading => return Ok(false),
                AssetStatus::Failed(error) => return Err(error.clone()),
                AssetStatus::Ready => stack.extend(&info.dependencies),
            }
        }
        Ok(true)
    }

    /// Сколько ресурсов загружается, загружено, упало.
    pub fn counts(&self) -> (usize, usize, usize) {
        let state = self.lock();
        let mut counts = (0, 0, 0);
        for info in state.infos.values() {
            match info.status {
                AssetStatus::Loading => counts.0 += 1,
                AssetStatus::Ready => counts.1 += 1,
                AssetStatus::Failed(_) => counts.2 += 1,
            }
        }
        counts
    }

    // ---------- внутреннее ----------

    pub(crate) fn register_type<T: Asset>(&self) {
        self.inner
            .types
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                TypeId::of::<T>(),
                TypeOps {
                    name: short_type_name::<T>(),
                    apply_loaded: apply_loaded::<T>,
                    apply_failed: apply_failed::<T>,
                    remove: remove::<T>,
                },
            );
    }

    pub(crate) fn register_loader<L: AssetLoader>(&self, loader: L) {
        self.inner
            .loaders
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(loader));
    }

    pub(crate) fn configure(&self, net: Option<Net>, root: String) {
        *self
            .inner
            .net
            .write()
            .unwrap_or_else(PoisonError::into_inner) = net;
        *self
            .inner
            .root
            .write()
            .unwrap_or_else(PoisonError::into_inner) = root;
    }

    fn strong_ref(&self, id: UntypedAssetId) -> StrongRef {
        StrongRef {
            id,
            drop_sender: self.inner.drops.0.clone(),
        }
    }

    /// Выделяет id (или находит существующий по пути) и возвращает сильный хэндл.
    pub(crate) fn allocate<T: Asset>(&self, path: Option<AssetPath>) -> Handle<T> {
        let type_id = TypeId::of::<T>();
        let mut state = self.lock();
        if let Some(path) = &path
            && let Some(&id) = state.by_path.get(&(path.clone(), type_id))
        {
            let info = state.infos.get_mut(&id).expect("путь без записи");
            let owner = info.owner.upgrade().unwrap_or_else(|| {
                let owner = Arc::new(self.strong_ref(id));
                info.owner = Arc::downgrade(&owner);
                owner
            });
            return Handle::strong(typed(id), owner);
        }
        let (index, generation) = state.allocators.entry(type_id).or_default().allocate();
        let id = UntypedAssetId {
            type_id,
            index,
            generation,
        };
        let owner = Arc::new(self.strong_ref(id));
        if let Some(path) = &path {
            state.by_path.insert((path.clone(), type_id), id);
        }
        state.infos.insert(
            id,
            Info {
                path,
                status: AssetStatus::Loading,
                owner: Arc::downgrade(&owner),
                dependencies: Vec::new(),
            },
        );
        Handle::strong(typed(id), owner)
    }

    pub(crate) fn set_status(&self, id: UntypedAssetId, status: AssetStatus) {
        if let Some(info) = self.lock().infos.get_mut(&id) {
            info.status = status;
        }
    }

    pub(crate) async fn read(&self, path: &AssetPath) -> Result<Bytes, AssetError> {
        let net = self
            .inner
            .net
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let root = self
            .inner
            .root
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        source::read(path, &root, net.as_ref()).await
    }

    /// Вложенный ресурс запрошен до загрузки файла: запускаем загрузку файла
    /// (тип узнаём по загрузчику для расширения) и держим его, пока жив вложенный.
    fn load_parent_of(&self, path: &AssetPath, child: UntypedAssetId) {
        let file = AssetPath::new(path.file());
        let Some(loader) = self.loader_for(&file, None) else {
            self.fail_now(child, AssetError::NoLoader(file.to_string()));
            return;
        };
        let type_id = loader.asset_type();
        let mut state = self.lock();
        let parent = if let Some(&id) = state.by_path.get(&(file.clone(), type_id)) {
            id
        } else {
            let (index, generation) = state.allocators.entry(type_id).or_default().allocate();
            let id = UntypedAssetId {
                type_id,
                index,
                generation,
            };
            state.by_path.insert((file.clone(), type_id), id);
            state.infos.insert(
                id,
                Info {
                    path: Some(file.clone()),
                    status: AssetStatus::Loading,
                    owner: Weak::new(),
                    dependencies: Vec::new(),
                },
            );
            drop(state);
            self.spawn_load(id, file, None);
            state = self.lock();
            id
        };
        // Вложенный зависит от файла: файл не выгрузится, пока жив вложенный
        if let Some(info) = state.infos.get_mut(&child) {
            info.dependencies.push(parent);
        }
    }

    fn loader_for(
        &self,
        path: &AssetPath,
        asset_type: Option<TypeId>,
    ) -> Option<Arc<dyn ErasedLoader>> {
        let extension = path.extension()?;
        let loaders = self
            .inner
            .loaders
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        loaders
            .iter()
            .filter(|loader| loader.extensions().contains(&extension.as_str()))
            .find(|loader| asset_type.is_none_or(|t| loader.asset_type() == t))
            .or_else(|| {
                loaders
                    .iter()
                    .find(|loader| loader.extensions().contains(&extension.as_str()))
            })
            .cloned()
    }

    fn fail_now(&self, id: UntypedAssetId, error: AssetError) {
        let _ = self.inner.results.0.send(LoadResult {
            id,
            result: Err(error),
            labeled: Vec::new(),
            dependencies: Vec::new(),
        });
    }

    fn spawn_load(&self, id: UntypedAssetId, path: AssetPath, asset_type: Option<TypeId>) {
        let Some(loader) = self.loader_for(&path, asset_type) else {
            self.fail_now(id, AssetError::NoLoader(path.to_string()));
            return;
        };
        if let Some(requested) = asset_type
            && loader.asset_type() != requested
        {
            let requested = self.type_name(requested);
            self.fail_now(
                id,
                AssetError::WrongType {
                    loader: loader.loader_name(),
                    produced: loader.asset_type_name(),
                    requested,
                },
            );
            return;
        }
        let server = self.clone();
        // [Rust] `detach` — задача работает сама по себе, результат придёт через канал
        let span = tracing::trace_span!("asset_load", path = path.as_str());
        tasks::spawn_io(
            async move {
                let mut ctx = LoadContext::new(server.clone(), path.clone());
                let result = match server.read(&path).await {
                    Ok(bytes) => loader.load(bytes, &mut ctx).await,
                    Err(error) => Err(error),
                };
                let _ = server.inner.results.0.send(LoadResult {
                    id,
                    result,
                    labeled: std::mem::take(&mut ctx.labeled),
                    dependencies: std::mem::take(&mut ctx.dependencies),
                });
            }
            .instrument(span),
        )
        .detach();
    }

    fn type_name(&self, type_id: TypeId) -> &'static str {
        self.inner
            .types
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&type_id)
            .map_or("?", |ops| ops.name)
    }

    fn ops(&self, type_id: TypeId) -> Option<TypeOps> {
        self.inner
            .types
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&type_id)
            .copied()
    }

    fn apply_result(&self, world: &mut World, result: LoadResult) {
        let LoadResult {
            id,
            result,
            labeled,
            dependencies,
        } = result;
        let path = {
            let mut state = self.lock();
            let Some(info) = state.infos.get_mut(&id) else {
                return; // ресурс выгружен, пока грузился
            };
            info.dependencies.extend(dependencies);
            info.status = match &result {
                Ok(_) => AssetStatus::Ready,
                Err(error) => AssetStatus::Failed(error.clone()),
            };
            info.path.clone()
        };
        let Some(ops) = self.ops(id.type_id) else {
            tracing::error!("Тип ресурса не зарегистрирован (init_asset)");
            return;
        };
        for child in labeled {
            if let Some(child_ops) = self.ops(child.id.type_id) {
                self.set_status(child.id, AssetStatus::Ready);
                (child_ops.apply_loaded)(world, child.id, child.value);
            }
        }
        let path = path.map_or_else(|| "<без пути>".to_owned(), |p| p.to_string());
        match result {
            Ok(value) => {
                tracing::debug!("Ресурс загружен: {path}");
                (ops.apply_loaded)(world, id, value);
                self.fail_missing_labels(world, &path);
            }
            Err(error) => {
                tracing::error!("Ресурс {path} ({}): {error}", ops.name);
                assert!(
                    !ASSETS_STRICT.get(),
                    "assets.strict: не удалось загрузить {path}: {error}"
                );
                (ops.apply_failed)(world, id, error.clone());
                self.fail_labels_of(world, &path, &error);
            }
        }
    }

    /// Вложенные ресурсы, запрошенные по метке, которой в файле не оказалось.
    fn fail_missing_labels(&self, world: &mut World, file: &str) {
        let missing: Vec<(UntypedAssetId, String)> = {
            let state = self.lock();
            state
                .infos
                .iter()
                .filter(|(_, info)| matches!(info.status, AssetStatus::Loading))
                .filter_map(|(id, info)| {
                    let path = info.path.as_ref()?;
                    (path.label().is_some() && path.file() == file).then(|| (*id, path.to_string()))
                })
                .collect()
        };
        for (id, path) in missing {
            let error = AssetError::NotFound(path);
            self.set_status(id, AssetStatus::Failed(error.clone()));
            if let Some(ops) = self.ops(id.type_id) {
                (ops.apply_failed)(world, id, error);
            }
        }
    }

    fn fail_labels_of(&self, world: &mut World, file: &str, error: &AssetError) {
        let children: Vec<UntypedAssetId> = {
            let state = self.lock();
            state
                .infos
                .iter()
                .filter(|(_, info)| {
                    info.path
                        .as_ref()
                        .is_some_and(|p| p.label().is_some() && p.file() == file)
                })
                .map(|(id, _)| *id)
                .collect()
        };
        for id in children {
            self.set_status(id, AssetStatus::Failed(error.clone()));
            if let Some(ops) = self.ops(id.type_id) {
                (ops.apply_failed)(world, id, error.clone());
            }
        }
    }

    /// Выгружает ресурс, если на него не осталось сильных хэндлов.
    fn try_unload(&self, world: &mut World, id: UntypedAssetId) {
        let removed = {
            let mut state = self.lock();
            let Some(info) = state.infos.get(&id) else {
                return;
            };
            if info.owner.strong_count() > 0 {
                return; // хэндл успели получить снова
            }
            let info = state.infos.remove(&id).expect("проверено выше");
            if let Some(path) = &info.path {
                state.by_path.remove(&(path.clone(), id.type_id));
            }
            if let Some(allocator) = state.allocators.get_mut(&id.type_id) {
                allocator.free(id.index);
            }
            info
        };
        if let Some(ops) = self.ops(id.type_id) {
            tracing::debug!(
                "Ресурс выгружен: {}",
                removed.path.map_or_else(String::new, |p| p.to_string())
            );
            // Выгрузка значения уничтожит хэндлы внутри него (меши glTF) — каскад
            (ops.remove)(world, id);
        }
    }
}

/// Применяет готовые загрузки и выгружает ресурсы без владельцев. Начало кадра.
// [Rust] Система с `&mut World` — «эксклюзивная»: монопольный доступ ко всему миру,
// выполняется одна, без параллельных систем. Нужна, потому что пишем в ресурсы любых типов.
pub(crate) fn apply_loads(world: &mut World) {
    let _zone = tracing::trace_span!("apply_loads").entered();
    let server = world.resource::<AssetServer>().clone();
    // Ограничение на кадр: применение результата может быть дорогим (вставка больших данных)
    for _ in 0..256 {
        let Ok(result) = server.inner.results.1.try_recv() else {
            break;
        };
        server.apply_result(world, result);
    }
    while let Ok(id) = server.inner.drops.1.try_recv() {
        server.try_unload(world, id);
    }
}

fn typed<T: Asset>(id: UntypedAssetId) -> AssetId<T> {
    debug_assert_eq!(id.type_id, TypeId::of::<T>());
    AssetId::new(id.index, id.generation)
}

fn apply_loaded<T: Asset>(world: &mut World, id: UntypedAssetId, value: AnyAsset) {
    // [Rust] downcast — проверка типа во время выполнения, аналог dynamic_cast
    let Ok(value) = value.downcast::<T>() else {
        tracing::error!("Загрузчик вернул не тот тип");
        return;
    };
    let id = typed::<T>(id);
    world.resource_mut::<Assets<T>>().set_loaded(id, *value);
    world.write_message(AssetEvent::Loaded(id));
}

fn apply_failed<T: Asset>(world: &mut World, id: UntypedAssetId, error: AssetError) {
    let id = typed::<T>(id);
    world
        .resource_mut::<Assets<T>>()
        .set_failed(id, error.clone());
    world.write_message(AssetEvent::Failed(id, error));
}

fn remove<T: Asset>(world: &mut World, id: UntypedAssetId) {
    let id = typed::<T>(id);
    let removed = world.resource_mut::<Assets<T>>().remove(id);
    world.write_message(AssetEvent::Unloaded(id));
    // Значение уничтожается здесь, уже без заимствования мира
    drop(removed);
}
