//! Дисковый кеш HTTP-ответов.
//!
//! Для каждого URL два файла: `<хэш>.bin` (тело) и `<хэш>.meta` (`ETag`, `Last-Modified`).
//! Время изменения `.bin` обновляется при каждом чтении — по нему при переполнении удаляются
//! давно не использованные записи (LRU). Запись атомарная: во временный файл, затем rename —
//! при падении посреди записи в кеше не окажется обрезанного файла.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Default)]
pub(crate) struct CacheEntry {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

pub(crate) struct DiskCache {
    dir: PathBuf,
    max_bytes: u64,
}

impl DiskCache {
    pub fn open(dir: PathBuf, max_bytes: u64) -> Option<Self> {
        if let Err(error) = fs::create_dir_all(&dir) {
            tracing::warn!("Кеш: не удалось создать {}: {error}", dir.display());
            return None;
        }
        tracing::info!("Кеш сети: {}", dir.display());
        Some(Self { dir, max_bytes })
    }

    pub fn lookup(&self, url: &str) -> Option<CacheEntry> {
        let (body, meta) = self.paths(url);
        if !body.exists() {
            return None;
        }
        let text = fs::read_to_string(meta).unwrap_or_default();
        let mut lines = text.lines();
        let read = |line: Option<&str>| line.filter(|l| !l.is_empty()).map(str::to_owned);
        Some(CacheEntry {
            etag: read(lines.next()),
            last_modified: read(lines.next()),
        })
    }

    pub fn read(&self, url: &str) -> io::Result<Vec<u8>> {
        let (body, _) = self.paths(url);
        let bytes = fs::read(&body)?;
        // Отмечаем использование для LRU; ошибка не критична
        let _ = fs::File::options()
            .write(true)
            .open(&body)
            .and_then(|file| file.set_modified(SystemTime::now()));
        Ok(bytes)
    }

    pub fn store(&self, url: &str, bytes: &[u8], entry: &CacheEntry) -> io::Result<()> {
        if bytes.len() as u64 > self.max_bytes {
            return Ok(()); // не поместится целиком — не кешируем
        }
        let (body, meta) = self.paths(url);
        let meta_text = format!(
            "{}\n{}\n{url}\n",
            entry.etag.as_deref().unwrap_or(""),
            entry.last_modified.as_deref().unwrap_or("")
        );
        write_atomic(&body, bytes)?;
        write_atomic(&meta, meta_text.as_bytes())?;
        self.evict()
    }

    /// Удаляет давно не использованные записи, пока кеш больше лимита.
    fn evict(&self) -> io::Result<()> {
        let mut entries = Vec::new();
        let mut total = 0u64;
        for item in fs::read_dir(&self.dir)? {
            let path = item?.path();
            if path.extension().is_some_and(|ext| ext == "bin") {
                let meta = fs::metadata(&path)?;
                total += meta.len();
                entries.push((
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    meta.len(),
                    path,
                ));
            }
        }
        if total <= self.max_bytes {
            return Ok(());
        }
        // [Rust] Кортежи сравниваются по полям слева направо — сортировка по времени
        entries.sort();
        for (_, len, path) in entries {
            if total <= self.max_bytes {
                break;
            }
            let _ = fs::remove_file(path.with_extension("meta"));
            fs::remove_file(&path)?;
            total -= len;
        }
        Ok(())
    }

    fn paths(&self, url: &str) -> (PathBuf, PathBuf) {
        let name = format!("{:016x}", fnv1a(url.as_bytes()));
        (
            self.dir.join(format!("{name}.bin")),
            self.dir.join(format!("{name}.meta")),
        )
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// FNV-1a: простой стабильный хэш для имён файлов.
/// [Rust] `std::hash::DefaultHasher` не подходит: его алгоритм может поменяться между
/// версиями Rust, и имена файлов кеша перестанут совпадать.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache(max_bytes: u64) -> (DiskCache, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "engine-net-test-{}-{max_bytes}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        (DiskCache::open(dir.clone(), max_bytes).unwrap(), dir)
    }

    #[test]
    fn store_lookup_read() {
        let (cache, dir) = temp_cache(1024);
        let entry = CacheEntry {
            etag: Some("\"abc\"".to_owned()),
            last_modified: None,
        };
        cache.store("https://x/a", b"hello", &entry).unwrap();
        let found = cache.lookup("https://x/a").unwrap();
        assert_eq!(found.etag.as_deref(), Some("\"abc\""));
        assert_eq!(cache.read("https://x/a").unwrap(), b"hello");
        assert!(cache.lookup("https://x/b").is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn evicts_least_recently_used() {
        let (cache, dir) = temp_cache(25);
        let entry = CacheEntry::default();
        cache.store("u1", &[1; 10], &entry).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.store("u2", &[2; 10], &entry).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.read("u1").unwrap(); // u1 теперь свежее u2
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.store("u3", &[3; 10], &entry).unwrap(); // 30 байт > 25 — вытесняется u2
        assert!(cache.lookup("u1").is_some());
        assert!(cache.lookup("u2").is_none());
        assert!(cache.lookup("u3").is_some());
        fs::remove_dir_all(dir).unwrap();
    }
}
