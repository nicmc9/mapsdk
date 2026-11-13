//! Встроенные форматы.

mod gltf;
mod image;
mod shader;

pub use self::gltf::{Gltf, GltfLoader, Material, Mesh, Primitive};
pub use self::image::{Image, ImageLoader};
pub use self::shader::{Shader, ShaderLoader};

use engine_core::App;

use crate::AssetAppExt;

pub(crate) fn register(app: &mut App) {
    app.register_loader(ImageLoader)
        .register_loader(ShaderLoader)
        .init_asset::<Mesh>()
        .register_loader(GltfLoader);
}
