//! Загрузка на native: блокирующий `ureq` в потоке пула IO + дисковый кеш.

use std::sync::atomic::Ordering;

use crate::cache::{CacheEntry, DiskCache};
use crate::{Bytes, Counters, NetError, Settings, is_retryable_status};

pub(crate) struct Backend {
    agent: ureq::Agent,
    cache: Option<DiskCache>,
}

impl Backend {
    pub fn new(settings: &Settings) -> Self {
        // [Rust] ureq — синхронный HTTP-клиент на чистом Rust, TLS через rustls (без OpenSSL).
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(settings.connect_timeout))
            .timeout_global(Some(settings.timeout))
            // Коды 4xx/5xx — не ошибка транспорта: разбираем статус сами (304, повторы)
            .http_status_as_error(false)
            .user_agent("mapsdk-engine")
            .build()
            .into();
        let cache = if settings.cache_max_bytes == 0 {
            None
        } else {
            engine_platform::paths::cache_dir()
                .and_then(|dir| DiskCache::open(dir.join("net"), settings.cache_max_bytes))
        };
        if cache.is_none() && settings.cache_max_bytes > 0 {
            tracing::warn!("Сеть: дисковый кеш недоступен");
        }
        Self { agent, cache }
    }

    pub fn fetch(
        &self,
        url: &str,
        settings: &Settings,
        counters: &Counters,
    ) -> Result<Bytes, NetError> {
        let _zone = tracing::trace_span!("net_fetch", url).entered();
        let cached = self.cache.as_ref().and_then(|cache| cache.lookup(url));
        let mut last_error = NetError::Transport("нет попыток".to_owned());

        for attempt in 0..=settings.retries {
            if attempt > 0 {
                counters.retries.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Settings::backoff(attempt));
            }
            match self.request(url, cached.as_ref(), settings) {
                Ok(Response::NotModified) => {
                    if let Some(bytes) = self.read_cached(url, cached.as_ref()) {
                        counters.cache_hits.fetch_add(1, Ordering::Relaxed);
                        counters.revalidated.fetch_add(1, Ordering::Relaxed);
                        return Ok(bytes);
                    }
                    last_error = NetError::Transport("304 без данных в кеше".to_owned());
                }
                Ok(Response::Body { bytes, entry }) => {
                    counters
                        .bytes_downloaded
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    if let Some(cache) = &self.cache
                        && let Err(error) = cache.store(url, &bytes, &entry)
                    {
                        tracing::warn!("Кеш: не удалось сохранить {url}: {error}");
                    }
                    return Ok(bytes.into());
                }
                Err(error) => {
                    let retry = match &error {
                        NetError::Status(code) => is_retryable_status(*code),
                        NetError::Timeout | NetError::Transport(_) => true,
                        NetError::TooLarge(_) => false,
                    };
                    last_error = error;
                    if !retry {
                        break;
                    }
                }
            }
        }

        // Сеть недоступна, но есть старая копия — лучше она, чем ничего (работа офлайн)
        if !matches!(last_error, NetError::Status(code) if code < 500)
            && let Some(bytes) = self.read_cached(url, cached.as_ref())
        {
            tracing::warn!("Сеть: {url}: {last_error}, используется кеш");
            counters.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(bytes);
        }
        Err(last_error)
    }

    fn request(
        &self,
        url: &str,
        cached: Option<&CacheEntry>,
        settings: &Settings,
    ) -> Result<Response, NetError> {
        let mut request = self.agent.get(url);
        // Условный запрос: сервер ответит 304, если файл не изменился
        if let Some(entry) = cached {
            if let Some(etag) = &entry.etag {
                request = request.header("If-None-Match", etag);
            }
            if let Some(modified) = &entry.last_modified {
                request = request.header("If-Modified-Since", modified);
            }
        }
        let mut response = request.call().map_err(map_error)?;
        let status = response.status().as_u16();
        if status == 304 {
            return Ok(Response::NotModified);
        }
        if !(200..300).contains(&status) {
            return Err(NetError::Status(status));
        }
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let entry = CacheEntry {
            etag: header("etag"),
            last_modified: header("last-modified"),
        };
        let bytes = response
            .body_mut()
            .with_config()
            .limit(settings.max_response_bytes)
            .read_to_vec()
            .map_err(|error| match error {
                ureq::Error::BodyExceedsLimit(limit) => NetError::TooLarge(limit),
                other => map_error(other),
            })?;
        Ok(Response::Body { bytes, entry })
    }

    fn read_cached(&self, url: &str, cached: Option<&CacheEntry>) -> Option<Bytes> {
        cached?;
        self.cache.as_ref()?.read(url).ok().map(Bytes::from)
    }
}

enum Response {
    NotModified,
    Body { bytes: Vec<u8>, entry: CacheEntry },
}

fn map_error(error: ureq::Error) -> NetError {
    match error {
        ureq::Error::Timeout(_) => NetError::Timeout,
        ureq::Error::StatusCode(code) => NetError::Status(code),
        // У ошибки ввода-вывода Display короткий («io») — берём текст исходной ошибки ОС
        ureq::Error::Io(error) => NetError::Transport(format!("ввод-вывод: {error}")),
        other => NetError::Transport(other.to_string()),
    }
}
