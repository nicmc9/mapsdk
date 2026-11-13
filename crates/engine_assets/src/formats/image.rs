use engine_core::tasks::BoxedFuture;

use crate::{Asset, AssetError, AssetLoader, Bytes, LoadContext};

/// Цветовое пространство данных изображения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    /// Цвет, видимый человеком: базовый цвет, свечение. GPU переводит в линейное при чтении.
    Srgb,
    /// Данные, а не цвет: нормали, металличность/шероховатость, маски.
    Linear,
}

/// Изображение в памяти: RGBA, 8 бит на канал.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub color_space: ColorSpace,
    /// `width * height * 4` байт, строки сверху вниз.
    pub data: Vec<u8>,
}

impl Image {
    /// Декодирует PNG или JPEG.
    pub fn decode(bytes: &[u8], color_space: ColorSpace) -> Result<Self, String> {
        let _zone = tracing::trace_span!("image_decode", bytes = bytes.len()).entered();
        let decoded = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let rgba = decoded.to_rgba8();
        Ok(Self {
            width: rgba.width(),
            height: rgba.height(),
            color_space,
            data: rgba.into_raw(),
        })
    }

    fn solid(rgba: [u8; 4]) -> Self {
        Self {
            width: 1,
            height: 1,
            color_space: ColorSpace::Srgb,
            data: rgba.to_vec(),
        }
    }

    /// Розово-чёрная «шахматка» 8×8 — сразу видно, что текстура не загрузилась.
    fn checker() -> Self {
        const SIZE: u32 = 8;
        let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let pink = (x + y) % 2 == 0;
                data.extend_from_slice(if pink {
                    &[255, 0, 255, 255]
                } else {
                    &[0, 0, 0, 255]
                });
            }
        }
        Self {
            width: SIZE,
            height: SIZE,
            color_space: ColorSpace::Srgb,
            data,
        }
    }
}

impl Asset for Image {
    fn loading_fallback() -> Option<Self> {
        Some(Self::solid([128, 128, 128, 255]))
    }
    fn error_fallback() -> Option<Self> {
        Some(Self::checker())
    }
}

/// PNG и JPEG. Отдельно загружаемые картинки считаются цветом (sRGB).
#[derive(Debug)]
pub struct ImageLoader;

impl AssetLoader for ImageLoader {
    type Asset = Image;

    fn extensions(&self) -> &[&'static str] {
        &["png", "jpg", "jpeg"]
    }

    fn load<'a>(
        &'a self,
        bytes: Bytes,
        ctx: &'a mut LoadContext,
    ) -> BoxedFuture<'a, Result<Image, AssetError>> {
        Box::pin(async move {
            Image::decode(&bytes, ColorSpace::Srgb).map_err(|e| AssetError::format(ctx.path(), e))
        })
    }
}
