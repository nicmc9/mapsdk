//! Пути приложения и файлы, поставляемые вместе с приложением.

use std::path::PathBuf;

/// Имя каталога приложения внутри системных каталогов.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // на Web файловой системы нет
const APP_DIR: &str = "mapsdk";

/// Каталог для кеша (можно удалить в любой момент — данные скачаются снова).
/// `None` — на платформе нет файловой системы (Web) или она ещё недоступна.
pub fn cache_dir() -> Option<PathBuf> {
    platform_cache_dir()
}

#[cfg(target_os = "macos")]
fn platform_cache_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Caches").join(APP_DIR))
}

#[cfg(target_os = "linux")]
fn platform_cache_dir() -> Option<PathBuf> {
    // Стандарт XDG: $XDG_CACHE_HOME или ~/.cache
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join(APP_DIR))
}

#[cfg(target_os = "ios")]
fn platform_cache_dir() -> Option<PathBuf> {
    // HOME в iOS — контейнер приложения (песочница)
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Caches").join(APP_DIR))
}

#[cfg(target_os = "android")]
fn platform_cache_dir() -> Option<PathBuf> {
    let app = crate::android::app()?;
    Some(app.internal_data_path()?.join("cache").join(APP_DIR))
}

#[cfg(target_arch = "wasm32")]
fn platform_cache_dir() -> Option<PathBuf> {
    None
}

/// Читает файл, поставляемый вместе с приложением: из папки `assets` внутри .app на iOS,
/// из Android assets внутри APK. На desktop и Web не используется (там обычная папка и HTTP).
pub fn read_bundled(path: &str) -> std::io::Result<Vec<u8>> {
    platform_read_bundled(path)
}

#[cfg(target_os = "ios")]
fn platform_read_bundled(path: &str) -> std::io::Result<Vec<u8>> {
    let exe = std::env::current_exe()?;
    let bundle = exe.parent().ok_or(std::io::ErrorKind::NotFound)?;
    std::fs::read(bundle.join("assets").join(path))
}

#[cfg(target_os = "android")]
fn platform_read_bundled(path: &str) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let app = crate::android::app().ok_or(std::io::ErrorKind::NotConnected)?;
    let name = std::ffi::CString::new(path).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let mut asset = app
        .asset_manager()
        .open(&name)
        .ok_or(std::io::ErrorKind::NotFound)?;
    let mut bytes = Vec::new();
    asset.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn platform_read_bundled(_path: &str) -> std::io::Result<Vec<u8>> {
    Err(std::io::ErrorKind::Unsupported.into())
}
