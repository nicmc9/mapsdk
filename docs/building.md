# Сборка и запуск

Версия Rust закреплена в `rust-toolchain.toml`: при первом вызове `cargo` rustup сам скачает
компилятор и стандартные библиотеки для всех платформ.

## Один раз

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # если Rust ещё не установлен
cargo install cargo-ndk --version 4.1.2 --locked                 # Android
cargo install wasm-bindgen-cli --version 0.2.130 --locked        # Web (версия = Cargo.lock)
```

Android: Android SDK (build-tools, platform 34) и NDK. По умолчанию ищутся в `~/Library/Android/sdk`,
иначе задайте `ANDROID_HOME` / `ANDROID_NDK_HOME`. iOS: Xcode.

## Платформы

| Платформа | Команда | Логи |
|-----------|---------|------|
| macOS / Linux | `cargo run -p sandbox` | терминал, уровень — `RUST_LOG=debug` |
| Android | `./platforms/android/build.sh --run` | `adb logcat -s engine` |
| iOS-симулятор | `./platforms/ios/run_sim.sh` | терминал |
| Web | `./platforms/web/build.sh --serve` → http://localhost:8000 | консоль браузера |

### Настройки (CVar)

Значения по умолчанию — `apps/sandbox/sandbox.toml`. Переопределить при запуске:

```sh
cargo run -p sandbox -- --cvar sim.hz=30 --cvar render.mode=inline --cvar log.filter=info,engine_render=debug
```

На Web — параметрами URL: `http://localhost:8000/?sim.hz=30&render.mode=inline`.

Сцена: `--cvar sandbox.scene=sponza` (`showcase`, `sponza`, `helmet`).
Управление камерой:
- мышь: левая кнопка — вращение, колесо — приближение;
- тачпад: два пальца — вращение, щипок — приближение;
- сенсорный экран: палец — вращение, щипок — приближение;
- клавиши Q/E — приближение.

Отладка: `` ` `` или F4 — интерфейс (статистика, редактор CVar), F1 — сетка, F2 — границы
мешей, F3 — оси. Запуск сразу с интерфейсом: `cargo run -p sandbox -- --cvar debug.ui=true`.
На Mac с ISO-клавиатурой macOS меняет местами коды клавиш `` ` `` и `§` — проще нажать F4
(на ноутбуке — с `fn`).

### Скриншот

```sh
cargo run -p sandbox -- --cvar render.screenshot=shot.png --cvar render.screenshot_frame=240 --cvar render.screenshot_exit=true
```

### Настройки на устройстве (Android, iOS)

Аргументов запуска на мобильных нет — значения берутся из `engine.toml` в каталоге
приложения (важнее встроенного `sandbox.toml`). Android, отладочная сборка:

```sh
adb shell run-as com.mapsdk.sandbox sh -c 'echo render.backend = \"gl\" > files/engine.toml'
```

**Эмулятор Android:** на Vulkan эмулятора (SwiftShader через gfxstream) создание PBR-пайплайна
зависает. Используйте `render.backend = "gl"`. На реальных устройствах Vulkan не проверен.

### Профилирование

```sh
cargo run -p sandbox --features engine/profiling
```

Затем запустите [Tracy](https://github.com/wolfpld/tracy) **версии 0.14.x** и нажмите Connect.
Версия GUI должна совпадать с клиентом внутри `tracy-client` 0.19 (Tracy 0.14.1), иначе
подключение не установится. Homebrew может отдавать другую версию — проверьте `brew info tracy`. Пока Tracy не подключён, данные не собираются.

Web требует браузер с WebGPU (решение D3: без WebGL2).
iOS на устройстве требует подписи сертификатом — будет через Xcode-проект позже.

## Ресурсы

Папка `assets/` в корне репозитория. Модели Khronos: `./assets/fetch_khronos.sh`
(Sponza в git не хранится — только через скрипт). Скрипты сборки сами кладут `assets`
в APK, в `.app` и рядом со страницей Web.

## Проверки, как в CI

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo ndk -t arm64-v8a clippy --workspace --all-targets -- -D warnings   # нужен компилятор NDK
cargo clippy --workspace --all-targets --target aarch64-apple-ios -- -D warnings
cargo clippy --workspace --lib --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace
cargo test -p engine_net -- --ignored   # реальная сеть
cargo test -p engine_render --test golden   # сравнение картинок (без GPU — пропуск)
cargo bench -p engine_math -p engine_core -p engine_scene   # бенчмарки (release)
```

Под Linux локально на Mac не проверить: `ring` (TLS) содержит C-код, нужен компилятор
для Linux. Эту сборку проверяет CI.

## Структура репозитория

```
Cargo.toml            workspace: список крейтов, версии зависимостей, правила lint
rust-toolchain.toml   версия Rust и платформы
crates/               крейты движка (engine — фасад, engine_* — подсистемы)
apps/sandbox/         тестовое приложение
platforms/            обвязка под платформы: манифест Android, Info.plist iOS, страница Web
docs/                 документация
.github/workflows/    CI
```
