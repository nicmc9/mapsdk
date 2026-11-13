use crate::App;

/// Единица расширения движка. Подсистема регистрирует в `build` всё своё:
/// ресурсы, системы, сообщения, загрузчики.
///
/// ```ignore
/// pub struct AudioPlugin;
/// impl Plugin for AudioPlugin {
///     fn build(&self, app: &mut App) {
///         app.init_resource::<AudioMixer>()
///             .add_systems(Stage::Update, play_sounds);
///     }
/// }
/// ```
// [Rust] Трейт — не базовый класс: данных в нём нет, только методы. Методы с телом
// (`finish`, `name`) — реализация по умолчанию, её можно не переопределять.
pub trait Plugin: 'static {
    /// Регистрация подсистемы. Вызывается сразу при `App::add_plugin`.
    fn build(&self, app: &mut App);

    /// Вызывается после `build` всех плагинов, перед первым кадром.
    /// Для того, что зависит от других плагинов (например, их ресурсов).
    fn finish(&self, _app: &mut App) {}

    /// Имя для логов и проверки повторного добавления.
    fn name(&self) -> &'static str {
        // Имя типа, например "engine_audio::AudioPlugin"
        std::any::type_name::<Self>()
    }
}
