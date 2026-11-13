use std::collections::HashSet;

use engine_ecs::prelude::*;
use engine_ecs::{Schedule, ScheduleSystem, Schedules};
use engine_platform::{Instant, LaunchContext, PlatformConfig, PlatformError};

use crate::window::MainWindow;
use crate::{AppLifecycle, FixedTime, Plugin, Stage, Time, Window, WindowResized};

/// Приложение движка: мир ECS, расписания стадий и подключённые плагины.
///
/// ```ignore
/// let mut app = App::new();
/// app.add_plugin(MyPlugin)
///    .add_systems(Stage::FixedUpdate, move_objects);
/// app.run(PlatformConfig::default(), LaunchContext::default())?;
/// ```
// [Rust] Методы настройки принимают и возвращают `&mut Self`, поэтому вызовы
// можно выстраивать цепочкой, как в C++ builder с `return *this;`.
pub struct App {
    world: World,
    // [Rust] `Box<dyn Plugin>` — владеющий указатель на объект любого типа с трейтом Plugin.
    // Аналог `std::unique_ptr<IPlugin>` с виртуальными вызовами.
    plugins: Vec<Box<dyn Plugin>>,
    plugin_names: HashSet<&'static str>,
    started: bool,
}

impl App {
    pub fn new() -> Self {
        let mut world = World::new();
        for stage in Stage::ALL {
            world.add_schedule(Schedule::new(stage));
        }
        world.insert_resource(Time::new(Instant::now()));
        world.init_resource::<FixedTime>();
        world.init_resource::<Window>();

        let mut app = Self {
            world,
            plugins: Vec::new(),
            plugin_names: HashSet::new(),
            started: false,
        };
        app.add_systems(Stage::First, engine_ecs::message_update_system);
        app.add_message::<WindowResized>()
            .add_message::<AppLifecycle>();
        app
    }

    /// Подключает плагин: сразу вызывает его `build`. Повторное добавление — ошибка в коде.
    // [Rust] `impl Plugin` в аргументе — принимается любой тип с трейтом Plugin
    // (аналог шаблонного параметра `template<class P> requires Plugin<P>`).
    pub fn add_plugin(&mut self, plugin: impl Plugin) -> &mut Self {
        let name = plugin.name();
        assert!(self.plugin_names.insert(name), "плагин {name} уже добавлен");
        log::debug!("Плагин: {name}");
        plugin.build(self);
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Проверка зависимостей: плагин может потребовать, чтобы другой был добавлен раньше.
    pub fn is_plugin_added<P: Plugin>(&self) -> bool {
        self.plugin_names.contains(std::any::type_name::<P>())
    }

    /// Добавляет системы в стадию.
    ///
    /// ```ignore
    /// app.add_systems(Stage::Update, (read_input, move_camera.after(read_input)));
    /// ```
    pub fn add_systems<M>(
        &mut self,
        stage: Stage,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> &mut Self {
        self.world
            .resource_mut::<Schedules>()
            .add_systems(stage, systems);
        self
    }

    /// Регистрирует тип сообщения для `MessageWriter`/`MessageReader`.
    pub fn add_message<M: Message>(&mut self) -> &mut Self {
        engine_ecs::register_message::<M>(&mut self.world);
        self
    }

    pub fn insert_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.world.insert_resource(resource);
        self
    }

    pub fn init_resource<R: Resource + Default>(&mut self) -> &mut Self {
        self.world.init_resource::<R>();
        self
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Выполняет один кадр. Обычно вызывается циклом платформы, вручную — в тестах.
    pub fn update(&mut self) {
        self.update_at(Instant::now());
    }

    /// Кадр с заданным временем — для детерминированных тестов.
    pub fn update_at(&mut self, now: Instant) {
        if !self.started {
            self.startup();
        }

        self.world.resource_mut::<Time>().update(now);
        let delta = self.world.resource::<Time>().delta();
        let steps = self.world.resource_mut::<FixedTime>().accumulate(delta);

        for stage in Stage::PER_FRAME_BEFORE_FIXED {
            self.world.run_schedule(stage);
        }
        for _ in 0..steps {
            // Сначала сдвигаем время, чтобы системы шага видели время этого шага
            self.world.resource_mut::<FixedTime>().consume_step();
            self.world.run_schedule(Stage::FixedUpdate);
        }
        for stage in Stage::PER_FRAME_AFTER_FIXED {
            self.world.run_schedule(stage);
        }
        // Сброс флагов Added/Changed для следующего кадра
        self.world.clear_trackers();
    }

    /// Когда нужен следующий кадр: к моменту очередного шага логики.
    /// `now` — время, с которым выполнялся последний кадр.
    pub(crate) fn next_update_time(&self, now: Instant) -> Instant {
        now + self.world.resource::<FixedTime>().until_next_step()
    }

    /// Запускает цикл событий платформы. На iOS не возвращает управление, на Web
    /// возвращает сразу (цикл продолжается в браузере).
    pub fn run(
        &mut self,
        config: PlatformConfig,
        launch: LaunchContext,
    ) -> Result<(), PlatformError> {
        // [Rust] `mem::take` забирает App из `self`, оставляя на его месте новый пустой
        // (через Default). Нужно, потому что цикл событий должен владеть App целиком.
        let app = std::mem::take(self);
        engine_platform::run(config, launch, crate::runner::AppRunner::new(app))
    }

    pub(crate) fn set_main_window(&mut self, window: MainWindow) {
        self.world.insert_resource(window);
    }

    fn startup(&mut self) {
        self.started = true;
        let plugins = std::mem::take(&mut self.plugins);
        for plugin in &plugins {
            plugin.finish(self);
        }
        self.plugins = plugins;
        self.world.run_schedule(Stage::Startup);
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

// Ручная реализация: World печатать целиком бессмысленно
impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("plugins", &self.plugin_names)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_platform::Duration;

    #[derive(Resource, Default)]
    struct Log(Vec<&'static str>);

    fn first(mut log: ResMut<Log>) {
        log.0.push("first");
    }
    fn fixed(mut log: ResMut<Log>) {
        log.0.push("fixed");
    }
    fn update(mut log: ResMut<Log>) {
        log.0.push("update");
    }

    #[test]
    fn stages_run_in_order_with_fixed_steps() {
        let mut app = App::new();
        app.init_resource::<Log>()
            .add_systems(Stage::PreUpdate, first)
            .add_systems(Stage::FixedUpdate, fixed)
            .add_systems(Stage::Update, update);

        let start = Instant::now();
        app.update_at(start); // первый кадр: delta = 0, шагов нет
        let step = app.world().resource::<FixedTime>().step();
        app.update_at(start + step * 2); // ровно два шага

        assert_eq!(
            app.world().resource::<Log>().0,
            ["first", "update", "first", "fixed", "fixed", "update"]
        );
    }

    struct TestPlugin;
    impl Plugin for TestPlugin {
        fn build(&self, app: &mut App) {
            app.init_resource::<Log>();
        }
        fn finish(&self, app: &mut App) {
            app.world_mut().resource_mut::<Log>().0.push("finish");
        }
    }

    #[test]
    fn plugin_build_and_finish() {
        let mut app = App::new();
        app.add_plugin(TestPlugin);
        assert!(app.is_plugin_added::<TestPlugin>());
        app.update_at(Instant::now());
        assert_eq!(app.world().resource::<Log>().0, ["finish"]);
    }

    #[test]
    #[should_panic(expected = "уже добавлен")]
    fn duplicate_plugin_panics() {
        App::new().add_plugin(TestPlugin).add_plugin(TestPlugin);
    }

    #[derive(Message)]
    struct Ping;

    #[derive(Resource, Default)]
    struct Received(usize);

    fn send(mut writer: MessageWriter<Ping>) {
        writer.write(Ping);
    }
    fn receive(mut reader: MessageReader<Ping>, mut received: ResMut<Received>) {
        received.0 += reader.read().count();
    }

    #[test]
    fn messages_are_delivered_once() {
        let mut app = App::new();
        app.add_message::<Ping>()
            .init_resource::<Received>()
            .add_systems(Stage::Update, (send, receive.after(send)));
        let start = Instant::now();
        for frame in 0..3 {
            app.update_at(start + Duration::from_millis(frame));
        }
        // Три кадра — три сообщения, каждое прочитано ровно один раз
        assert_eq!(app.world().resource::<Received>().0, 3);
    }
}
