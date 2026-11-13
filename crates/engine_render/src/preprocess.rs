//! Препроцессор WGSL (R2-16): `#include "файл.wgsl"`, `#define ИМЯ`,
//! `#ifdef ИМЯ` / `#ifndef ИМЯ` / `#else` / `#endif`.
//!
//! Своя реализация вместо `naga_oil`: тот привязан к версии `naga` внутри `wgpu`, и каждое
//! обновление `wgpu` превращалось бы в подбор совместимой пары. Нам нужно немного.

use std::collections::{HashMap, HashSet};

/// Раскрывает директивы. `sources` — все известные файлы шейдеров по имени
/// (`"shaders/pbr.wgsl"`), `defines` — заранее заданные имена.
pub(crate) fn preprocess(
    name: &str,
    sources: &HashMap<String, String>,
    defines: &[&str],
) -> Result<String, String> {
    let mut defines: HashSet<String> = defines.iter().map(|d| (*d).to_owned()).collect();
    let mut included = HashSet::new();
    let mut out = String::new();
    expand(name, sources, &mut defines, &mut included, &mut out)?;
    Ok(out)
}

fn expand(
    name: &str,
    sources: &HashMap<String, String>,
    defines: &mut HashSet<String>,
    included: &mut HashSet<String>,
    out: &mut String,
) -> Result<(), String> {
    // Каждый файл включается один раз (как #pragma once)
    if !included.insert(name.to_owned()) {
        return Ok(());
    }
    let source = sources
        .get(name)
        .ok_or_else(|| format!("шейдер «{name}» не найден"))?;
    let dir = name.rsplit_once('/').map_or("", |(dir, _)| dir);
    // Стек условий: активна ли текущая ветка
    let mut active: Vec<bool> = Vec::new();
    for (line_number, line) in source.lines().enumerate() {
        let directive = line.trim_start();
        let location = || format!("{name}:{}", line_number + 1);
        let enabled = active.iter().all(|&a| a);
        if let Some(rest) = directive.strip_prefix("#ifdef ") {
            active.push(defines.contains(rest.trim()));
        } else if let Some(rest) = directive.strip_prefix("#ifndef ") {
            active.push(!defines.contains(rest.trim()));
        } else if directive.starts_with("#else") {
            let last = active
                .last_mut()
                .ok_or_else(|| format!("{}: #else без #ifdef", location()))?;
            *last = !*last;
        } else if directive.starts_with("#endif") {
            active
                .pop()
                .ok_or_else(|| format!("{}: #endif без #ifdef", location()))?;
        } else if !enabled {
            // Строка в выключенной ветке — пропускаем
        } else if let Some(rest) = directive.strip_prefix("#define ") {
            defines.insert(rest.trim().to_owned());
        } else if let Some(rest) = directive.strip_prefix("#include ") {
            let file = rest.trim().trim_matches('"');
            let path = if dir.is_empty() {
                file.to_owned()
            } else {
                format!("{dir}/{file}")
            };
            expand(&path, sources, defines, included, out)
                .map_err(|e| format!("{}: {e}", location()))?;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if active.is_empty() {
        Ok(())
    } else {
        Err(format!("{name}: не закрыт #ifdef"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(files: &[(&str, &str)]) -> HashMap<String, String> {
        files
            .iter()
            .map(|(n, s)| ((*n).to_owned(), (*s).to_owned()))
            .collect()
    }

    #[test]
    fn include_once_and_relative_paths() {
        let files = sources(&[
            (
                "shaders/main.wgsl",
                "#include \"common.wgsl\"\n#include \"common.wgsl\"\nmain",
            ),
            ("shaders/common.wgsl", "common"),
        ]);
        assert_eq!(
            preprocess("shaders/main.wgsl", &files, &[]).unwrap(),
            "common\nmain\n"
        );
    }

    #[test]
    fn conditions_and_defines() {
        let files = sources(&[(
            "a.wgsl",
            "#ifdef SKINNED\nskin\n#else\nstatic\n#endif\n#define X\n#ifndef X\nnever\n#endif\nend",
        )]);
        assert_eq!(
            preprocess("a.wgsl", &files, &["SKINNED"]).unwrap(),
            "skin\nend\n"
        );
        assert_eq!(preprocess("a.wgsl", &files, &[]).unwrap(), "static\nend\n");
    }

    #[test]
    fn errors_point_to_the_place() {
        let files = sources(&[
            ("a.wgsl", "#include \"missing.wgsl\""),
            ("b.wgsl", "#ifdef X\n"),
        ]);
        let error = preprocess("a.wgsl", &files, &[]).unwrap_err();
        assert!(
            error.contains("a.wgsl:1") && error.contains("missing.wgsl"),
            "{error}"
        );
        assert!(
            preprocess("b.wgsl", &files, &[])
                .unwrap_err()
                .contains("не закрыт")
        );
    }
}
