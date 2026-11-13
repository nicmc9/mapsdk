//! Чтение кадра с GPU в память процессора.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::command::Screenshot;

pub(crate) struct Readback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row: u32,
    bgra: bool,
}

/// Копирует текстуру в буфер. Строки в буфере выравниваются на 256 байт — требование GPU.
pub(crate) fn copy_to_buffer(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
) -> Readback {
    let (width, height) = (texture.width(), texture.height());
    let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot"),
        size: u64::from(padded_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    let bgra = matches!(
        texture.format(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    Readback {
        buffer,
        width,
        height,
        padded_row,
        bgra,
    }
}

impl Readback {
    /// Ждёт GPU и читает данные. Блокирует поток (скриншоты редки).
    /// На Web блокировать нельзя — там скриншоты пока не поддерживаются.
    pub fn read(self, device: &wgpu::Device) -> Result<Screenshot, String> {
        let slice = self.buffer.slice(..);
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        // [Rust] Обратный вызов вызовется внутри device.poll — флаг передаём через Arc
        slice.map_async(wgpu::MapMode::Read, move |result| {
            if result.is_ok() {
                flag.store(true, Ordering::Release);
            }
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        if !done.load(Ordering::Acquire) {
            return Err("не удалось прочитать кадр с GPU".to_owned());
        }
        let mapped = slice.get_mapped_range().map_err(|e| e.to_string())?;
        let row = (self.width * 4) as usize;
        let mut data = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            let start = y * self.padded_row as usize;
            data.extend_from_slice(&mapped[start..start + row]);
        }
        drop(mapped);
        if self.bgra {
            // [Rust] as_chunks_mut — срез массивов [u8; 4]: длина куска известна компилятору
            for pixel in data.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        Ok(Screenshot {
            width: self.width,
            height: self.height,
            data,
        })
    }
}
