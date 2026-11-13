use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

/// Устройство GPU: всё, что нужно для работы с видеокартой.
#[derive(Debug)]
pub(crate) struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// Асинхронная инициализация GPU.
///
/// [Rust] Запрос адаптера и устройства в `wgpu` — асинхронный (`Future`). На native он
/// завершается сразу, а в браузере ждёт промис JavaScript. Блокирующе ждать в браузере
/// нельзя, поэтому future хранится здесь и опрашивается (`poll`) каждый кадр, пока не
/// будет готов. Исполнитель (executor) для этого не нужен.
pub(crate) struct GpuInit {
    future: InitFuture,
}

// [Rust] `Pin<Box<dyn Future>>` — future в куче с закреплённым адресом. Async-код может
// хранить ссылки на собственные поля, поэтому перемещать его в памяти после первого
// poll нельзя — Pin это гарантирует.
//
// На native future должен быть `Send`: рендер вместе с ним переезжает в свой поток.
// На Web объекты wgpu — это JS-объекты, они не `Send`, но и потоков там нет.
#[cfg(not(target_arch = "wasm32"))]
type InitFuture = Pin<Box<dyn Future<Output = Result<Gpu, String>> + Send>>;
#[cfg(target_arch = "wasm32")]
type InitFuture = Pin<Box<dyn Future<Output = Result<Gpu, String>>>>;

impl GpuInit {
    /// `surface` — `None` для рендера без окна.
    pub fn start(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'static>>) -> Self {
        // Future адаптера создаётся до async-блока: он не держит ссылку на surface,
        // поэтому весь блок может жить сколько угодно ('static).
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            // Остальное — по умолчанию (программный адаптер не нужен и т.п.)
            ..Default::default()
        });
        let future = async move {
            let adapter = adapter.await.map_err(|e| format!("адаптер: {e}"))?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("engine"),
                    // Лимиты, которые поддерживает этот адаптер (GLES3 и мобильные GPU слабее desktop)
                    required_limits: adapter.limits(),
                    ..Default::default()
                })
                .await
                .map_err(|e| format!("устройство: {e}"))?;
            Ok(Gpu {
                adapter,
                device,
                queue,
            })
        };
        Self {
            future: Box::pin(future),
        }
    }

    /// Проверяет готовность. `None` — ещё ждём.
    pub fn poll(&mut self) -> Option<Result<Gpu, String>> {
        // Waker::noop: нас не нужно будить — опросим сами в следующем кадре
        let mut context = Context::from_waker(Waker::noop());
        match self.future.as_mut().poll(&mut context) {
            Poll::Ready(result) => Some(result),
            Poll::Pending => None,
        }
    }
}

impl std::fmt::Debug for GpuInit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GpuInit")
    }
}
