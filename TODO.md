# TODO

## Phase 1: Core Tracks + Transport

- [x] 4 track selection and per-track sample loading
- [x] Global transport (play/stop all tracks in sync)
- [x] Per-track level and mute
- [x] Loop controls (start/length/x-fade + loop enable)
- [x] Project save/load (JSON)
- [x] Standalone audio settings (device, sample rate, buffer size)

## Phase 2: Material Device (Tape)

- [x] Implement tape device parameters: speed, tempo, start, length, rotate, x-fade
- [x] Implement tape actions: load, monitor, overdub, record, save, reverse, freeze, keylock
- [x] Add UI for tape page 1/2 parameters and action buttons

## Phase 3: Material Device (Poly)

- [ ] Implement polyphonic sampler playback with pitch/loop
- [ ] Add amp envelope and filter envelope controls
- [ ] Add MIDI note input per track and velocity response

## Phase 4: Granular Device (Mosaic)

- [ ] Implement granular buffer and grain spawning
- [ ] Add pitch/rate/size/contour/warp/spray/pattern/wet
- [ ] Add random rate/size, detune, and SOS controls

## Phase 5: Filter Device (Ring)

- [ ] Implement resonator/filter bank core controls
- [ ] Add animation (waves/noise/tilt/detune) and pre/post mode

## Phase 6: Color Device (Deform)

- [ ] Implement drive/compress/crush/tilt/noise chain
- [ ] Add noise gate and wet/dry control

## Phase 7: Space Device (Vast)

- [ ] Implement delay + reverb chain
- [ ] Add clear/freeze actions

## Phase 8: Modulation

- [ ] Add 4 mod slots per track (wave/random/ADSR)
- [ ] Add modulation routing and amount controls

## Phase 9: Master + I/O

- [ ] Master level, DJ filters, compression
- [ ] Record main output
- [ ] MIDI CC mapping and sync
