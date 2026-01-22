---
title: User Manual
sidebar_position: 3
---

# User Manual

This manual covers the current TLBX-1 workflow as of the latest Mosaic update.

## Transport

- **Play/Stop**: The global transport button in the header starts or stops all tracks simultaneously.
- **Audition**: Located within the engine controls, this momentary button plays only the currently selected track while held down, allowing for quick checks without starting the entire project.
- **Keyboard (standalone)**: Space toggles Play/Stop. Escape closes open modals.

## Visualizers

- Use the visualizer toggles to switch between oscilloscope, spectrum, and vectorscope.

## Tracks

- Use the Track 1–4 buttons to select the active track.
- Each track can load a sample and run the Tape engine + Mosaic device, or load Animate/SimpKick.

## Tape Engine

- Load the Tape engine per track via the Engine selector + Load Engine.
- Use the **Audition** button for momentary playback of the selected track.
- Use Tape controls for speed/tempo/loop/start/length/x‑fade/rotate and tape actions (reverse, freeze, keylock, monitor, overdub).

## Animate Engine

- Load the Animate engine per track via the Engine selector + Load Engine.
- Animate displays its own slot controls and X‑Y pad when loaded.

## SimpKick Engine

- Load the SimpKick engine per track via the Engine selector + Load Engine.
- The kick synth includes Pitch, Decay, Attack, Drive, and Level controls.
- Use the 16-step sequencer row to toggle steps on/off per track.

## Mosaic Device

- Mosaic runs after Tape and draws from a 4‑second buffer.
- Mosaic ON/BYPASS toggles granular processing per track.
- Pitch is bipolar (±36 semitones); contour is bipolar; other params are unipolar.
- All Mosaic parameters are smoothed to avoid zipper noise.

## Audio Settings (Standalone)

- Open Settings to choose output/input device, sample rate, and buffer size.
- Settings open in a modal window and can be closed with Escape.

## Project Management

- Save Project / Load Project stores per‑track sample paths and loop/mix state.
- Use the Browser panel to browse project files and sample libraries.
- Add Library Folder registers a folder in the browser list.

## Browser

- Open the Browser to view saved projects and library folders.
- Selecting a folder updates the entry list for quick loading.

## Documentation

- Use the Docs button to open the local documentation site (installed with the app).
- UI controls are based on the [Resonance Designs Slint UI Component Kit](https://github.com/resonance-designs/rds-slint-ui-kit).
