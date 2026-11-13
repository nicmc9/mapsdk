//! Анимационный клип glTF: ключевые кадры позиции, поворота и масштаба узлов.

use engine_math::{Quat, Vec3};

use crate::Asset;

/// Как значения меняются между ключевыми кадрами.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    /// Скачком в момент ключа.
    Step,
    /// Линейно (повороты — сферически, slerp).
    Linear,
    /// Кубический сплайн Эрмита: у каждого ключа есть входящая и исходящая касательные.
    CubicSpline,
}

/// Значения ключей. Для `CubicSpline` на каждый ключ три значения: касательная входа,
/// значение, касательная выхода.
#[derive(Debug, Clone)]
pub enum Keyframes {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

/// Значение канала в момент времени.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sample {
    Translation(Vec3),
    Rotation(Quat),
    Scale(Vec3),
}

/// Анимация одного свойства одного узла.
#[derive(Debug, Clone)]
pub struct Channel {
    /// Индекс узла в `Gltf::nodes`.
    pub node: usize,
    /// Время ключей, секунды, по возрастанию.
    pub times: Vec<f32>,
    pub keyframes: Keyframes,
    pub interpolation: Interpolation,
}

impl Channel {
    /// Значение в момент `time` (за границами — крайний ключ).
    pub fn sample(&self, time: f32) -> Option<Sample> {
        let (index, t, dt) = self.locate(time)?;
        Some(match &self.keyframes {
            Keyframes::Translation(v) => Sample::Translation(self.sample_vec3(v, index, t, dt)),
            Keyframes::Scale(v) => Sample::Scale(self.sample_vec3(v, index, t, dt)),
            Keyframes::Rotation(v) => Sample::Rotation(self.sample_quat(v, index, t, dt)),
        })
    }

    /// Индекс ключа слева, доля пути до следующего (0..1) и длина интервала.
    fn locate(&self, time: f32) -> Option<(usize, f32, f32)> {
        let last = self.times.len().checked_sub(1)?;
        if time <= self.times[0] || last == 0 {
            return Some((0, 0.0, 0.0));
        }
        if time >= self.times[last] {
            return Some((last, 0.0, 0.0));
        }
        // [Rust] partition_point — двоичный поиск первого элемента, для которого условие ложно
        let next = self.times.partition_point(|&t| t <= time);
        let index = next - 1;
        let dt = self.times[next] - self.times[index];
        let t = if dt > 0.0 {
            (time - self.times[index]) / dt
        } else {
            0.0
        };
        Some((index, t, dt))
    }

    fn sample_vec3(&self, values: &[Vec3], index: usize, t: f32, dt: f32) -> Vec3 {
        let last = self.times.len() - 1;
        match self.interpolation {
            Interpolation::Step => values[index],
            Interpolation::Linear => {
                if index >= last {
                    values[last]
                } else {
                    values[index].lerp(values[index + 1], t)
                }
            }
            Interpolation::CubicSpline => {
                // Ключ i: [касательная входа, значение, касательная выхода]
                let value = |i: usize| values[i * 3 + 1];
                if index >= last {
                    return value(last);
                }
                let (p0, m0) = (value(index), values[index * 3 + 2] * dt);
                let (p1, m1) = (value(index + 1), values[(index + 1) * 3] * dt);
                hermite(p0, m0, p1, m1, t)
            }
        }
    }

    fn sample_quat(&self, values: &[Quat], index: usize, t: f32, dt: f32) -> Quat {
        let last = self.times.len() - 1;
        match self.interpolation {
            Interpolation::Step => values[index],
            Interpolation::Linear => {
                if index >= last {
                    values[last]
                } else {
                    values[index].slerp(values[index + 1], t)
                }
            }
            Interpolation::CubicSpline => {
                let value = |i: usize| values[i * 3 + 1];
                if index >= last {
                    return value(last).normalize();
                }
                // Эрмит покомпонентно с последующей нормализацией — как в спецификации glTF
                let as_vec = |q: Quat| engine_math::Vec4::from(q);
                let p0 = as_vec(value(index));
                let m0 = as_vec(values[index * 3 + 2]) * dt;
                let p1 = as_vec(value(index + 1));
                let m1 = as_vec(values[(index + 1) * 3]) * dt;
                let (t2, t3) = (t * t, t * t * t);
                let v = p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
                    + m0 * (t3 - 2.0 * t2 + t)
                    + p1 * (-2.0 * t3 + 3.0 * t2)
                    + m1 * (t3 - t2);
                Quat::from_vec4(v).normalize()
            }
        }
    }
}

fn hermite(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

/// Анимационный клип. Вложенный ресурс glTF: `файл.glb#Animation0`.
#[derive(Debug, Clone)]
pub struct AnimationClip {
    pub name: String,
    /// Длительность, секунды (время последнего ключа).
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Asset for AnimationClip {}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(interpolation: Interpolation, values: Vec<Vec3>) -> Channel {
        Channel {
            node: 0,
            times: vec![0.0, 1.0, 3.0],
            keyframes: Keyframes::Translation(values),
            interpolation,
        }
    }

    #[test]
    fn linear_step_and_clamping() {
        let c = channel(
            Interpolation::Linear,
            vec![Vec3::ZERO, Vec3::X, Vec3::X * 3.0],
        );
        assert_eq!(c.sample(0.5), Some(Sample::Translation(Vec3::X * 0.5)));
        assert_eq!(c.sample(2.0), Some(Sample::Translation(Vec3::X * 2.0)));
        assert_eq!(
            c.sample(-1.0),
            Some(Sample::Translation(Vec3::ZERO)),
            "до начала — первый ключ"
        );
        assert_eq!(
            c.sample(9.0),
            Some(Sample::Translation(Vec3::X * 3.0)),
            "после конца — последний"
        );
        let s = channel(Interpolation::Step, vec![Vec3::ZERO, Vec3::X, Vec3::Y]);
        assert_eq!(s.sample(0.99), Some(Sample::Translation(Vec3::ZERO)));
    }

    #[test]
    fn cubic_spline_passes_through_keys() {
        // Нулевые касательные: на ключах — ровно значения, посередине — среднее
        let zero = Vec3::ZERO;
        let c = channel(
            Interpolation::CubicSpline,
            vec![
                zero,
                Vec3::ZERO,
                zero,
                zero,
                Vec3::X,
                zero,
                zero,
                Vec3::Y,
                zero,
            ],
        );
        assert_eq!(c.sample(1.0), Some(Sample::Translation(Vec3::X)));
        let Some(Sample::Translation(mid)) = c.sample(0.5) else {
            panic!()
        };
        assert!((mid - Vec3::X * 0.5).length() < 1e-5);
    }
}
