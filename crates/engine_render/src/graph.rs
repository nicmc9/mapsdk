//! Render graph (R2-15): кадр описывается проходами, которые читают и пишут ресурсы.
//!
//! Граф по зависимостям:
//! - упорядочивает проходы (тот, кто пишет ресурс, — раньше того, кто его читает);
//! - отбрасывает проходы, результат которых не доходит до итоговой цели;
//! - выделяет временные текстуры из пула и переиспользует их между кадрами.
//!
//! Барьеры (переходы состояний ресурсов) `wgpu` расставляет сам — графу о них думать не нужно.
//!
//! Исполнение — по виду прохода (`PassKind`), а не через замыкания: замыкания, захватившие
//! `&mut` разных частей рендера, не уживутся с borrow checker. [Rust] В C++ здесь обычно
//! хранят `std::function` с захватом `this`; в Rust тот же результат даёт `match` по enum.

use std::collections::HashMap;

/// Идентификатор ресурса в графе текущего кадра.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ResourceId(usize);

/// Вид прохода — по нему рендер выбирает, что выполнить.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PassKind {
    Scene,
    Ui,
}

/// Описание временной текстуры. Размер — как у цели кадра.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TextureDesc {
    pub format: wgpu::TextureFormat,
    pub usage: wgpu::TextureUsages,
}

#[derive(Debug)]
enum Resource {
    /// Внешний ресурс: кадр окна или offscreen-текстура. Итоговая цель кадра.
    Imported { name: &'static str },
    Transient {
        name: &'static str,
        desc: TextureDesc,
    },
}

#[derive(Debug)]
struct Pass {
    kind: PassKind,
    reads: Vec<ResourceId>,
    writes: Vec<ResourceId>,
}

/// Граф кадра. Строится заново каждый кадр — это дёшево: проходов единицы.
#[derive(Debug, Default)]
pub(crate) struct RenderGraph {
    resources: Vec<Resource>,
    passes: Vec<Pass>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum GraphError {
    #[error("цикл зависимостей между проходами")]
    Cycle,
}

impl RenderGraph {
    pub fn import(&mut self, name: &'static str) -> ResourceId {
        self.resources.push(Resource::Imported { name });
        ResourceId(self.resources.len() - 1)
    }

    pub fn transient(&mut self, name: &'static str, desc: TextureDesc) -> ResourceId {
        self.resources.push(Resource::Transient { name, desc });
        ResourceId(self.resources.len() - 1)
    }

    pub fn add_pass(&mut self, kind: PassKind, reads: &[ResourceId], writes: &[ResourceId]) {
        self.passes.push(Pass {
            kind,
            reads: reads.to_vec(),
            writes: writes.to_vec(),
        });
    }

    #[cfg_attr(not(test), allow(dead_code))] // для отладки и тестов
    pub fn name(&self, id: ResourceId) -> &'static str {
        match &self.resources[id.0] {
            Resource::Imported { name } | Resource::Transient { name, .. } => name,
        }
    }

    /// Порядок выполнения проходов, нужных для получения `output`.
    pub fn compile(&self, output: ResourceId) -> Result<Vec<PassKind>, GraphError> {
        // 1. Отбор: идём от итоговой цели назад — какие проходы на неё работают
        let mut needed = vec![false; self.passes.len()];
        let mut wanted = vec![output];
        let mut seen = std::collections::HashSet::new();
        while let Some(resource) = wanted.pop() {
            if !seen.insert(resource) {
                continue;
            }
            for (index, pass) in self.passes.iter().enumerate() {
                if pass.writes.contains(&resource) && !needed[index] {
                    needed[index] = true;
                    wanted.extend(&pass.reads);
                    // Проход, дописывающий в ресурс (load + store), зависит и от прошлых записей
                    wanted.extend(&pass.writes);
                }
            }
        }

        // 2. Порядок. B зависит от A, если:
        //    - B читает ресурс, который пишет A (в любом порядке добавления);
        //    - оба пишут один ресурс без чтения — тогда решает порядок добавления
        //      (UI дорисовывает поверх сцены — добавляется после неё).
        let indices: Vec<usize> = (0..self.passes.len()).filter(|&i| needed[i]).collect();
        let mut depends: HashMap<usize, Vec<usize>> = HashMap::new();
        for &b in &indices {
            for &a in &indices {
                if a == b {
                    continue;
                }
                let (pa, pb) = (&self.passes[a], &self.passes[b]);
                let reads_written = pb.reads.iter().any(|r| pa.writes.contains(r));
                let both_write = a < b && pb.writes.iter().any(|r| pa.writes.contains(r));
                if reads_written || both_write {
                    depends.entry(b).or_default().push(a);
                }
            }
        }
        let mut order = Vec::with_capacity(indices.len());
        let mut done = vec![false; self.passes.len()];
        while order.len() < indices.len() {
            let ready = indices.iter().copied().find(|&i| {
                !done[i]
                    && depends
                        .get(&i)
                        .is_none_or(|deps| deps.iter().all(|&d| done[d]))
            });
            let Some(index) = ready else {
                return Err(GraphError::Cycle);
            };
            done[index] = true;
            order.push(index);
        }
        Ok(order.into_iter().map(|i| self.passes[i].kind).collect())
    }

    /// Временные ресурсы: id, имя (метка текстуры в отладчиках GPU) и описание.
    pub fn transients(&self) -> impl Iterator<Item = (ResourceId, &'static str, TextureDesc)> + '_ {
        self.resources
            .iter()
            .enumerate()
            .filter_map(|(index, resource)| match resource {
                Resource::Transient { name, desc } => Some((ResourceId(index), *name, *desc)),
                Resource::Imported { .. } => None,
            })
    }
}

/// Пул временных текстур: текстура нужного формата и размера переиспользуется
/// из кадра в кадр, при смене размера цели — пересоздаётся.
#[derive(Default)]
pub(crate) struct TexturePool {
    textures: HashMap<(TextureDesc, u32, u32), wgpu::TextureView>,
}

impl TexturePool {
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        name: &'static str,
        desc: TextureDesc,
        size: (u32, u32),
    ) -> wgpu::TextureView {
        self.textures
            .entry((desc, size.0, size.1))
            .or_insert_with(|| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(name),
                        size: wgpu::Extent3d {
                            width: size.0,
                            height: size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: desc.format,
                        usage: desc.usage,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            })
            .clone()
    }

    /// Убрать текстуры, не подходящие под новый размер цели.
    pub fn retain_size(&mut self, size: (u32, u32)) {
        self.textures.retain(|(_, w, h), _| (*w, *h) == size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPTH: TextureDesc = TextureDesc {
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    };

    #[test]
    fn orders_by_dependencies_and_culls_unused() {
        let mut graph = RenderGraph::default();
        let target = graph.import("target");
        let depth = graph.transient("depth", DEPTH);
        let unused = graph.transient("unused", DEPTH);
        // Добавлены «не по порядку»: UI раньше сцены
        graph.add_pass(PassKind::Ui, &[], &[target]);
        graph.add_pass(PassKind::Scene, &[], &[target, depth]);
        // Проход, результат которого никуда не идёт, — выброшен
        graph.add_pass(PassKind::Scene, &[], &[unused]);
        let order = graph.compile(target).unwrap();
        // Оба пишут в target без чтения: решает порядок добавления
        assert_eq!(order, [PassKind::Ui, PassKind::Scene]);
    }

    #[test]
    fn reader_runs_after_writer() {
        let mut graph = RenderGraph::default();
        let target = graph.import("target");
        let shadow = graph.transient("shadow", DEPTH);
        graph.add_pass(PassKind::Scene, &[shadow], &[target]); // читает тень
        graph.add_pass(PassKind::Ui, &[], &[shadow]); // пишет тень (условно)
        let order = graph.compile(target).unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], PassKind::Ui, "запись тени — до её чтения");
    }

    #[test]
    fn cycle_is_an_error() {
        let mut graph = RenderGraph::default();
        let target = graph.import("target");
        let a = graph.transient("a", DEPTH);
        let b = graph.transient("b", DEPTH);
        graph.add_pass(PassKind::Scene, &[a], &[b, target]);
        graph.add_pass(PassKind::Ui, &[b], &[a]);
        assert_eq!(graph.compile(target), Err(GraphError::Cycle));
    }

    #[test]
    fn transient_names_and_list() {
        let mut graph = RenderGraph::default();
        let target = graph.import("target");
        let depth = graph.transient("depth", DEPTH);
        assert_eq!(graph.name(target), "target");
        assert_eq!(graph.name(depth), "depth");
        assert_eq!(graph.transients().count(), 1);
    }
}
