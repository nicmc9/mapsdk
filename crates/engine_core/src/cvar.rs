//! Переменные конфигурации (CVar) — как в id Tech и Unreal.
//!
//! ```ignore
//! // Объявление — глобальная константа рядом с кодом, который её использует
//! pub static SIM_HZ: CVar<f64> = CVar::new("sim.hz", 60.0, "Частота шага логики, Гц")
//!     .with_flags(CVarFlags::STARTUP);
//!
//! // Регистрация — в Plugin::build
//! app.register_cvar(&SIM_HZ);
//!
//! // Чтение — из любого потока
//! let hz = SIM_HZ.get();
//! ```
//!
//! Источники значений по возрастанию приоритета: значение по умолчанию → файл TOML →
//! аргументы запуска (`--cvar sim.hz=30` или `?sim.hz=30` на Web) → консоль.
//!
//! [Rust] В Rust нет статических конструкторов: код до `main` не выполняется, поэтому
//! переменная не может зарегистрироваться сама, как глобальный объект в C++.
//! Регистрация всегда явная.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};

use engine_ecs::prelude::*;

/// Строковое значение CVar. [Rust] `Cow` («clone on write») — либо ссылка на строку
/// в бинарнике (`Cow::Borrowed`, годится для значения по умолчанию в `static`),
/// либо собственная строка (`Cow::Owned`, после изменения).
pub type CVarString = Cow<'static, str>;

/// Флаги поведения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CVarFlags(u8);

impl CVarFlags {
    pub const NONE: Self = Self(0);
    /// Читается только при запуске: из консоли после старта не меняется.
    pub const STARTUP: Self = Self(1);
    /// Меняется из консоли только при `cheats = true`.
    pub const CHEAT: Self = Self(1 << 1);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    // [Rust] `const fn` — можно вызывать при инициализации `static`.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Тип, который может храниться в CVar: умеет разбираться из строки и печататься в строку.
pub trait CVarValue: Clone + Send + Sync + 'static {
    /// Имя типа для справки в консоли.
    const TYPE_NAME: &'static str;
    fn parse(text: &str) -> Result<Self, String>;
    fn format(&self) -> String;
}

impl CVarValue for bool {
    const TYPE_NAME: &'static str = "bool";
    fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "on" | "yes" => Ok(true),
            "0" | "false" | "off" | "no" => Ok(false),
            other => Err(format!("ожидается true/false, получено «{other}»")),
        }
    }
    fn format(&self) -> String {
        self.to_string()
    }
}

// [Rust] Макрос генерирует одинаковые реализации для нескольких числовых типов —
// аналог шаблона с явными специализациями.
macro_rules! impl_cvar_number {
    ($($ty:ty),*) => {$(
        impl CVarValue for $ty {
            const TYPE_NAME: &'static str = stringify!($ty);
            fn parse(text: &str) -> Result<Self, String> {
                text.trim().parse().map_err(|e| format!("«{text}»: {e}"))
            }
            fn format(&self) -> String {
                self.to_string()
            }
        }
    )*};
}
impl_cvar_number!(i32, i64, u32, u64, f32, f64);

impl CVarValue for CVarString {
    const TYPE_NAME: &'static str = "string";
    fn parse(text: &str) -> Result<Self, String> {
        Ok(Cow::Owned(text.to_owned()))
    }
    fn format(&self) -> String {
        self.to_string()
    }
}

/// Переменная конфигурации. Объявляется как `static`, читается из любого потока.
#[derive(Debug)]
pub struct CVar<T: CVarValue> {
    name: &'static str,
    description: &'static str,
    flags: CVarFlags,
    default: T,
    // `None` — значение по умолчанию. RwLock: читают часто и из разных потоков, пишут редко.
    value: RwLock<Option<T>>,
    /// Растёт при каждом изменении — по нему системы замечают, что значение поменялось.
    version: AtomicU64,
}

impl<T: CVarValue> CVar<T> {
    pub const fn new(name: &'static str, default: T, description: &'static str) -> Self {
        Self {
            name,
            description,
            flags: CVarFlags::NONE,
            default,
            value: RwLock::new(None),
            version: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub const fn with_flags(mut self, flags: CVarFlags) -> Self {
        self.flags = self.flags.union(flags);
        self
    }

    /// Текущее значение (копия).
    pub fn get(&self) -> T {
        // [Rust] «Отравление» блокировки: если поток запаниковал, держа её, Rust помечает
        // блокировку как испорченную. Для простого значения это не опасно — берём как есть.
        let value = self.value.read().unwrap_or_else(PoisonError::into_inner);
        value.clone().unwrap_or_else(|| self.default.clone())
    }

    /// Устанавливает значение из кода (без проверок флагов — их делает [`CVars::set`]).
    pub fn set(&self, value: T) {
        *self.value.write().unwrap_or_else(PoisonError::into_inner) = Some(value);
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Возвращает значение по умолчанию.
    pub fn reset(&self) {
        *self.value.write().unwrap_or_else(PoisonError::into_inner) = None;
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Номер изменения. Сравните с сохранённым, чтобы узнать, менялось ли значение.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// CVar без указания типа — для реестра, консоли и загрузки из файлов.
// [Rust] Трейт-объект (`&dyn AnyCVar`) позволяет хранить переменные разных типов
// в одной коллекции — как указатель на базовый класс с виртуальными методами.
pub trait AnyCVar: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn flags(&self) -> CVarFlags;
    fn type_name(&self) -> &'static str;
    fn value_text(&self) -> String;
    fn default_text(&self) -> String;
    fn set_text(&self, text: &str) -> Result<(), String>;
    fn reset(&self);
}

impl<T: CVarValue> AnyCVar for CVar<T> {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        self.description
    }
    fn flags(&self) -> CVarFlags {
        self.flags
    }
    fn type_name(&self) -> &'static str {
        T::TYPE_NAME
    }
    fn value_text(&self) -> String {
        self.get().format()
    }
    fn default_text(&self) -> String {
        self.default.format()
    }
    fn set_text(&self, text: &str) -> Result<(), String> {
        self.set(T::parse(text)?);
        Ok(())
    }
    fn reset(&self) {
        CVar::reset(self);
    }
}

/// Откуда пришло значение. Порядок — приоритет (больше — важнее).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CVarSource {
    File,
    LaunchArguments,
    Console,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CVarError {
    #[error("неизвестная переменная «{0}»")]
    Unknown(String),
    #[error("«{0}» задаётся только при запуске")]
    StartupOnly(&'static str),
    #[error("«{0}» — чит-переменная, сначала включите cheats")]
    CheatsDisabled(&'static str),
    #[error("«{name}»: {reason}")]
    InvalidValue { name: &'static str, reason: String },
}

/// Разрешает менять чит-переменные из консоли.
pub static CHEATS: CVar<bool> = CVar::new("cheats", false, "Разрешить чит-переменные");

/// Реестр переменных приложения. Доступен системам как `Res<CVars>` / `ResMut<CVars>`.
#[derive(Resource, Default)]
pub struct CVars {
    vars: BTreeMap<&'static str, &'static dyn AnyCVar>,
    /// Значения для переменных, которые ещё не зарегистрированы (плагин добавят позже).
    pending: BTreeMap<String, (String, CVarSource)>,
    started: bool,
}

impl CVars {
    /// Регистрирует переменную и применяет к ней значение из файла/аргументов, если оно было.
    pub fn register(&mut self, cvar: &'static dyn AnyCVar) {
        let name = cvar.name();
        if let Some(existing) = self.vars.get(name) {
            // [Rust] Сравнение адресов: та же переменная, зарегистрированная повторно, — не ошибка
            assert!(
                std::ptr::addr_eq(*existing, cvar),
                "две разные CVar с одним именем «{name}»"
            );
            return;
        }
        self.vars.insert(name, cvar);
        if let Some((text, source)) = self.pending.remove(name)
            && let Err(error) = cvar.set_text(&text)
        {
            tracing::error!("CVar {name} из {source:?}: {error}");
        }
    }

    /// Устанавливает значение по имени с проверкой флагов.
    pub fn set(&mut self, name: &str, text: &str, source: CVarSource) -> Result<(), CVarError> {
        let Some(cvar) = self.vars.get(name).copied() else {
            if self.started {
                return Err(CVarError::Unknown(name.to_owned()));
            }
            // До старта переменную ещё может зарегистрировать плагин — запоминаем.
            // Источник с большим приоритетом перезаписывает меньший.
            let keep_existing = self
                .pending
                .get(name)
                .is_some_and(|(_, existing)| *existing > source);
            if !keep_existing {
                self.pending
                    .insert(name.to_owned(), (text.to_owned(), source));
            }
            return Ok(());
        };
        let flags = cvar.flags();
        if self.started && source == CVarSource::Console {
            if flags.contains(CVarFlags::STARTUP) {
                return Err(CVarError::StartupOnly(cvar.name()));
            }
            if flags.contains(CVarFlags::CHEAT) && !CHEATS.get() {
                return Err(CVarError::CheatsDisabled(cvar.name()));
            }
        }
        cvar.set_text(text)
            .map_err(|reason| CVarError::InvalidValue {
                name: cvar.name(),
                reason,
            })
    }

    /// Загружает значения из TOML. Таблицы превращаются в префиксы имён:
    /// `[sim] hz = 30` → `sim.hz = 30`.
    pub fn load_toml(&mut self, text: &str, source: CVarSource) -> Result<(), String> {
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let mut flat = Vec::new();
        flatten_toml("", &toml::Value::Table(table), &mut flat);
        for (name, value) in flat {
            if let Err(error) = self.set(&name, &value, source) {
                tracing::error!("{error}");
            }
        }
        Ok(())
    }

    /// Текстовое значение по имени.
    pub fn get_text(&self, name: &str) -> Option<String> {
        self.vars.get(name).map(|cvar| cvar.value_text())
    }

    /// Все переменные в алфавитном порядке.
    pub fn iter(&self) -> impl Iterator<Item = &'static dyn AnyCVar> + '_ {
        self.vars.values().copied()
    }

    /// Старт приложения: дальше незарегистрированные имена — ошибка.
    pub(crate) fn mark_started(&mut self) {
        self.started = true;
        for (name, (_, source)) in std::mem::take(&mut self.pending) {
            tracing::warn!("Неизвестная переменная «{name}» ({source:?}) — опечатка?");
        }
    }
}

impl std::fmt::Debug for CVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CVars")
            .field("vars", &self.vars.keys().collect::<Vec<_>>())
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

fn flatten_toml(prefix: &str, value: &toml::Value, out: &mut Vec<(String, String)>) {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_toml(&name, value, out);
            }
        }
        // Строки — без кавычек, остальное — как записано в TOML
        toml::Value::String(text) => out.push((prefix.to_owned(), text.clone())),
        other => out.push((prefix.to_owned(), other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [Rust] Каждый тест использует свои static-переменные: тесты выполняются
    // параллельно в разных потоках, общие глобальные значения сделали бы их нестабильными.

    #[test]
    fn typed_get_set_and_version() {
        static SPEED: CVar<f32> = CVar::new("test.speed", 1.5, "");
        assert!((SPEED.get() - 1.5).abs() < f32::EPSILON);
        let version = SPEED.version();
        SPEED.set(3.0);
        assert!((SPEED.get() - 3.0).abs() < f32::EPSILON);
        assert!(SPEED.version() > version);
        SPEED.reset();
        assert!((SPEED.get() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn pending_values_apply_on_register_with_priority() {
        static HZ: CVar<u32> = CVar::new("test.pending.hz", 60, "");
        let mut cvars = CVars::default();
        cvars
            .load_toml("[test.pending]\nhz = 30", CVarSource::File)
            .unwrap();
        cvars
            .set("test.pending.hz", "120", CVarSource::LaunchArguments)
            .unwrap();
        // Файл прочитан позже аргументов, но приоритет у аргументов
        cvars
            .load_toml("[test.pending]\nhz = 45", CVarSource::File)
            .unwrap();
        cvars.register(&HZ);
        assert_eq!(HZ.get(), 120);
    }

    #[test]
    fn startup_and_cheat_flags() {
        static MODE: CVar<CVarString> =
            CVar::new("test.flags.mode", Cow::Borrowed("a"), "").with_flags(CVarFlags::STARTUP);
        static GOD: CVar<bool> =
            CVar::new("test.flags.god", false, "").with_flags(CVarFlags::CHEAT);
        let mut cvars = CVars::default();
        cvars.register(&MODE);
        cvars.register(&GOD);
        cvars.mark_started();
        assert_eq!(
            cvars.set("test.flags.mode", "b", CVarSource::Console),
            Err(CVarError::StartupOnly("test.flags.mode"))
        );
        assert_eq!(
            cvars.set("test.flags.god", "true", CVarSource::Console),
            Err(CVarError::CheatsDisabled("test.flags.god"))
        );
        assert!(matches!(
            cvars.set("test.flags.nope", "1", CVarSource::Console),
            Err(CVarError::Unknown(_))
        ));
    }

    #[test]
    fn invalid_value_is_rejected() {
        static COUNT: CVar<i32> = CVar::new("test.invalid.count", 1, "");
        let mut cvars = CVars::default();
        cvars.register(&COUNT);
        assert!(matches!(
            cvars.set("test.invalid.count", "много", CVarSource::Console),
            Err(CVarError::InvalidValue { .. })
        ));
        assert_eq!(COUNT.get(), 1);
    }
}
