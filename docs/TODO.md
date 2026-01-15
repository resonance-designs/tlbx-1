# TODO

## Core

### Tracks + Transport

- [x] 4 track selection and per-track sample loading
- [x] Global transport (play/stop all tracks in sync)
- [x] Per-track level and mute
- [x] Project save/load (JSON)
- [x] Standalone audio settings (device, sample rate, buffer size)
- [x] UI layout supports scrolling when content exceeds window height

### Master + I/O

- [ ] Master level, DJ filters, compression
- [ ] Record main output
- [ ] MIDI CC mapping and sync

### Engine 1: Tape-Deck

#### Phase 1: Material Device (Tape)

- [x] Loop controls (start/length/x-fade + loop enable)
- [x] Implement tape device parameters: speed, tempo, start, length, rotate, x-fade
- [x] Implement tape actions: load, monitor, overdub, record, save, reverse, freeze, keylock
- [x] Add UI for tape page 1/2 parameters and action buttons

#### Phase 2: Granular Device (Mosaic)

- [x] Implement granular buffer and grain spawning (basic)
- [x] Map pitch/rate/size/contour/warp/spray/pattern/wet to DSP
- [x] Map random rate/size, detune, and SOS to DSP
- [x] Add smoothing for Mosaic parameters

#### Phase 3: Filter Device (Ring)

- [ ] Implement resonator/filter bank core controls
- [ ] Add animation (waves/noise/tilt/detune) and pre/post mode

#### Phase 4: Color Device (Deform)

- [ ] Implement drive/compress/crush/tilt/noise chain
- [ ] Add noise gate and wet/dry control

#### Phase 5: Space Device (Vast)

- [ ] Implement delay + reverb chain
- [ ] Add clear/freeze actions

#### Phase 6: Modulation

- [ ] Add 4 mod slots per track (wave/random/ADSR)
- [ ] Add modulation routing and amount controls

#### Phase 7: Material Device (Poly)

- [ ] Implement polyphonic sampler playback with pitch/loop
- [ ] Add amp envelope and filter envelope controls
- [ ] Add MIDI note input per track and velocity response

### Engine 2: Animate

#### Phase 1: Vector Device (Vector)

- [ ] Replicate the vector synth oscillators (switchable between wavetable and sample type) and X-Y vector controls from the AniMMI synth in /ref/animmi
- [ ] 4 Oscillator slots with two modes the select from (vector/sample)

## Ongoing: Tooling + Docs

- [x] Add Docusaurus docs site
- [x] Add Storybook MDX end-user docs
- [x] Add developer onboarding docs
- [x] Add cross-platform packaging scripts (NSIS/pkgbuild/Linux staging)
- [x] Bundle docs site with installers and expose in-app Docs link
- [ ] Refine documentation
