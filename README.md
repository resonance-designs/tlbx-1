# GrainRust

GrainRust is a 4-track granular sampler built in Rust, inspired by the Torso S-4 workflow. It runs as a standalone app and as a plugin via nih-plug, with a Slint-based UI.

## Features

- 4 stereo tracks with per-track playback
- Sample loading per track (wav/flac/mp3/ogg)
- Global transport (play/stop all tracks)
- Per-track level, mute, and loop controls (start/length/x-fade)
- Engine loader per track (Tape)
- Project save/load (JSON)
- Standalone audio device settings (device, sample rate, buffer size)
- Smoothed per-track level changes to reduce crackle

## Build

### Standalone

``` bash
cargo run
```

### Plugin

Build a plugin binary using nih-plug (VST3/CLAP/etc.) depending on your local setup. See nih-plug documentation for details.

## Controls (Current UI)

- Track selection buttons choose the active track for editing
- Engine selector + Load Engine loads the Tape engine for the active track
- Loading an engine on an already-loaded track prompts a confirmation warning
- Load Sample opens a file picker for the active track
- Record toggles recording for the active track
- Play toggles global transport (all tracks)
- Track Level and Mute affect only the active track
- Loop Start/Length/XFade apply to the active track
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
- The S-4 manual is included under `3rd-party/docs/` for reference.

## License

TBD
