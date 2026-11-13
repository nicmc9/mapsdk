use engine_assets::AssetId;
use engine_assets::formats::{AlphaMode, Image, Material, Mesh};
use engine_core::sync::reply::ReplySender;
use engine_platform::WindowSize;

/// Материал для рендера: текстуры — по id (рендеру не нужно владеть хэндлами).
#[derive(Debug, Clone)]
pub(crate) struct MaterialData {
    pub base_color: [f32; 4],
    pub base_color_texture: Option<AssetId<Image>>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_texture: Option<AssetId<Image>>,
    pub normal_texture: Option<AssetId<Image>>,
    pub occlusion_texture: Option<AssetId<Image>>,
    pub emissive: [f32; 3],
    pub emissive_texture: Option<AssetId<Image>>,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
}

impl From<&Material> for MaterialData {
    fn from(m: &Material) -> Self {
        let id =
            |h: &Option<engine_assets::Handle<Image>>| h.as_ref().map(engine_assets::Handle::id);
        Self {
            base_color: m.base_color,
            base_color_texture: id(&m.base_color_texture),
            metallic: m.metallic,
            roughness: m.roughness,
            metallic_roughness_texture: id(&m.metallic_roughness_texture),
            normal_texture: id(&m.normal_texture),
            occlusion_texture: id(&m.occlusion_texture),
            emissive: m.emissive,
            emissive_texture: id(&m.emissive_texture),
            alpha_mode: m.alpha_mode,
            double_sided: m.double_sided,
        }
    }
}

/// Изменения текстур egui. egui 0.37 паникует, если изменения уничтожены необработанными
/// (защита от потерянного атласа шрифта). Но бывает, что применить их некуда: GPU так и не
/// инициализировался, рендер завершается. Обёртка в этом случае явно их отбрасывает.
#[derive(Debug)]
pub(crate) struct UiTextures(pub egui::TexturesDelta);

impl Drop for UiTextures {
    // [Rust] Drop вызывается при любом уничтожении значения — и после применения,
    // и если команда так и осталась в очереди. Аналог деструктора в C++.
    fn drop(&mut self) {
        self.0.clear();
    }
}

/// Снимок экрана: RGBA, 8 бит на канал, sRGB.
#[derive(Debug, Clone)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Команды рендеру. Отправляются из главного потока, разбираются в начале кадра рендера.
// [Rust] Варианты enum несут данные (как std::variant с именованными полями).
// `match` по команде обязан обработать все варианты — забыть новый нельзя.
pub(crate) enum RenderCommand {
    /// Окно доступно: surface создан в главном потоке (на macOS/iOS иначе нельзя).
    SurfaceCreated {
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        size: WindowSize,
    },
    /// Рисовать без окна в текстуру (тесты, серверный рендер).
    InitHeadless {
        instance: wgpu::Instance,
        size: WindowSize,
    },
    /// Окно скоро будет уничтожено (уход в фон). Рендер обязан отпустить surface
    /// и ответить — до ответа главный поток не вернёт управление ОС.
    SurfaceLost {
        reply: ReplySender<()>,
    },
    Resize(WindowSize),
    // Данные ресурсов перемещаются в рендер (копия делается один раз, при загрузке)
    UploadMesh(AssetId<Mesh>, Box<Mesh>),
    UploadImage(AssetId<Image>, Box<Image>),
    UploadMaterial(AssetId<Material>, MaterialData),
    RemoveMesh(AssetId<Mesh>),
    RemoveImage(AssetId<Image>),
    RemoveMaterial(AssetId<Material>),
    /// Изменения текстур интерфейса (атлас шрифта) — терять нельзя, поэтому командой.
    UiTextures(Box<UiTextures>),
    /// Новый исходник шейдера движка (горячая перезагрузка).
    UpdateShader {
        name: String,
        source: String,
    },
    /// Снимок следующего кадра.
    Screenshot {
        reply: ReplySender<Result<Screenshot, String>>,
    },
    /// Завершить работу рендера.
    Shutdown,
}

// Ручной Debug: wgpu-объекты и данные ресурсов печатать бессмысленно
impl std::fmt::Debug for RenderCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SurfaceCreated { size, .. } => write!(f, "SurfaceCreated({size:?})"),
            Self::InitHeadless { size, .. } => write!(f, "InitHeadless({size:?})"),
            Self::SurfaceLost { .. } => write!(f, "SurfaceLost"),
            Self::Resize(size) => write!(f, "Resize({size:?})"),
            Self::UploadMesh(id, _) => write!(f, "UploadMesh({id:?})"),
            Self::UploadImage(id, _) => write!(f, "UploadImage({id:?})"),
            Self::UploadMaterial(id, _) => write!(f, "UploadMaterial({id:?})"),
            Self::RemoveMesh(id) => write!(f, "RemoveMesh({id:?})"),
            Self::RemoveImage(id) => write!(f, "RemoveImage({id:?})"),
            Self::RemoveMaterial(id) => write!(f, "RemoveMaterial({id:?})"),
            Self::UiTextures(_) => write!(f, "UiTextures"),
            Self::UpdateShader { name, .. } => write!(f, "UpdateShader({name})"),
            Self::Screenshot { .. } => write!(f, "Screenshot"),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}
