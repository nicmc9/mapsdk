#!/usr/bin/env bash
# Сборка .app для iOS-симулятора без Xcode-проекта и запуск с выводом логов в терминал.
#   ./platforms/ios/run_sim.sh                 — симулятор по умолчанию ("iPhone 15")
#   SIM_DEVICE="iPhone 15 Pro" ./platforms/ios/run_sim.sh
# Для запуска на устройстве нужна подпись сертификатом — это сделаем через Xcode-проект позже.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
IOS_DIR="$ROOT/platforms/ios"
APP="$IOS_DIR/build/Sandbox.app"
TARGET=aarch64-apple-ios-sim
DEVICE="${SIM_DEVICE:-iPhone 15}"

# 1. Исполняемый файл (на iOS winit сам вызывает UIApplicationMain из main)
cd "$ROOT"
cargo build -p sandbox --bin sandbox --target "$TARGET"

# 2. Бандл .app: исполняемый файл + Info.plist, ad-hoc подпись (симулятору достаточно)
rm -rf "$APP" && mkdir -p "$APP"
cp "$ROOT/target/$TARGET/debug/sandbox" "$APP/"
cp "$IOS_DIR/Info.plist" "$APP/"
codesign --force --sign - "$APP"

# 3. Запуск в симуляторе
xcrun simctl boot "$DEVICE" 2>/dev/null || true   # уже запущен — не ошибка
# Окно симулятора (необязательно: без него приложение тоже работает)
open -a "$(xcode-select -p)/Applications/Simulator.app" 2>/dev/null || true
xcrun simctl install "$DEVICE" "$APP"
# --console-pty: stdout/stderr приложения (наши логи) идут в этот терминал
xcrun simctl launch --console-pty --terminate-running-process "$DEVICE" com.mapsdk.sandbox
