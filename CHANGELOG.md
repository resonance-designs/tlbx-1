# Changelog

## [0.1.17] - 2026-01-29

- Added G8 trance gate device (32-step per-track gate) with UI and DSP, placed after Ring in the downstream chain.
- Added Granulator rate sync behavior: first half of the control locks to BPM divisions (1, 1/2, 1/4, 1/8, 1/16); second half is free-rate.
- Documentation updates for G8 and Mosaic rate sync.

## [0.1.16] - 2026-01-27

- Significant UI updates to the main app header/bar.
- Removed Storybook and folded it's content into docusaurus site.
- Improved some UI components.
- Attempting to implement a shift-modifier for click action on track buttons to mute the track.
- Significant UI updates to the Tape-Deck engine, Granulator device, and Silk device.
- Updated to latest version of the RD Slint UI Component Kit.
- Rearranged the SynDRM interface
- Did some rewiring for pre/post filter processing on SynDRM tracks
- Improved knob, label, and selector components
- Added code comments
- Added options for pre/post filter switching per drum track on the SynDRM engine
- Added pitch envelope to SynDRM kick drum
- More SynDRM UI improvements
- Can now switch drum channel filters between pre/post of overdrive
- Changed range of LFO's in Animate engine
- Experimental: Tape engine can load video files and display playback in place of the waveform (audio still drives playback).

## [0.1.15] - 2026-01-23

- Cleaned build warnings by removing unused fields/helpers and tightening follow_host_tempo field usage across TLBX1/SlintEditor/SlintWindow, plus simplified loops.
- Removed unused project serialization structs/helpers. Fixed browser panel padding by wrapping in a layout.
- Adjusted the spectrum display to be less “wide” and show more movement.
- Per-engine mute toggles for Animate, SynDRM, Void Seed, and Tape UI.
- Transport decoupling to allow synth voice tails after transport stops.
- Void Seed output boost (+3 dB).
- Non-tape engine mixing fix (un-nesting track mix block).
- Added a public commit() function on RDSNumericKeypad and wired the modal “Okay” button to call it, so it uses the same committed path as the keypad.
- Removed "Quit" button and relocated "Browser" button.
- Reintroduced Void Seed smoothing in the DSP path and synced the reset/init path so the smooth atoms track the raw values again.

## [0.1.14] - 2026-01-23

- Renamed SimpKick to SynDRM and add snare synth lane + DSP params (drive, filters, attack)
- Migrated SynDRM DSP to FunDSP with per‑track chains and new filter types
- Overhauled SynDRM sequencer UI (pages/lanes/step editor, randomize/clear tooling)
- Extracted Mosaic (Granulator) and Ring (Silk) devices into dedicated components
- Added reusable UI components: XY pad, numeric keypad modal, keybed
- Integrated keybed in Animate UI and wired via global bus to avoid TLBX1 callback crashes
- Updated all relevant documentation.

## [0.1.13] - 2026-01-23

- Added Void Seed engine: a 12-oscillator generative drone swarm with chaotic LFO modulation, integrated feedback/diffusion delay, and "Chaos/Entropy" XY pad.
- Updated user manual and developer onboarding documentation

## [0.1.12] - 2026-01-22

- Fixed bug where granulator and silk devices were affecting audio processing for all tracks instead of just the track it's loaded on.
- Attempts to fix no keyboard input issue.
- Updated win support crate to 0.61.2 and added Win32_UI_Input_KeyboardAndMouse dependency.
- Moved engine components to their own folder.

## [0.1.11] - 2026-01-21

- Added SimpKick engine UI with a 16-step sequencer
- Added SimpKick attack control for click smoothing
- Added project/library browser panel for quick loading
- Updated docs to reflect current engine lineup and UI component usage

## [0.1.10] - 2026-01-21

- A little polish to the Tape-Deck UI.

## [0.1.9] - 2026-01-21

- Added Animate engine UI component (`animate_engine.slint`)
- Introduced a custom RDSComboBox with a popup list
- Theme updates for shared fonts and UI tokens
- Documentation updates for UI components and kit usage

## [0.1.8] - 2026-01-20

- Renamed the project to TLBX-1 across UI, packaging, and docs
- Updated plugin identifiers for TLBX-1 (VST3/CLAP)
- Logging now respects `RUST_LOG` via env_logger init
- Added branding to UI

## [0.1.7] - 2026-01-20

- Consolidated visualizer and meter components into `viz.slint`
- Split visualizers into dedicated oscilloscope/spectrum/vectorscope components
- Renamed Waveform visualizer component to `RDSWaveformViz`

## [0.1.6] - 2026-01-20

- Extracted Tape engine UI into `tape_engine.slint` for cleaner component structure
- Added lo-fi knob rendering path for performance and new indicator styles (circle/line/carrot)
- Settings panel now opens as a modal with Escape-to-close support in standalone

## [0.1.5] - 2026-01-19

- Reorganized UI layout with a consolidated 4x3 control grid for tape engine parameters
- Moved global transport controls (Play/Stop) to the application header
- Introduced momentary "Audition" button for per-track targeted playback
- Refactored Slint UI structure for better responsiveness and codebase stability

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
