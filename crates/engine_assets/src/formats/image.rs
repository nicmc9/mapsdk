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
    /// Уменьшенные копии (мип-уровни) 1..N: каждая вдвое меньше предыдущей, до 1×1.
    /// GPU выбирает уровень по расстоянию — без них дальние текстуры мерцают.
    pub mips: Vec<Vec<u8>>,
}

impl Image {
    /// Декодирует PNG или JPEG.
    pub fn decode(bytes: &[u8], color_space: ColorSpace) -> Result<Self, String> {
        let _zone = tracing::trace_span!("image_decode", bytes = bytes.len()).entered();
        let decoded = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let rgba = decoded.to_rgba8();
        let mut image = Self {
            width: rgba.width(),
            height: rgba.height(),
            color_space,
            data: rgba.into_raw(),
            mips: Vec::new(),
        };
        image.generate_mips();
        Ok(image)
    }

    /// Число уровней вместе с основным.
    pub fn mip_level_count(&self) -> u32 {
        1 + u32::try_from(self.mips.len()).unwrap_or(0)
    }

    /// Строит мип-уровни усреднением 2×2. Для sRGB усредняет в линейном пространстве:
    /// прямое усреднение закодированных значений делает дальние уровни темнее.
    pub fn generate_mips(&mut self) {
        let _zone = tracing::trace_span!("generate_mips").entered();
        self.mips.clear();
        let srgb = self.color_space == ColorSpace::Srgb;
        let (mut width, mut height) = (self.width, self.height);
        let mut source = self.data.clone();
        while width > 1 || height > 1 {
            let (next_w, next_h) = ((width / 2).max(1), (height / 2).max(1));
            let mut next = vec![0u8; (next_w * next_h * 4) as usize];
            for y in 0..next_h {
                for x in 0..next_w {
                    for channel in 0..4 {
                        let mut sum = 0.0f32;
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let sx = (x * 2 + dx).min(width - 1);
                            let sy = (y * 2 + dy).min(height - 1);
                            let value = source[((sy * width + sx) * 4 + channel) as usize];
                            // Альфа всегда линейная
                            sum += if srgb && channel < 3 {
                                SRGB_TO_LINEAR[value as usize]
                            } else {
                                f32::from(value) / 255.0
                            };
                        }
                        let average = sum / 4.0;
                        let encoded = if srgb && channel < 3 {
                            linear_to_srgb(average)
                        } else {
                            average
                        };
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let byte = (encoded * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                        next[((y * next_w + x) * 4 + channel) as usize] = byte;
                    }
                }
            }
            self.mips.push(next.clone());
            source = next;
            width = next_w;
            height = next_h;
        }
    }

    fn solid(rgba: [u8; 4]) -> Self {
        Self {
            width: 1,
            height: 1,
            color_space: ColorSpace::Srgb,
            data: rgba.to_vec(),
            mips: Vec::new(),
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
        // Без мип-уровней: «шахматка» должна оставаться заметной издалека
        Self {
            width: SIZE,
            height: SIZE,
            color_space: ColorSpace::Srgb,
            data,
            mips: Vec::new(),
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

/// Таблица sRGB → линейное для 256 значений байта.
// [Rust] `LazyLock` — значение вычисляется при первом обращении (аналог static-переменной
// с инициализацией внутри функции в C++), потокобезопасно.
static SRGB_TO_LINEAR: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
    std::array::from_fn(|i| {
        #[allow(clippy::cast_precision_loss)]
        let c = i as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
});

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_down_to_one_pixel_and_srgb_average() {
        // 2×1: чёрный и белый. Среднее в линейном пространстве = 0.5 → sRGB ≈ 188, а не 128
        let mut image = Image {
            width: 2,
            height: 1,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255, 255, 255, 255, 255],
            mips: Vec::new(),
        };
        image.generate_mips();
        assert_eq!(image.mip_level_count(), 2);
        assert_eq!(&image.mips[0][..], &[188, 188, 188, 255]);
    }
}
