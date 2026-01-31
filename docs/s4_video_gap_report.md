# Torso S-4 Video Gap Report vs grainrust

Date: 2026-01-31

## Source and method
- Video source: `ref/torso_docs/Torso S-4 Sculpting Sampler (it sounds WILD). [MrlMerBQHwM].mp4`
- Frames sampled with local ffmpeg (`C:\Program Files\Krita (x64)\bin\ffmpeg.exe`) at ~1 frame/minute.
- Sampled frames: `ref/torso_docs/s4_video_frames_min/frame_*.jpg` (43 frames)
- Contact sheet: `ref/torso_docs/s4_video_frames_min/contact.jpg`
- Note: This report compares *visible UI/feature behavior* in the video to the current codebase. It does **not** validate audio results.

## What the video shows (UI pages and controls)
Frame references below are from `ref/torso_docs/s4_video_frames_min/`:

- Tape page
  - Visible controls: Speed, Tempo, Start, Length, Rotate, XFade; waveform display; Load/Monitor/Overdub/Record; Reverse/Freeze/Keylock (seen in expanded view).
  - Example frames: `frame_0005.jpg`, `frame_0007.jpg`, `frame_0009.jpg`, `frame_0022.jpg`.

- Mosaic (granulator) page, set A
  - Controls: Pitch, Rate, Size, Contour, Warp, Spray, Pattern, Wet.
  - Example frames: `frame_0012.jpg`, `frame_200.jpg`.

- Mosaic page, set B
  - Controls: Detune, Rand Rate, Rand Size, SOS.
  - Example frames: `frame_0035.jpg`, `frame_00080.jpg`.

- Ring page (filter device)
  - Controls visible: Cutoff, Resonance, Decay, Tone, Scale, Wet; Pre/Post toggle shown in UI.
  - Example frames: `frame_0016.jpg`, `frame_0028.jpg`, `frame_0048.jpg`.

- Vast page (reverb-like device)
  - Controls visible: Delay/Time/Reverb, Step, Decay, Feedback, Spread, Damp (and related parameters).
  - Example frames: `frame_0020.jpg`.

- Deform page (drive/distortion/compression-type device)
  - Controls visible: Drive, Compress, Crush, Tilt, Noise, Noise Decay, Noise Color, Net, Gate.
  - Example frames: `frame_0001.jpg`.

- Random mod page
  - Controls: Rate, Amount, Phase, Offset, Length, Variation, Smooth, Spread.
  - Example frames: `frame_0024.jpg`, `frame_0032.jpg`.

- Wave mod page
  - Controls: Rate, Amount, Phase, Offset, Skew, Curve, Spread.
  - Example frames: `frame_0026.jpg`.

- Mapping / Mod page
  - “MAPPING: MOD 1” UI with MOD 1–4 selection shown as a page overlay.
  - Example frame: `frame_0030.jpg`.

- Mix page
  - Per-track Level + Filter with mute toggles for 4 tracks.
  - Example frames: `frame_0034.jpg`.

## Current codebase coverage (grainrust)

### Implemented (UI and DSP present)
- Tape engine UI and DSP are present.
  - UI: `src/ui/engines/tape_engine.slint` shows Speed, Tempo, T.Start, L.Start, Length, Rotate, XFade, Reverse/Freeze/Keylock/Monitor/Overdub/Record.
  - DSP: `src/lib.rs` includes tape processing and parameters wired to UI.

- Mosaic (Granulator) device UI and DSP are present.
  - UI: `src/ui/devices/granulator_device.slint` exposes Pitch, Rate, Size, Contour, Warp, Spray, Pattern, Wet, Detune, Spatial, Rand Rate, Rand Size, SOS.
  - DSP: `src/lib.rs` includes `mosaic_*` parameters and processing (grain spawn, rate, size, contour, warp, spray, pattern, detune, spatial, SOS, wet).

- Ring (Silk) filter device UI and DSP are present.
  - UI: `src/ui/devices/silk_device.slint` exposes Cutoff, Res, Pitch, Tone, Tilt, Slope, Decay, Decay mode, Wet, Detune, Waves, W-Rate, Noise, N-Rate, plus Pre/Post.
  - DSP: `src/lib.rs` includes `ring_*` parameters with modulation for waves/noise and quantized scale handling.

### Partially implemented / missing UI
- Ring Scale control exists in DSP (`ring_scale` in `src/lib.rs`) and is wired in UI state (`src/ui/tlbx1.slint` properties), **but no UI control** in `src/ui/devices/silk_device.slint` exposes it.

## Gaps vs video (features visible in S-4 video but not in code)

1. **Deform device**
   - Video shows a Deform page with Drive/Compress/Crush/Tilt/Noise/Noise Decay/Noise Color/Net/Gate.
   - No Deform device or DSP in `src/ui` or `src/lib.rs`.

2. **Vast reverb device**
   - Video shows a Vast page with Reverb/Delay/Time/Step/Decay/Feedback/Spread/Damp.
   - No Vast device or DSP in `src/ui` or `src/lib.rs`.

3. **Random mod source page**
   - Video shows a Random mod page (Rate/Amount/Phase/Offset/Length/Variation/Smooth/Spread).
   - No Random mod device/page exists in `src/ui` or `src/lib.rs`.

4. **Wave mod source page**
   - Video shows a Wave mod page (Rate/Amount/Phase/Offset/Skew/Curve/Spread).
   - No Wave mod device/page exists in `src/ui` or `src/lib.rs`.

5. **Mapping / Mod matrix page**
   - Video shows “MAPPING: MOD 1” with MOD 1–4 selection.
   - No mod-mapping UI or mod matrix implementation in `src/ui` or `src/lib.rs`.

6. **Mix page (per-track levels/filters/mute)**
   - Video shows a Mix page with per-track Level + Filter and Mute.
   - No equivalent “Mix” page UI found in `src/ui`.

7. **Ring Scale control missing in UI**
   - Video explicitly shows “Scale” on the Ring page.
   - Code has `ring_scale`, but UI lacks a Scale control in `src/ui/devices/silk_device.slint`.

## Notes / uncertainties
- The report is based on sampled frames, not a full time-coded walkthrough of the entire video. There may be additional S-4 pages not captured in the 1-minute sampling.
- Audio behavior is not validated here; this is a UI/DSP feature coverage comparison only.

## Suggested next steps
- If you want, I can expand this report into a roadmap (UI + DSP tasks) or add a second pass that captures more frames (e.g., every 10 seconds) to reduce the chance of missing pages.
