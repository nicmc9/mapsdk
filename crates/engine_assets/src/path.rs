use std::fmt;

/// Путь ресурса: `models/box.glb`, `models/box.glb#Mesh0` (вложенный ресурс)
/// или `https://cdn.example.com/box.glb`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetPath(String);

impl AssetPath {
    pub fn new(path: &str) -> Self {
        Self(path.trim().replace('\\', "/"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Путь к файлу без метки вложенного ресурса.
    pub fn file(&self) -> &str {
        self.0.split_once('#').map_or(&self.0, |(file, _)| file)
    }

    /// Метка вложенного ресурса (`Mesh0` в `box.glb#Mesh0`).
    pub fn label(&self) -> Option<&str> {
        self.0.split_once('#').map(|(_, label)| label)
    }

    #[must_use]
    pub fn with_label(&self, label: &str) -> Self {
        Self(format!("{}#{label}", self.file()))
    }

    pub fn is_remote(&self) -> bool {
        self.0.starts_with("http://") || self.0.starts_with("https://")
    }

    /// Расширение файла в нижнем регистре.
    pub fn extension(&self) -> Option<String> {
        let file = self.file();
        let name = file.rsplit('/').next().unwrap_or(file);
        let name = name.split('?').next().unwrap_or(name);
        name.rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
    }

    /// Путь относительно папки этого ресурса: для `models/a.gltf` и `tex/b.png` —
    /// `models/tex/b.png`. Учитывает `..`.
    #[must_use]
    pub fn resolve(&self, relative: &str) -> Self {
        let file = self.file();
        let base = file.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut parts: Vec<&str> = if base.is_empty() {
            Vec::new()
        } else {
            base.split('/').collect()
        };
        // Для URL схема и хост («https:», «», «host») не должны схлопываться через `..`
        let protected = if self.is_remote() { 3 } else { 0 };
        for part in relative.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.len() > protected {
                        parts.pop();
                    }
                }
                other => parts.push(other),
            }
        }
        Self(parts.join("/"))
    }
}

impl fmt::Display for AssetPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_extensions_and_relative_paths() {
        let path = AssetPath::new("models/Sponza/Sponza.gltf#Mesh3");
        assert_eq!(path.file(), "models/Sponza/Sponza.gltf");
        assert_eq!(path.label(), Some("Mesh3"));
        assert_eq!(path.extension().as_deref(), Some("gltf"));
        assert_eq!(
            path.resolve("tex/a.jpg").as_str(),
            "models/Sponza/tex/a.jpg"
        );
        assert_eq!(path.resolve("../b.png").as_str(), "models/b.png");
        let url = AssetPath::new("https://host/dir/x.glb");
        assert!(url.is_remote());
        assert_eq!(url.resolve("../../y.bin").as_str(), "https://host/y.bin");
    }
}
