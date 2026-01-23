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

- [x] Master level, DJ filters, compression
- [ ] Record main output
- [ ] Offline audio export
  - [ ] Click export button
  - [ ] File dialog to pick location and file name
  - [ ] Choose length (default: 1 minute)
- [ ] MIDI CC mapping and sync

### Engine 1: Tape-Deck

#### Phase 1: Material Device (Tape)

- [x] Loop controls (start/length/x-fade + loop enable)
- [x] Implement tape device parameters: speed, tempo, start, length, rotate, x-fade
- [x] Implement tape actions: load, monitor, overdub, record, save, reverse, freeze, keylock
- [x] Add UI for tape page 1/2 parameters and action buttons
- [x] Extract Tape UI into its own Slint component

#### Phase 2: Granular Device (Mosaic / Granulator)

- [x] Implement granular buffer and grain spawning (basic)
- [x] Map pitch/rate/size/contour/warp/spray/pattern/wet to DSP
- [x] Map random rate/size, detune, and SOS to DSP
- [x] Add smoothing for Mosaic parameters

#### Phase 3: Filter Device (Ring / Silk)

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

- [ ] Replicate the vector synth from the AniMMI synth in /ref/animmi
  - [x] 4 Oscillator slots with two types to select from (wavetable/sample)
    - [ ] Parameters that are applicable to both oscillator types
      - [x] Volume/Gain
      - [x] Pan
      - [x] Coarse Pitch
      - [x] Fine Pitch
      - [ ] Filter Type
        - [ ] Moog
        - [x] High-Pass
        - [x] Band-Pass
      - [x] Filter Cutoff
      - [x] Filter Resonance
      - [ ] Filter Amount
      - [ ] ADSR Envelope
    - [ ] Wavetable Slot
      - [x] Wavetable selector
        - [x] Large array of .wav wavetables files
      - [x] Wavetable LFO
        - [x] Amount
        - [x] Waveform
          - [x] Sine
          - [x] Triangle
          - [x] Saw
          - [x] Square
          - [x] S&H
        - [ ] Rate
          - [ ] Min: 0.01hz
          - [ ] Max: 20hz
        - [x] Sync
          - [x] Timing division based on BPM
            - [x] 1/16
            - [x] 1/8
            - [x] 1/4
            - [x] 1/3
            - [x] 1/2
            - [x] 1
            - [x] 2
            - [x] 4
    - [x] Sample Oscillator
      - [x] Sample Start Position
      - [x] Loop Start Position
      - [x] Loop End Position
  - [ ] Vector modulation between the 4 slots
    - [ ] X-Y Pad Interface
    - [ ] X-Y LFO's
      - [x] Amount
      - [x] Waveform
        - [x] Sine
        - [x] Triangle
        - [x] Saw
        - [x] Square
        - [x] S&H
      - [x] Rate
      - [x] Sync
        - [ ] Rate
          - [ ] Min: 0.01hz
          - [ ] Max: 20hz
        - [x] Timing division based on BPM
          - [x] 1/16
          - [x] 1/8
          - [x] 1/4
          - [x] 1/3
          - [x] 1/2
          - [x] 1
          - [x] 2
          - [x] 4
  - [ ] Sequencer
    - [ ] 16 steps X 8 pages (128 Steps total)
    - [ ] 10 lanes
    - [ ] Page navigation buttons to navigate to different pages of the sequencer
    - [ ] Lane navigation buttons to navigate through the 10 available lanes of the sequencer
    - [ ] Step navigation buttons to navigate up and down the steps of sequencer
    - [ ] Loop is determined by whether there are active steps in pages. If there are only active steps in page 1, the sequencer will loop after the 16th step of the sequencer (page 1). If page 2 has active steps it will loop after the 32nd step of the sequencer. If page 1 has steps, page 2 does not, and page 3 does, it will loop after the 48th step, and so on.
    - [ ] Scale mode (drop-down selector with button to assign the scale):
    - [ ] Each lane is assigned a note based on the scale
    - [ ] Can assign note changes per step to override the note set by the sequencers scale
    - [ ] Can assign parameter changes of the engine per step

### Engine 3: SynDRM

- [ ] 10 track drum synth engine
  - [x] Kick synth
    - [x] Parameters
      - [x] Pitch
      - [x] Decay
      - [x] Attack
      - [x] Drive
      - [x] Volume
  - [x] Snare synth
    - [x] Parameters
      - [x] Tone
      - [x] Decay
      - [x] Attack
      - [x] Drive
      - [x] Volume
  - [ ] Sequencer
    - [ ] 16 steps X 8 pages (128 Steps total)
    - [ ] 10 lanes (1 lane for each drum channel)
    - [ ] Page navigation buttons to navigate to different pages of the sequencer
    - [ ] Lane navigation buttons to navigate through the 10 available lanes of the sequencer
    - [ ] Step navigation buttons to navigate up and down the steps of sequencer
    - [ ] Loop is determined by whether there are active steps in pages. If there are only active steps in page 1, the sequencer will loop after the 16th step of the sequencer (page 1). If page 2 has active steps it will loop after the 32nd step of the sequencer. If page 1 has steps, page 2 does not, and page 3 does, it will loop after the 48th step, and so on.
    - [ ] Can assign parameter changes of the engine per step

### Engine 4: Void Seed

- [x] Replicate the generative drone synth built with Tone.js from my MMIBox project.

## Ongoing: Tooling + Docs

- [x] Add Docusaurus docs site
- [x] Add Storybook MDX end-user docs
- [x] Add developer onboarding docs
- [x] Add cross-platform packaging scripts (NSIS/pkgbuild/Linux staging)
- [x] Bundle docs site with installers and expose in-app Docs link
- [x] Refine documentation
- [x] Document modal settings, keyboard shortcuts, and lo-fi knob rendering
- [x] Document use of the Resonance Designs Slint UI Component Kit
