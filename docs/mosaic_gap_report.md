# Mosaic / Granulator Gap Report (Torso S-4 vs Current Implementation)

## Scope
Compare the Mosaic/Granulator device behavior in this repo against the Torso S-4 manual specification. This report is descriptive only (no code changes).

## Sources Reviewed
- Torso S-4 Manual: `ref/torso_docs/s4_manual.txt` (Mosaic device, page 62 section)
- Current DSP: `src/lib.rs` (`process_track_mosaic` and supporting params)
- Current UI: `src/ui/devices/granulator_device.slint`

## Torso S-4 Mosaic Spec (Manual Summary)
From the manual (page 62):
- 4-second live audio buffer
- Time‑warping and pitch‑shifting algorithms
- Up to 128 grains per track
- Rate + Rate Mode: Straight / Dotted / Triplet / Free
- Parameters (Page 1): Pitch, Rate, Size, Contour, Warp, Spray, Pattern, Wet
- Parameters (Page 2): Detune, Random Rate, Random Size, SOS
- Pitch can be **quantized to scale** with a mode list:
  Chromatic, Major, Minor, Dorian, Lydian, Mixolydian, Super Locrian, Hex Aeolian,
  Hex Dorian, Blues, Pentatonic, Hirajoshi, Kumoi, Iwato, Whole Tone, Pelog,
  Tetratonic, Fifths, Octaves, Free

## Current Implementation (Observed)
DSP (`src/lib.rs`):
- 4-second buffer (`MOSAIC_BUFFER_SECONDS`) is used.
- Grain playback is **single‑grain at a time** (one active grain state tracked by
  `mosaic_grain_start/pos/len/pitch/pan`, with `mosaic_grain_wait`).
- Rate uses a **sync/free split**: `mosaic_rate <= 0.5` syncs to a list of
  divisions; above 0.5 is free rate (continuous).
- Parameters implemented and smoothed: pitch, rate, size, contour, warp, spray,
  pattern, wet, detune, random rate, random size, SOS, spatial (random pan).
- Pattern currently biases **grain start position** between recent vs random;
  it does **not** quantize pitch.
- SOS blends input into the circular buffer (overdub).

UI (`src/ui/devices/granulator_device.slint`):
- Exposes knobs for all parameters above and a bypass toggle.
- Rate readout displays a combined list of straight/dotted/triplet labels for
  the synced half of the knob and “Free %” for the upper half.
- There is **no explicit Rate Mode selector**.
- There is **no scale/pitch quantize selector**.

## Gaps / Mismatches vs S-4 Spec
1. **Grain Count / Density**
   - Spec: “Up to 128 grains per track.”
   - Current: Appears **single-grain at a time** with a wait interval; no
     concurrent grains or explicit grain count cap.

2. **Rate Mode Selection (Straight/Dotted/Triplet/Free)**
   - Spec: explicit “Rate Mode” with four modes.
   - Current: one continuous knob where the first half is synced to a **mixed**
     list of straight/dotted/triplet divisions; second half is free.
   - Missing: an explicit mode selector to constrain the list to one mode.

3. **Pitch Quantize to Scale**
   - Spec: pitch can be quantized to named scales.
   - Current: **no pitch quantize** or scale selection in DSP/UI.
   - “Pattern” does not map to pitch scale; it biases grain start position.

4. **Time‑Warp / Playhead Modulation**
   - Spec: time‑warping algorithms and internal modulation of the playhead.
   - Current: `warp` applies a **nonlinear warp** to grain start position
     (`pos.powf(1.0 + warp*2.0)`), but there is no explicit time‑warp algorithm
     or dedicated playhead modulation control.

5. **Pitch Modulation vs Playhead Modulation**
   - Spec lists “Pitch Modulation” and “Playhead Modulation” (from manual index).
   - Current: pitch is a ratio + detune, but **no dedicated modulation mode**
     or pitch quantize; playhead modulation is only implicit via pattern/warp
     randomization.

6. **Naming/Behavior Mapping**
   - Some parameter labels exist but behavior may not match manual descriptions
     (e.g., “Pattern” in S‑4 is pitch‑pattern quantized to scale; here it changes
     grain start position randomness).

## Areas Likely OK / Implemented
- 4‑second buffer
- Pitch, Rate, Size, Contour, Warp, Spray, Wet
- Detune, Random Rate, Random Size
- SOS (overdub into buffer)
- Stereo spray/spatial random pan

## Unknown / Needs Verification
- Whether any other part of the DSP implements pitch scale quantization
  (not seen in `process_track_mosaic`).
- Whether grain “density” intended by S‑4 is fully represented by the current
  rate algorithm (single‑grain vs multi‑grain).
- Any hidden/alternate UI or engine mode for Mosaic beyond
  `src/ui/devices/granulator_device.slint`.

## Summary
The current implementation captures many **surface parameters** (knobs, buffer,
randomization, SOS) but diverges in **core Mosaic identity** points from the
S‑4 manual: no explicit Rate Mode selection, no pitch‑scale quantize, and no
multi‑grain (up to 128) behavior. The “Pattern” control in code does not match
S‑4’s pitch‑pattern description. These are the primary gaps affecting “feel”
for S‑4 users.
