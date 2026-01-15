#!/usr/bin/env bash
set -euo pipefail

BIN_SRC="${1:-}"
VST3_SRC="${2:-}"

if [[ -z "$BIN_SRC" ]]; then
  echo "Usage: install.sh <standalone-bin> [vst3-bundle]" >&2
  exit 1
fi

install -Dm755 "$BIN_SRC" /usr/local/bin/grainrust

if [[ -n "$VST3_SRC" ]]; then
  sudo mkdir -p /usr/lib/vst3
  sudo cp -R "$VST3_SRC" /usr/lib/vst3/GrainRust.vst3
fi

echo "Installed GrainRust."
