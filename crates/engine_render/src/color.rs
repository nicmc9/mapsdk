/// Цвет в линейном пространстве, компоненты 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    // [Rust] `const fn` можно вызывать при компиляции — например, в объявлении констант
    // выше. Аналог `constexpr`.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Цвет из оттенка (0..1, по кругу), насыщенность и яркость — максимальные.
    #[must_use]
    pub fn hue(hue: f32) -> Self {
        // Круг оттенков делится на 6 секторов, в каждом один канал растёт или убывает
        let sector = hue.rem_euclid(1.0) * 6.0;
        let ramp = 1.0 - (sector % 2.0 - 1.0).abs();
        // [Rust] `match` с условиями (guard) — аналог цепочки if/else if, но компилятор
        // проверяет, что все варианты покрыты (последняя ветка `_`).
        let (red, green, blue) = match sector {
            s if s < 1.0 => (1.0, ramp, 0.0),
            s if s < 2.0 => (ramp, 1.0, 0.0),
            s if s < 3.0 => (0.0, 1.0, ramp),
            s if s < 4.0 => (0.0, ramp, 1.0),
            s if s < 5.0 => (ramp, 0.0, 1.0),
            _ => (1.0, 0.0, ramp),
        };
        Self::rgb(red, green, blue)
    }

    /// Линейная интерполяция: `t = 0` — `self`, `t = 1` — `other`.
    // [Rust] `#[must_use]` — предупреждение, если результат вызова проигнорирован:
    // `color.lerp(..)` ничего не меняет в `color`, а возвращает новый цвет.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self {
            r: mix(self.r, other.r),
            g: mix(self.g, other.g),
            b: mix(self.b, other.b),
            a: mix(self.a, other.a),
        }
    }

    pub(crate) fn to_wgpu(self) -> wgpu::Color {
        wgpu::Color {
            r: f64::from(self.r),
            g: f64::from(self.g),
            b: f64::from(self.b),
            a: f64::from(self.a),
        }
    }
}
