#!/usr/bin/env bash
# Сборка APK без Gradle: cargo-ndk → aapt2 → zipalign → apksigner.
#   ./platforms/android/build.sh            — debug APK
#   ./platforms/android/build.sh --release  — release-сборка Rust (подпись всё равно debug)
#   ./platforms/android/build.sh --run      — собрать, установить и запустить на устройстве/эмуляторе
# Требуется: Android SDK (build-tools, platforms), NDK, cargo-ndk.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ANDROID_DIR="$ROOT/platforms/android"
OUT="$ANDROID_DIR/build"
PACKAGE=com.mapsdk.sandbox
ABI=arm64-v8a
API=34
CARGO_FLAGS=()
RUN=0

for arg in "$@"; do
  case "$arg" in
    --release) CARGO_FLAGS+=(--release) ;;
    --run) RUN=1 ;;
    *) echo "Неизвестный аргумент: $arg" >&2; exit 1 ;;
  esac
done

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
# Берём последние установленные версии build-tools и NDK
BUILD_TOOLS="$ANDROID_HOME/build-tools/$(ls "$ANDROID_HOME/build-tools" | sort -V | tail -1)"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/$(ls "$ANDROID_HOME/ndk" | sort -V | tail -1)}"
ANDROID_JAR="$ANDROID_HOME/platforms/android-$API/android.jar"
[[ -f "$ANDROID_JAR" ]] || { echo "Нет $ANDROID_JAR (установите platform android-$API)" >&2; exit 1; }

rm -rf "$OUT" && mkdir -p "$OUT"

# 1. Rust → lib/arm64-v8a/libsandbox.so
cd "$ROOT"
cargo ndk -t "$ABI" -P 26 -o "$OUT/lib" build -p sandbox --lib ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}

# 2. Манифест → пустой APK, затем добавляем нативные библиотеки
# -A: папка assets попадает в APK (читается через AAssetManager)
"$BUILD_TOOLS/aapt2" link -o "$OUT/unaligned.apk" --manifest "$ANDROID_DIR/AndroidManifest.xml" \
  -I "$ANDROID_JAR" -A "$ROOT/assets"
(cd "$OUT" && zip -qr unaligned.apk lib)

# 3. Выравнивание и подпись debug-ключом (создаётся при первом запуске)
"$BUILD_TOOLS/zipalign" -f 4 "$OUT/unaligned.apk" "$OUT/aligned.apk"
KEYSTORE="$HOME/.android/debug.keystore"
if [[ ! -f "$KEYSTORE" ]]; then
  mkdir -p "$(dirname "$KEYSTORE")"
  keytool -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android \
    -alias androiddebugkey -keyalg RSA -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
fi
"$BUILD_TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --out "$OUT/sandbox.apk" "$OUT/aligned.apk"
echo "Готово: $OUT/sandbox.apk"

if [[ $RUN -eq 1 ]]; then
  ADB="$ANDROID_HOME/platform-tools/adb"
  "$ADB" install -r "$OUT/sandbox.apk"
  "$ADB" shell am start -n "$PACKAGE/android.app.NativeActivity"
  echo "Логи: $ADB logcat -s engine"
fi
