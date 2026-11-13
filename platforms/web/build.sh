#!/usr/bin/env bash
# Сборка под Web: wasm + JS-обвязка в platforms/web/pkg.
#   ./platforms/web/build.sh            — debug
#   ./platforms/web/build.sh --release  — release
#   ./platforms/web/build.sh --serve    — собрать и запустить сервер на http://localhost:8000
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WEB="$ROOT/platforms/web"
TARGET=wasm32-unknown-unknown
PROFILE=debug
CARGO_FLAGS=()
SERVE=0

for arg in "$@"; do
  case "$arg" in
    --release) PROFILE=release; CARGO_FLAGS+=(--release) ;;
    --serve) SERVE=1 ;;
    *) echo "Неизвестный аргумент: $arg" >&2; exit 1 ;;
  esac
done

# Версия CLI wasm-bindgen обязана совпадать с версией крейта в Cargo.lock.
REQUIRED=$(grep -A1 '^name = "wasm-bindgen"$' "$ROOT/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')
INSTALLED=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [[ "$INSTALLED" != "$REQUIRED" ]]; then
  echo "Нужен wasm-bindgen-cli $REQUIRED (установлен: ${INSTALLED:-нет}). Установка:" >&2
  echo "  cargo install wasm-bindgen-cli --version $REQUIRED --locked" >&2
  exit 1
fi

cd "$ROOT"
cargo build -p sandbox --lib --target "$TARGET" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
wasm-bindgen --target web --no-typescript \
  --out-dir "$WEB/pkg" "$ROOT/target/$TARGET/$PROFILE/sandbox.wasm"

echo "Готово: $WEB/pkg"
if [[ $SERVE -eq 1 ]]; then
  echo "Открой http://localhost:8000"
  python3 -m http.server 8000 --directory "$WEB"
fi
