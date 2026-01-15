#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/linux"

mkdir -p "$DIST_DIR/bin" "$DIST_DIR/vst3"

"$ROOT_DIR"/cargo build --release

cp "$ROOT_DIR/target/release/grainrust" "$DIST_DIR/bin/grainrust"

VST3_PATH="${GRAINRUST_VST3_PATH:-}"
if [[ -n "$VST3_PATH" && -d "$VST3_PATH" ]]; then
  cp -R "$VST3_PATH" "$DIST_DIR/vst3/GrainRust.vst3"
else
  echo "GRAINRUST_VST3_PATH not set or not found; skipping VST3 staging."
fi

echo "Staged artifacts in $DIST_DIR"
