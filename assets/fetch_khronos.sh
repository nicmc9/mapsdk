#!/usr/bin/env bash
# Скачивает тестовые модели Khronos glTF-Sample-Assets в assets/models/khronos.
# Версия набора закреплена коммитом, чтобы тесты сравнения картинок не ломались от обновлений.
#   ./assets/fetch_khronos.sh          — все модели
#   ./assets/fetch_khronos.sh Sponza   — только указанные
set -euo pipefail

REPO=https://github.com/KhronosGroup/glTF-Sample-Assets.git
REV=8f492390ddc7a2ae221a1369ef560589cf8c70fb
ALL=(Box BoxTextured DamagedHelmet CesiumMan Fox Sponza)

MODELS=("${@:-${ALL[@]}}")
DEST="$(cd "$(dirname "$0")" && pwd)/models/khronos"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Частичный клон: без истории и без содержимого файлов, затем только нужные папки
git clone -q --filter=blob:none --no-checkout "$REPO" "$TMP/repo"
git -C "$TMP/repo" sparse-checkout set --no-cone $(printf '/Models/%s/\n' "${MODELS[@]}")
git -C "$TMP/repo" checkout -q "$REV"

for model in "${MODELS[@]}"; do
  src="$TMP/repo/Models/$model"
  out="$DEST/$model"
  rm -rf "$out" && mkdir -p "$out"
  cp "$src/LICENSE.md" "$src/README.md" "$out/"
  # Предпочитаем один .glb; если его нет (Sponza) — папку glTF с внешними файлами
  if [[ -d "$src/glTF-Binary" ]]; then
    cp "$src/glTF-Binary/"*.glb "$out/"
  else
    cp -R "$src/glTF/." "$out/"
  fi
  echo "$model: $(du -sh "$out" | cut -f1)"
done
