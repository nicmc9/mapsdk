//! Шейдеры движка (R2-16): встроенные исходники, препроцессор, сборка с проверкой ошибок.
//!
//! Исходники встроены в бинарник и доступны ресурсам как `engine://shaders/*.wgsl`.
//! При разработке на desktop CVar `assets.engine_dir = "crates/engine_render/src"` берёт их
//! из папки, и правка `.wgsl` применяется на лету (горячая перезагрузка ресурсов).

use std::collections::HashMap;

/// Встроенные исходники: путь относительно `engine://` и текст.
// [Rust] `include_str!` вставляет содержимое файла в бинарник на этапе компиляции.
pub(crate) const EMBEDDED: &[(&str, &str)] = &[
    ("shaders/common.wgsl", include_str!("shaders/common.wgsl")),
    ("shaders/pbr.wgsl", include_str!("shaders/pbr.wgsl")),
    ("shaders/debug.wgsl", include_str!("shaders/debug.wgsl")),
];

pub(crate) fn embedded_sources() -> HashMap<String, String> {
    EMBEDDED
        .iter()
        .map(|(name, source)| ((*name).to_owned(), (*source).to_owned()))
        .collect()
}

/// Препроцессор + создание модуля. Ошибки WGSL ловятся через error scope и возвращаются
/// текстом — без этого wgpu сообщил бы о них через обработчик ошибок устройства (паника).
pub(crate) fn compile(
    device: &wgpu::Device,
    sources: &HashMap<String, String>,
    name: &str,
) -> Result<wgpu::ShaderModule, String> {
    let code = crate::preprocess::preprocess(name, sources, &[])?;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(code.into()),
    });
    let error = pop_scope(scope);
    match error {
        Some(error) => Err(format!("{name}: {error}")),
        None => Ok(module),
    }
}

/// Создание пайплайна с перехватом ошибок валидации (несовпадение раскладок и т.п.).
pub(crate) fn checked<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, String> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = create();
    match pop_scope(scope) {
        Some(error) => Err(error.to_string()),
        None => Ok(value),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pop_scope(scope: wgpu::ErrorScopeGuard) -> Option<wgpu::Error> {
    // На native результат готов сразу — ожидание не блокирует
    engine_core::tasks::block_on(scope.pop())
}

#[cfg(target_arch = "wasm32")]
fn pop_scope(scope: wgpu::ErrorScopeGuard) -> Option<wgpu::Error> {
    // В браузере блокировать нельзя; горячей перезагрузки на Web нет — проверку пропускаем
    drop(scope.pop());
    None
}
