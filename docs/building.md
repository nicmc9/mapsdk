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

### Профилирование

```sh
cargo run -p sandbox --features engine/profiling
```

Затем запустите [Tracy](https://github.com/wolfpld/tracy) **версии 0.14.x** и нажмите Connect.
Версия GUI должна совпадать с клиентом внутри `tracy-client` 0.19 (Tracy 0.14.1), иначе
подключение не установится. Homebrew может отдавать другую версию — проверьте `brew info tracy`. Пока Tracy не подключён, данные не собираются.

Web требует браузер с WebGPU (решение D3: без WebGL2).
iOS на устройстве требует подписи сертификатом — будет через Xcode-проект позже.

## Проверки, как в CI

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --target aarch64-linux-android -- -D warnings
cargo clippy --workspace --all-targets --target aarch64-apple-ios -- -D warnings
cargo clippy --workspace --lib --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace
```

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
