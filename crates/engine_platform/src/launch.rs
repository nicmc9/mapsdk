/// Данные, которые ОС передаёт приложению при запуске.
///
/// На большинстве платформ пусто. На Android здесь Activity — без неё
/// невозможно создать окно и цикл событий.
// [Rust] `cfg_attr` применяет атрибут только при условии: Default есть везде, кроме Android,
// где пустой LaunchContext создать нельзя — нужен объект от системы.
#[cfg_attr(not(target_os = "android"), derive(Default))]
#[derive(Debug)]
pub struct LaunchContext {
    #[cfg(target_os = "android")]
    android_app: crate::AndroidApp,
}

#[cfg(target_os = "android")]
impl LaunchContext {
    pub fn android(app: crate::AndroidApp) -> Self {
        // Сразу делаем Activity доступной: пути приложения нужны ещё до запуска цикла
        // событий (конфигурация читается в App::new)
        crate::android::set_app(&app);
        Self { android_app: app }
    }

    // [Rust] `self` без `&` забирает объект целиком (перемещение). Похоже на метод
    // с квалификатором `&&` в C++, но обращение к объекту после вызова — ошибка компиляции.
    pub(crate) fn into_android_app(self) -> crate::AndroidApp {
        self.android_app
    }
}
