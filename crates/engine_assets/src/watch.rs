//! Горячая перезагрузка (A-05): изменённый в папке `assets` файл перезагружается
//! в тот же хэндл. Только desktop — на мобильных и Web ресурсы упакованы в приложение.

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, Sender};

/// Следит за папкой и отдаёт изменённые пути относительно неё.
pub(crate) struct Watcher {
    // Наблюдатель работает, пока жив этот объект
    _watcher: notify::RecommendedWatcher,
    changes: Receiver<String>,
}

impl Watcher {
    pub fn start(root: &Path) -> Option<Self> {
        use notify::Watcher as _;
        let root = std::fs::canonicalize(root).ok()?;
        let (tx, rx): (Sender<String>, Receiver<String>) = crossbeam_channel::unbounded();
        let base = root.clone();
        // [Rust] Обработчик вызывается в потоке notify — передаём пути через канал
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if !(event.kind.is_modify() || event.kind.is_create()) {
                    return;
                }
                for path in event.paths {
                    if let Some(relative) = relative(&base, &path) {
                        let _ = tx.send(relative);
                    }
                }
            })
            .ok()?;
        watcher
            .watch(&root, notify::RecursiveMode::Recursive)
            .ok()?;
        tracing::info!("Горячая перезагрузка: слежу за {}", root.display());
        Some(Self {
            _watcher: watcher,
            changes: rx,
        })
    }

    /// Изменённые с прошлого вызова пути (без повторов).
    pub fn changes(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.changes.try_iter().collect();
        paths.sort();
        paths.dedup();
        paths
    }
}

fn relative(base: &Path, path: &Path) -> Option<String> {
    let path: PathBuf = std::fs::canonicalize(path).ok()?;
    let relative = path.strip_prefix(base).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
}
