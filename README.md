# GrainRust

Version: 0.1.2

GrainRust is a 4-track granular sampler built in Rust, inspired by the Torso S-4 workflow. It runs as a standalone app and as a plugin via nih-plug, with a Slint-based UI.

## Features

- 4 stereo tracks with per-track playback
- Sample loading per track (wav/flac/mp3/ogg)
- Global transport (play/stop all tracks)
- Per-track level, mute, and loop controls (start/length/x-fade)
- Engine loader per track (Tape)
- Post-tape Mosaic granular buffer with bypass toggle
- Project save/load (JSON)
- Standalone audio device settings (device, sample rate, buffer size)
- Smoothed per-track level changes to reduce crackle
- Scrollable UI when content exceeds the window height

## Build

### Standalone

``` bash
npm run grainrust:dev
```

To run with backtraces enabled:

```bash
npm run grainrust:dev-bt
```

### Plugin

Build a plugin binary using nih-plug (VST3/CLAP/etc.) depending on your local setup. See nih-plug documentation for details.

## Documentation

- Developer onboarding: `docs/DEVELOPER_ONBOARDING.md`
- Developer docs live in `docs/` and are published with Docusaurus
- End-user MDX docs are maintained in Storybook

### Docs (Docusaurus)

```bash
npm run docs:install
npm run docs:dev
```

### End-User Docs (Storybook)

```bash
npm run storybook
```

## Packaging

This project builds a standalone app and a VST3 plugin. Installers are OS-specific:

- Windows: NSIS installer with optional VST3 component
- macOS: pkgbuild/productbuild with optional VST3 component
- Linux: staged artifacts (no installer)

### Build installers/packages

```bash
npm run grainrust:build
```

### VST3 input path

Set `GRAINRUST_VST3_PATH` to the built VST3 bundle before running `grainrust:build` on all platforms. On macOS also set `GRAINRUST_APP_PATH` to the `.app` bundle.

## Controls (Current UI)

- Track selection buttons choose the active track for editing
- Engine selector + Load Engine loads the Tape engine for the active track
- Loading an engine on an already-loaded track prompts a confirmation warning
- Load Sample opens a file picker for the active track
- Record toggles recording for the active track
- Play toggles global transport (all tracks)
- Track Level and Mute affect only the active track
- Loop Start/Length/XFade apply to the active track
- Mosaic enable toggles the post-tape granular buffer per track
- Save/Load Project stores track paths and loop/mix state
- Settings panel is for standalone audio device configuration
- The engine controls are hidden until an engine is loaded for the active track

## Project Files

Project files are saved as JSON and include:

- Sample path per track (if loaded)
- Track level and mute state
- Loop start/length/x-fade and loop enabled state

## Notes

- This project is an early-stage implementation focused on Phase 1 behavior. Device models and modulation are planned next.
- Mosaic UI controls are present, but the DSP still uses fixed grain settings while the mapping is built out.
- The S-4 manual is included under `3rd-party/docs/` for reference.

## License

TBD
