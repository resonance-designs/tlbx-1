#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/linux"

mkdir -p "$DIST_DIR/bin" "$DIST_DIR/vst3"

"$ROOT_DIR"/cargo build --release

cp "$ROOT_DIR/target/release/tlbx-1" "$DIST_DIR/bin/tlbx-1"

if [[ -d "$ROOT_DIR/docs-site/build" ]]; then
  mkdir -p "$DIST_DIR/documentation"
  rm -rf "$DIST_DIR/documentation"
  mkdir -p "$DIST_DIR/documentation"
  cp -R "$ROOT_DIR/docs-site/build/." "$DIST_DIR/documentation"
fi

VST3_PATH="${TLBX_VST3_PATH:-}"
if [[ -n "$VST3_PATH" && -d "$VST3_PATH" ]]; then
  cp -R "$VST3_PATH" "$DIST_DIR/vst3/TLBX-1.vst3"
else
  echo "TLBX_VST3_PATH not set or not found; skipping VST3 staging."
fi

echo "Staged artifacts in $DIST_DIR"
