use std::any::{Any, TypeId};

use engine_core::tasks::BoxedFuture;

use crate::handle::short_type_name;
use crate::source::Bytes;
use crate::{Asset, AssetError, AssetPath, AssetServer, Handle, UntypedAssetId};

/// Загрузчик формата: превращает байты файла в ресурс.
///
/// ```ignore
/// struct TextLoader;
/// impl AssetLoader for TextLoader {
///     type Asset = Text;
///     fn extensions(&self) -> &[&'static str] { &["txt"] }
///     fn load<'a>(&'a self, bytes: Bytes, _ctx: &'a mut LoadContext)
///         -> BoxedFuture<'a, Result<Text, AssetError>>
///     {
///         Box::pin(async move { Ok(Text(String::from_utf8_lossy(&bytes).into_owned())) })
///     }
/// }
/// ```
///
/// [Rust] `load` асинхронный, чтобы загрузчик мог дочитать внешние файлы
/// (`ctx.read(..).await`), не блокируя поток в браузере. В трейтах, которые хранятся
/// как трейт-объекты, `async fn` пока нельзя — поэтому future возвращается в коробке
/// (`BoxedFuture`), а тело оборачивается в `Box::pin(async move { ... })`.
pub trait AssetLoader: Send + Sync + 'static {
    type Asset: Asset;
    /// Расширения файлов в нижнем регистре, без точки.
    fn extensions(&self) -> &[&'static str];
    fn load<'a>(
        &'a self,
        bytes: Bytes,
        ctx: &'a mut LoadContext,
    ) -> BoxedFuture<'a, Result<Self::Asset, AssetError>>;
}

/// Загруженное значение без типа — сервер не знает типов, их знают зарегистрированные функции.
pub(crate) type AnyAsset = Box<dyn Any + Send + Sync>;

/// Загрузчик без типа ресурса — чтобы хранить загрузчики разных типов в одном списке.
// [Rust] Стирание типа (type erasure): обобщённый трейт с ассоциированным типом нельзя
// положить в Vec<Box<dyn ...>>. Поэтому есть второй трейт без типа и автоматическая
// реализация его для любого AssetLoader — аналог базового класса с виртуальными методами.
pub(crate) trait ErasedLoader: Send + Sync {
    fn extensions(&self) -> &[&'static str];
    fn asset_type(&self) -> TypeId;
    fn asset_type_name(&self) -> &'static str;
    fn loader_name(&self) -> &'static str;
    fn load<'a>(
        &'a self,
        bytes: Bytes,
        ctx: &'a mut LoadContext,
    ) -> BoxedFuture<'a, Result<AnyAsset, AssetError>>;
}

impl<L: AssetLoader> ErasedLoader for L {
    fn extensions(&self) -> &[&'static str] {
        AssetLoader::extensions(self)
    }
    fn asset_type(&self) -> TypeId {
        TypeId::of::<L::Asset>()
    }
    fn asset_type_name(&self) -> &'static str {
        short_type_name::<L::Asset>()
    }
    fn loader_name(&self) -> &'static str {
        short_type_name::<L>()
    }
    fn load<'a>(
        &'a self,
        bytes: Bytes,
        ctx: &'a mut LoadContext,
    ) -> BoxedFuture<'a, Result<AnyAsset, AssetError>> {
        Box::pin(async move {
            let asset = AssetLoader::load(self, bytes, ctx).await?;
            Ok(Box::new(asset) as AnyAsset)
        })
    }
}

/// Вложенный ресурс, созданный загрузчиком (меш внутри glTF).
pub(crate) struct Labeled {
    pub id: UntypedAssetId,
    pub value: AnyAsset,
}

/// Контекст загрузки: дочитать соседние файлы, запросить зависимости, добавить вложенные ресурсы.
pub struct LoadContext {
    server: AssetServer,
    path: AssetPath,
    pub(crate) labeled: Vec<Labeled>,
    pub(crate) dependencies: Vec<UntypedAssetId>,
}

impl std::fmt::Debug for LoadContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadContext")
            .field("path", &self.path)
            .field("labeled", &self.labeled.len())
            .finish_non_exhaustive()
    }
}

impl LoadContext {
    pub(crate) fn new(server: AssetServer, path: AssetPath) -> Self {
        Self {
            server,
            path,
            labeled: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    /// Путь загружаемого ресурса.
    pub fn path(&self) -> &AssetPath {
        &self.path
    }

    /// Читает файл рядом с ресурсом (например, `.bin`-буфер glTF).
    pub async fn read(&self, relative: &str) -> Result<Bytes, AssetError> {
        let path = self.path.resolve(relative);
        self.server
            .read(&path)
            .await
            .map_err(|error| AssetError::Dependency {
                dependency: path.to_string(),
                source: Box::new(error),
            })
    }

    /// Запрашивает зависимость (например, внешнюю текстуру). Грузится параллельно.
    pub fn load<T: Asset>(&mut self, relative: &str) -> Handle<T> {
        let handle = self.server.load::<T>(self.path.resolve(relative).as_str());
        self.dependencies.push(handle.untyped_id());
        handle
    }

    /// Добавляет вложенный ресурс с меткой: путь будет `файл#метка`.
    pub fn add_labeled<T: Asset>(&mut self, label: &str, value: T) -> Handle<T> {
        let handle = self.server.allocate::<T>(Some(self.path.with_label(label)));
        self.labeled.push(Labeled {
            id: handle.untyped_id(),
            value: Box::new(value),
        });
        self.dependencies.push(handle.untyped_id());
        handle
    }
}
