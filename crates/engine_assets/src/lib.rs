//! Ресурсы (`docs/acrhitecture.md`, раздел 7).
//!
//! ```ignore
//! fn startup(server: Res<AssetServer>, mut commands: Commands) {
//!     let helmet: Handle<Gltf> = server.load("models/khronos/DamagedHelmet/DamagedHelmet.glb");
//!     commands.insert_resource(Scene { helmet });
//! }
//!
//! fn update(scene: Res<Scene>, gltfs: Res<Assets<Gltf>>) {
//!     if let Some(resolved) = gltfs.get(&scene.helmet) { /* resolved.asset, resolved.status */ }
//! }
//! ```
//!
//! - Хэндл выдаётся сразу, загрузка идёт в задаче, результат применяется в начале кадра.
//! - Пока ресурс грузится или если загрузка упала — подставляется заглушка этого типа
//!   (если она есть), а статус говорит, что произошло.
//! - Когда исчезает последний сильный хэндл, ресурс выгружается.

mod error;
mod handle;
mod loader;
mod path;
mod server;
mod source;
mod storage;
#[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios")))]
mod watch;

pub mod formats;

use engine_core::{App, CVar, CVarFlags, CVarString, Plugin, Stage};

pub use error::AssetError;
pub use handle::{AssetId, Handle, UntypedAssetId};
pub use loader::{AssetLoader, LoadContext};
pub use path::AssetPath;
pub use server::AssetServer;
pub use source::Bytes;
pub use storage::{AssetEvent, AssetStatus, Assets, Resolved};

/// Тип ресурса. Заглушки — что показывать, пока ресурс грузится и если загрузка упала.
///
/// [Rust] Методы с телом — реализация по умолчанию: тип без заглушек просто пишет
/// `impl Asset for MyType {}`.
pub trait Asset: Send + Sync + 'static {
    fn loading_fallback() -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
    fn error_fallback() -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

pub static ASSETS_ROOT: CVar<CVarString> = CVar::new(
    "assets.root",
    std::borrow::Cow::Borrowed("assets"),
    "Корень ресурсов: папка на desktop, префикс URL на Web",
)
.with_flags(CVarFlags::STARTUP);

pub static ASSETS_WATCH: CVar<bool> = CVar::new(
    "assets.watch",
    true,
    "Перезагружать изменённые файлы ресурсов (только desktop)",
)
.with_flags(CVarFlags::STARTUP);

pub static ASSETS_STRICT: CVar<bool> = CVar::new(
    "assets.strict",
    false,
    "Ошибка загрузки ресурса — паника (для CI и тестов)",
);

/// Подключает систему ресурсов и встроенные форматы (изображения, glTF, WGSL).
/// Требует `NetPlugin` — для загрузки по HTTP и на Web.
#[derive(Debug, Default)]
pub struct AssetPlugin;

impl Plugin for AssetPlugin {
    fn build(&self, app: &mut App) {
        assert!(
            app.is_plugin_added::<engine_net::NetPlugin>(),
            "AssetPlugin требует NetPlugin: добавьте его раньше"
        );
        app.register_cvar(&ASSETS_ROOT)
            .register_cvar(&ASSETS_STRICT)
            .register_cvar(&ASSETS_WATCH)
            .insert_resource(AssetServer::new())
            .add_systems(Stage::First, server::apply_loads);
        formats::register(app);
    }

    fn finish(&self, app: &mut App) {
        // Здесь уже применены CVar (файл, аргументы) и создан клиент сети (NetPlugin::finish
        // вызывается раньше — плагины завершаются в порядке добавления)
        let net = app.world().get_resource::<engine_net::Net>().cloned();
        let server = app.world().resource::<AssetServer>();
        server.configure(net, ASSETS_ROOT.get().into_owned());
        #[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios")))]
        if ASSETS_WATCH.get() {
            server.start_watching(std::path::Path::new(ASSETS_ROOT.get().as_ref()));
        }
    }
}

/// Методы регистрации типов ресурсов и загрузчиков в [`App`].
// [Rust] Трейт-расширение: добавляет методы чужому типу App (своего App мы тут не владеем).
// Аналог свободных функций, но вызываются через точку: `app.init_asset::<Image>()`.
pub trait AssetAppExt {
    fn init_asset<T: Asset>(&mut self) -> &mut Self;
    fn register_loader<L: AssetLoader>(&mut self, loader: L) -> &mut Self;
}

impl AssetAppExt for App {
    fn init_asset<T: Asset>(&mut self) -> &mut Self {
        if self.world().contains_resource::<Assets<T>>() {
            return self;
        }
        let server = self.world().resource::<AssetServer>().clone();
        server.register_type::<T>();
        let mut assets = Assets::<T>::new();
        assets.set_server(server);
        self.insert_resource(assets).add_message::<AssetEvent<T>>();
        self
    }

    fn register_loader<L: AssetLoader>(&mut self, loader: L) -> &mut Self {
        self.init_asset::<L::Asset>();
        self.world()
            .resource::<AssetServer>()
            .register_loader(loader);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{Gltf, Mesh};
    use engine_core::CVarSource;

    #[test]
    fn load_dedup_and_unload_with_labeled() {
        let mut app = App::new();
        // Тест запускается из папки крейта — корень ресурсов на два уровня выше
        app.world_mut()
            .resource_mut::<engine_core::CVars>()
            .set("assets.root", "../../assets", CVarSource::LaunchArguments)
            .unwrap();
        app.add_plugin(engine_net::NetPlugin)
            .add_plugin(AssetPlugin);
        app.update(); // finish плагинов, Startup

        let server = app.world().resource::<AssetServer>().clone();
        let a: Handle<Gltf> = server.load("models/khronos/Box/Box.glb");
        let b: Handle<Gltf> = server.load("models/khronos/Box/Box.glb");
        assert_eq!(a, b, "один путь — один ресурс");

        // Ждём загрузку (в тесте можно покрутить кадры)
        for _ in 0..500 {
            app.update();
            if matches!(server.is_loaded_with_dependencies(a.untyped_id()), Ok(true)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mesh = {
            let gltfs = app.world().resource::<Assets<Gltf>>();
            let gltf = gltfs.get_loaded(&a).expect("Box.glb загружен");
            gltf.meshes[0].weak()
        };
        assert!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get_loaded(&mesh)
                .is_some()
        );

        // Последние сильные хэндлы уничтожены → в следующем кадре выгрузка, каскадом и меш
        drop(a);
        drop(b);
        app.update();
        app.update();
        assert!(app.world().resource::<Assets<Gltf>>().is_empty());
        assert!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get_loaded(&mesh)
                .is_none()
        );
    }
}
