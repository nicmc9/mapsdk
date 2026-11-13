//! Виртуальная файловая система: откуда берутся байты ресурса.
//!
//! | Путь | Платформа | Источник |
//! |------|-----------|----------|
//! | `http(s)://...` | все | сеть (`engine_net`): кеш, повторы, дедупликация |
//! | относительный | macOS, Linux | папка `assets.root` (по умолчанию `assets`) |
//! | относительный | Web | HTTP: `assets.root` + путь относительно страницы |
//! | относительный | iOS | папка `assets` внутри .app |
//! | относительный | Android | assets внутри APK |

use engine_net::Net;

use crate::{AssetError, AssetPath};

pub type Bytes = engine_net::Bytes;

pub(crate) async fn read(
    path: &AssetPath,
    root: &str,
    net: Option<&Net>,
) -> Result<Bytes, AssetError> {
    // [Rust] Зону профайлера нельзя держать открытой через `.await` с помощью `entered()`:
    // после ожидания задача может продолжиться в другом потоке. `instrument` открывает
    // зону на каждом шаге выполнения future.
    use tracing::Instrument;
    let span = tracing::trace_span!("asset_read", path = path.file());
    read_inner(path, root, net).instrument(span).await
}

async fn read_inner(path: &AssetPath, root: &str, net: Option<&Net>) -> Result<Bytes, AssetError> {
    if path.is_remote() {
        let net = net.ok_or(AssetError::NoNetwork)?;
        return Ok(net.get(path.file()).await?);
    }
    read_local(path.file(), root, net).await
}

#[cfg(not(any(target_arch = "wasm32", target_os = "ios", target_os = "android")))]
// Сигнатура общая для всех платформ: на Web чтение асинхронное
#[allow(clippy::unused_async)]
async fn read_local(file: &str, root: &str, _net: Option<&Net>) -> Result<Bytes, AssetError> {
    // Блокирующее чтение допустимо: мы в потоке пула IO
    let full = std::path::Path::new(root).join(file);
    std::fs::read(&full)
        .map(Bytes::from)
        .map_err(|error| io_error(file, &error))
}

#[cfg(any(target_os = "ios", target_os = "android"))]
#[allow(clippy::unused_async)]
async fn read_local(file: &str, _root: &str, _net: Option<&Net>) -> Result<Bytes, AssetError> {
    engine_platform::paths::read_bundled(file)
        .map(Bytes::from)
        .map_err(|error| io_error(file, &error))
}

#[cfg(target_arch = "wasm32")]
async fn read_local(file: &str, root: &str, net: Option<&Net>) -> Result<Bytes, AssetError> {
    let net = net.ok_or(AssetError::NoNetwork)?;
    let url = format!("{}/{file}", root.trim_end_matches('/'));
    match net.get(&url).await {
        Err(engine_net::NetError::Status(404)) => Err(AssetError::NotFound(file.to_owned())),
        other => Ok(other?),
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn io_error(file: &str, error: &std::io::Error) -> AssetError {
    if error.kind() == std::io::ErrorKind::NotFound {
        AssetError::NotFound(file.to_owned())
    } else {
        AssetError::Io {
            path: file.to_owned(),
            reason: error.to_string(),
        }
    }
}
