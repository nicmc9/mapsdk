//! Время GPU по проходам (R2-17): метки в начале и конце прохода (timestamp queries).
//!
//! Результаты читаются кольцом из трёх буферов без ожидания GPU: видим время кадра двух-
//! трёхкадровой давности, зато кадр не простаивает. Возможность необязательная: на GLES и
//! части мобильных GPU её нет — тогда таймер не создаётся.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::graph::PassKind;

/// Проходы, время которых меряем. Индекс — слот меток.
pub(crate) const TIMED_PASSES: [PassKind; 2] = [PassKind::Scene, PassKind::Ui];
// Проходов единицы — усечение usize → u32 невозможно
#[allow(clippy::cast_possible_truncation)]
const QUERY_COUNT: u32 = TIMED_PASSES.len() as u32 * 2;
const RESULT_SIZE: u64 = QUERY_COUNT as u64 * 8;

// Состояния буфера чтения
const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;

struct Readback {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

pub(crate) struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readbacks: Vec<Readback>,
    /// Наносекунд в одном «тике» метки GPU.
    period: f32,
    /// Последнее известное время прохода, мкс.
    pub results: [Option<u64>; TIMED_PASSES.len()],
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            tracing::info!("Время GPU: метки времени не поддерживаются");
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pass_timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamps_resolve"),
            size: RESULT_SIZE,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readbacks = (0..3)
            .map(|_| Readback {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timestamps_read"),
                    size: RESULT_SIZE,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            readbacks,
            period: queue.get_timestamp_period(),
            results: [None; TIMED_PASSES.len()],
        })
    }

    /// Метки для прохода; `None` — проход не измеряется.
    pub fn writes(&self, pass: PassKind) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let slot = u32::try_from(TIMED_PASSES.iter().position(|&p| p == pass)?).ok()?;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(slot * 2),
            end_of_pass_write_index: Some(slot * 2 + 1),
        })
    }

    /// После проходов: метки → буфер чтения. Возвращает номер буфера (или `None`, если все
    /// три ещё заняты — тогда этот кадр не измеряем).
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) -> Option<usize> {
        let index = self
            .readbacks
            .iter()
            .position(|r| r.state.load(Ordering::Acquire) == FREE)?;
        encoder.resolve_query_set(&self.queries, 0..QUERY_COUNT, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &self.resolve,
            0,
            &self.readbacks[index].buffer,
            0,
            RESULT_SIZE,
        );
        self.readbacks[index]
            .state
            .store(PENDING, Ordering::Release);
        Some(index)
    }

    /// После отправки кадра: попросить GPU отдать буфер, когда будет готов.
    pub fn after_submit(&self, index: usize) {
        let state = Arc::clone(&self.readbacks[index].state);
        self.readbacks[index]
            .buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(if result.is_ok() { READY } else { FREE }, Ordering::Release);
            });
    }

    /// Забирает готовые результаты (без ожидания).
    pub fn collect(&mut self, device: &wgpu::Device) {
        // Неблокирующий опрос: вызывает обработчики уже завершённых map_async
        let _ = device.poll(wgpu::PollType::Poll);
        for readback in &self.readbacks {
            if readback.state.load(Ordering::Acquire) != READY {
                continue;
            }
            if let Ok(view) = readback.buffer.slice(..).get_mapped_range() {
                let ticks: &[u64] = bytemuck::cast_slice(&view);
                for (slot, pair) in ticks.as_chunks::<2>().0.iter().enumerate() {
                    // Проход мог не выполниться в этом кадре — тогда метки пустые
                    self.results[slot] = (pair[1] > pair[0]).then(|| {
                        #[allow(
                            clippy::cast_precision_loss,
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss
                        )]
                        let micros =
                            ((pair[1] - pair[0]) as f64 * f64::from(self.period) / 1000.0) as u64;
                        micros
                    });
                }
            }
            readback.buffer.unmap();
            readback.state.store(FREE, Ordering::Release);
        }
    }
}
