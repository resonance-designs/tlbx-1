#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/macos"
PKG_DIR="$DIST_DIR/pkg"

mkdir -p "$DIST_DIR" "$PKG_DIR"

"$ROOT_DIR"/cargo build --release

APP_PATH="${GRAINRUST_APP_PATH:-}"
VST3_PATH="${GRAINRUST_VST3_PATH:-}"

if [[ -z "$APP_PATH" || -z "$VST3_PATH" ]]; then
  echo "Set GRAINRUST_APP_PATH and GRAINRUST_VST3_PATH before running." >&2
  exit 1
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "App bundle not found: $APP_PATH" >&2
  exit 1
fi

if [[ ! -d "$VST3_PATH" ]]; then
  echo "VST3 bundle not found: $VST3_PATH" >&2
  exit 1
fi

APP_PKG="$PKG_DIR/GrainRust-App.pkg"
VST3_PKG="$PKG_DIR/GrainRust-VST3.pkg"

pkgbuild \
  --root "$APP_PATH" \
  --install-location "/Applications/GrainRust.app" \
  --identifier "com.grainrust.app" \
  --version "0.1.0" \
  "$APP_PKG"

pkgbuild \
  --root "$VST3_PATH" \
  --install-location "/Library/Audio/Plug-Ins/VST3/GrainRust.vst3" \
  --identifier "com.grainrust.vst3" \
  --version "0.1.0" \
  "$VST3_PKG"

cat > "$PKG_DIR/Distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="1">
  <title>GrainRust</title>
  <options customize="always"/>
  <choices-outline>
    <line choice="app_choice"/>
    <line choice="vst3_choice"/>
  </choices-outline>
  <choice id="app_choice" visible="true" start_selected="true">
    <pkg-ref id="com.grainrust.app"/>
  </choice>
  <choice id="vst3_choice" visible="true" start_selected="false">
    <pkg-ref id="com.grainrust.vst3"/>
  </choice>
  <pkg-ref id="com.grainrust.app" version="0.1.0" auth="Root">GrainRust-App.pkg</pkg-ref>
  <pkg-ref id="com.grainrust.vst3" version="0.1.0" auth="Root">GrainRust-VST3.pkg</pkg-ref>
</installer-gui-script>
XML

productbuild \
  --distribution "$PKG_DIR/Distribution.xml" \
  --package-path "$PKG_DIR" \
  "$DIST_DIR/GrainRust.pkg"

printf '\nCreated %s\n' "$DIST_DIR/GrainRust.pkg"
