# Changelog

## [0.1.4] - 2026-01-15

- Added ring modulator effect with adjustable parameters including cutoff, resonance, decay, pitch, tone, tilt, slope, wet/dry mix, detuning, waveform selection, and noise integration
- Metronome with count-in support for playback and recording modes
- Global tempo control with host tempo synchronization capability
- Tempo settings persistence across project saves and loads

## [0.1.3] - 2026-01-14

- Added Mosaic parameter smoothing and full DSP mapping
- Added local docs deployment, app Docs link, and installer docs bundling

## [0.1.2] - 2026-01-14

- Added Mosaic granular buffer with per-track bypass
- Added Mosaic parameter controls and grouped Tape/Mosaic sections
- Added full-window UI scrolling for tall layouts
- Added docs tooling (Docusaurus + Storybook MDX) and onboarding guide
- Added cross-platform packaging scripts (NSIS/pkgbuild/Linux staging)
- Added npm scripts for app/dev/build workflows

## [0.1.1] - 2026-01-13

- Added per-track engine loader with confirmation prompt
- Default UI now starts minimal and reveals controls after engine load

## [0.1.0] - 2026-01-13

- Switched UI to Slint with baseview + softbuffer renderer
- Added global transport and per-track playback
- Added per-track level/mute and loop controls (start/length/x-fade)
- Added project save/load (JSON)
- Added standalone audio settings panel
- Fixed waveform rendering and mono->stereo playback routing
- Added per-track level smoothing to reduce crackle on fast changes
- UI now uses full window space with scrolling support
