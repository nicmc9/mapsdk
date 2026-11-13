//! Встроенные форматы.

mod gltf;
mod image;
mod shader;

pub use self::gltf::{AlphaMode, Gltf, GltfLoader, Material, Mesh, Node, Primitive, Skin};
pub use self::image::{ColorSpace, Image, ImageLoader};
pub use self::shader::{Shader, ShaderLoader};

use engine_core::App;

use crate::AssetAppExt;

pub(crate) fn register(app: &mut App) {
    app.register_loader(ImageLoader)
        .register_loader(ShaderLoader)
        .init_asset::<Mesh>()
        .init_asset::<Material>()
        .register_loader(GltfLoader);
}
