/**
 * GrainRust - A Rust-based granular audio sampler.
 * Copyright (C) 2026 Richard Bakos @ Resonance Designs.
 * Author: Richard Bakos <info@resonancedesigns.dev>
 * Website: https://resonancedesigns.dev
 * Version: 0.1.3
 * Component: Core Logic
 */

/**
 * GrainRust - A Rust-based granular audio sampler.
 * Copyright (C) 2026 Richard Bakos @ Resonance Designs.
 * Author: Richard Bakos <info@resonancedesigns.dev>
 * Website: https://resonancedesigns.dev
 * Version: 0.1.3
 * Component: Core Logic
 */

use nih_plug::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait};
use parking_lot::Mutex;
use slint::{LogicalPosition, ModelRc, PhysicalSize, SharedString, VecModel};
use slint::platform::{
    self, Platform, PlatformError, PointerEventButton, WindowEvent,
};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use baseview::{
    Event as BaseEvent, EventStatus as BaseEventStatus, Window as BaseWindow, WindowHandle,
    WindowHandler as BaseWindowHandler, WindowOpenOptions, WindowScalePolicy,
};
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use raw_window_handle_06 as raw_window_handle_06;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::cell::RefCell;
use std::sync::mpsc;
use std::sync::{Arc, Once};
use std::time::Instant;
use std::f32::consts::PI;

pub const NUM_TRACKS: usize = 4;
pub const WAVEFORM_SUMMARY_SIZE: usize = 100;
pub const RECORD_MAX_SECONDS: usize = 30;
pub const RECORD_MAX_SAMPLE_RATE: usize = 48_000;
pub const RECORD_MAX_SAMPLES: usize = RECORD_MAX_SECONDS * RECORD_MAX_SAMPLE_RATE;
pub const MOSAIC_BUFFER_SECONDS: usize = 4;
pub const MOSAIC_BUFFER_SAMPLES: usize = MOSAIC_BUFFER_SECONDS * RECORD_MAX_SAMPLE_RATE;
pub const MOSAIC_BUFFER_CHANNELS: usize = 2;
pub const MOSAIC_OUTPUT_GAIN: f32 = 1.0;
const MOSAIC_RATE_MIN: f32 = 2.0;
const MOSAIC_RATE_MAX: f32 = 60.0;
const MOSAIC_SIZE_MIN_MS: f32 = 10.0;
const MOSAIC_SIZE_MAX_MS: f32 = 250.0;
const MOSAIC_PITCH_SEMITONES: f32 = 36.0;
const MOSAIC_DETUNE_CENTS: f32 = 25.0;
const MOSAIC_PARAM_SMOOTH_MS: f32 = 20.0;
const RING_PITCH_SEMITONES: f32 = 24.0;
const RING_CUTOFF_MIN_HZ: f32 = 20.0;
const RING_CUTOFF_MAX_HZ: f32 = 20_000.0;
const RING_DETUNE_CENTS: f32 = 20.0;
const RING_DETUNE_RATE_HZ: f32 = 0.25;
const METRONOME_CLICK_MS: f32 = 12.0;
const METRONOME_CLICK_GAIN: f32 = 0.25;
const METRONOME_COUNT_IN_MAX_TICKS: u32 = 8;
const KEYLOCK_GRAIN_SIZE: usize = 256;
const KEYLOCK_GRAIN_HOP: usize = KEYLOCK_GRAIN_SIZE / 2;
const OSCILLOSCOPE_SAMPLES: usize = 256;
const SPECTRUM_BINS: usize = 64;
const SPECTRUM_WINDOW: usize = 256;
const VECTORSCOPE_POINTS: usize = 128;

fn default_window_size() -> baseview::Size {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;

        let physical_width = unsafe { GetSystemMetrics(SM_CXSCREEN) } as f64;
        let physical_height = unsafe { GetSystemMetrics(SM_CYSCREEN) } as f64;
        let dpi = unsafe { GetDpiForSystem() } as f64;
        let scale = if dpi > 0.0 { dpi / 96.0 } else { 1.0 };
        return baseview::Size::new(physical_width / scale, physical_height / scale);
    }

    #[cfg(not(target_os = "windows"))]
    {
        baseview::Size::new(1280.0, 800.0)
    }
}

slint::include_modules!();

struct Track {
    /// Audio data for the track. Each channel is a Vec of f32.
    samples: Arc<Mutex<Vec<Vec<f32>>>>,
    /// Last loaded sample path, if any.
    sample_path: Arc<Mutex<Option<PathBuf>>>,
    /// Pre-calculated waveform summary for fast drawing.
    waveform_summary: Arc<Mutex<Vec<f32>>>,
    /// Whether the track is currently recording.
    is_recording: AtomicBool,
    /// Whether the track is armed for recording.
    record_armed: AtomicBool,
    /// Pending play start after count-in.
    pending_play: AtomicBool,
    /// Pending record start after count-in.
    pending_record: AtomicBool,
    /// Count-in samples remaining before starting.
    count_in_remaining: AtomicU32,
    /// Recording head position in samples.
    record_pos: AtomicU32,
    /// Whether the track is currently playing.
    is_playing: AtomicBool,
    /// Playback position in samples. Stored as u32 bits for f32.
    play_pos: AtomicU32,
    /// Track output level (linear gain).
    level: AtomicU32,
    /// Smoothed track output level (linear gain).
    level_smooth: AtomicU32,
    /// Smoothed track meter (left).
    meter_left: AtomicU32,
    /// Smoothed track meter (right).
    meter_right: AtomicU32,
    /// Track mute state.
    is_muted: AtomicBool,
    /// Tape speed multiplier.
    tape_speed: AtomicU32,
    /// Smoothed tape speed.
    tape_speed_smooth: AtomicU32,
    /// Tape tempo (BPM).
    tape_tempo: AtomicU32,
    /// Tape rate mode: 0=Free, 1=Straight, 2=Dotted, 3=Triplet.
    tape_rate_mode: AtomicU32,
    /// Tape rotate amount (normalized 0..1).
    tape_rotate: AtomicU32,
    /// Tape glide amount (normalized 0..1).
    tape_glide: AtomicU32,
    /// Tape sound-on-sound amount (normalized 0..1).
    tape_sos: AtomicU32,
    /// Tape reverse toggle.
    tape_reverse: AtomicBool,
    /// Tape freeze toggle.
    tape_freeze: AtomicBool,
    /// Tape keylock toggle.
    tape_keylock: AtomicBool,
    /// Keylock grain phase (0..KEYLOCK_GRAIN_HOP).
    keylock_phase: AtomicU32,
    /// Keylock grain A start position in samples.
    keylock_grain_a: AtomicU32,
    /// Keylock grain B start position in samples.
    keylock_grain_b: AtomicU32,
    /// Tape monitor toggle.
    tape_monitor: AtomicBool,
    /// Tape overdub toggle.
    tape_overdub: AtomicBool,
    /// Loop start position as normalized 0..1.
    loop_start: AtomicU32,
    /// Loop length as normalized 0..1.
    loop_length: AtomicU32,
    /// Loop crossfade amount as normalized 0..0.5.
    loop_xfade: AtomicU32,
    /// Loop enabled.
    loop_enabled: AtomicBool,
    /// Loop mode for playback.
    loop_mode: AtomicU32,
    /// Playback direction for ping-pong mode.
    loop_dir: AtomicI32,
    /// Last loop start position in samples (for jump-to behavior).
    loop_start_last: AtomicU32,
    /// Granular device type (0 = none, 1 = Mosaic).
    granular_type: AtomicU32,
    /// Mosaic pitch amount.
    mosaic_pitch: AtomicU32,
    /// Smoothed mosaic pitch amount.
    mosaic_pitch_smooth: AtomicU32,
    /// Mosaic grain rate.
    mosaic_rate: AtomicU32,
    /// Smoothed mosaic grain rate.
    mosaic_rate_smooth: AtomicU32,
    /// Mosaic grain size.
    mosaic_size: AtomicU32,
    /// Smoothed mosaic grain size.
    mosaic_size_smooth: AtomicU32,
    /// Mosaic contour.
    mosaic_contour: AtomicU32,
    /// Smoothed mosaic contour.
    mosaic_contour_smooth: AtomicU32,
    /// Mosaic warp amount.
    mosaic_warp: AtomicU32,
    /// Smoothed mosaic warp amount.
    mosaic_warp_smooth: AtomicU32,
    /// Mosaic spray amount.
    mosaic_spray: AtomicU32,
    /// Smoothed mosaic spray amount.
    mosaic_spray_smooth: AtomicU32,
    /// Mosaic pattern amount.
    mosaic_pattern: AtomicU32,
    /// Smoothed mosaic pattern amount.
    mosaic_pattern_smooth: AtomicU32,
    /// Mosaic wet/dry.
    mosaic_wet: AtomicU32,
    /// Smoothed mosaic wet/dry.
    mosaic_wet_smooth: AtomicU32,
    /// Mosaic detune.
    mosaic_detune: AtomicU32,
    /// Smoothed mosaic detune.
    mosaic_detune_smooth: AtomicU32,
    /// Mosaic random rate.
    mosaic_rand_rate: AtomicU32,
    /// Smoothed mosaic random rate.
    mosaic_rand_rate_smooth: AtomicU32,
    /// Mosaic random size.
    mosaic_rand_size: AtomicU32,
    /// Smoothed mosaic random size.
    mosaic_rand_size_smooth: AtomicU32,
    /// Mosaic sound-on-sound.
    mosaic_sos: AtomicU32,
    /// Smoothed mosaic sound-on-sound.
    mosaic_sos_smooth: AtomicU32,
    /// Mosaic output enabled.
    mosaic_enabled: AtomicBool,
    /// Mosaic ring buffer fed by tape output.
    mosaic_buffer: Arc<Mutex<Vec<Vec<f32>>>>,
    /// Mosaic ring buffer write position.
    mosaic_write_pos: AtomicU32,
    /// Mosaic grain start position in ring buffer.
    mosaic_grain_start: AtomicU32,
    /// Mosaic grain position within current grain.
    mosaic_grain_pos: AtomicU32,
    /// Mosaic grain length in output samples.
    mosaic_grain_len: AtomicU32,
    /// Mosaic samples to wait before starting the next grain.
    mosaic_grain_wait: AtomicU32,
    /// Mosaic pitch ratio for the active grain.
    mosaic_grain_pitch: AtomicU32,
    /// Mosaic RNG state for grain start selection.
    mosaic_rng_state: AtomicU32,
    /// Ring filter cutoff (normalized 0..1).
    ring_cutoff: AtomicU32,
    /// Smoothed ring cutoff.
    ring_cutoff_smooth: AtomicU32,
    /// Ring filter resonance (normalized 0..1).
    ring_resonance: AtomicU32,
    /// Smoothed ring resonance.
    ring_resonance_smooth: AtomicU32,
    /// Ring filter decay (normalized 0..1).
    ring_decay: AtomicU32,
    /// Smoothed ring decay.
    ring_decay_smooth: AtomicU32,
    /// Ring filter decay mode (0 = sustain, 1 = choke).
    ring_decay_mode: AtomicU32,
    /// Ring filter pitch offset (normalized 0..1, bipolar).
    ring_pitch: AtomicU32,
    /// Smoothed ring pitch offset.
    ring_pitch_smooth: AtomicU32,
    /// Ring filter tone (normalized 0..1, bipolar).
    ring_tone: AtomicU32,
    /// Smoothed ring tone.
    ring_tone_smooth: AtomicU32,
    /// Ring filter tilt (normalized 0..1, bipolar).
    ring_tilt: AtomicU32,
    /// Smoothed ring tilt.
    ring_tilt_smooth: AtomicU32,
    /// Ring filter slope (normalized 0..1).
    ring_slope: AtomicU32,
    /// Smoothed ring slope.
    ring_slope_smooth: AtomicU32,
    /// Ring filter wet mix (normalized 0..1).
    ring_wet: AtomicU32,
    /// Smoothed ring wet mix.
    ring_wet_smooth: AtomicU32,
    /// Ring filter detune (normalized 0..1).
    ring_detune: AtomicU32,
    /// Smoothed ring detune.
    ring_detune_smooth: AtomicU32,
    /// Ring detune LFO phase.
    ring_detune_phase: AtomicU32,
    /// Ring filter enabled.
    ring_enabled: AtomicBool,
    /// Ring filter low-pass state per channel.
    ring_low: [AtomicU32; 2],
    /// Ring filter band-pass state per channel.
    ring_band: [AtomicU32; 2],
    /// Engine type loaded for this track (0 = none, 1 = tape).
    engine_type: AtomicU32,
    /// Logs one debug line per playback start to confirm audio thread output.
    debug_logged: AtomicBool,
    /// Sample rate of the loaded/recorded audio.
    sample_rate: AtomicU32,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            samples: Arc::new(Mutex::new(vec![vec![]; 2])),
            sample_path: Arc::new(Mutex::new(None)),
            waveform_summary: Arc::new(Mutex::new(vec![0.0; WAVEFORM_SUMMARY_SIZE])),
            is_recording: AtomicBool::new(false),
            record_armed: AtomicBool::new(false),
            pending_play: AtomicBool::new(false),
            pending_record: AtomicBool::new(false),
            count_in_remaining: AtomicU32::new(0),
            record_pos: AtomicU32::new(0.0f32.to_bits()),
            is_playing: AtomicBool::new(false),
            play_pos: AtomicU32::new(0.0f32.to_bits()),
            level: AtomicU32::new(1.0f32.to_bits()),
            level_smooth: AtomicU32::new(1.0f32.to_bits()),
            meter_left: AtomicU32::new(0.0f32.to_bits()),
            meter_right: AtomicU32::new(0.0f32.to_bits()),
            is_muted: AtomicBool::new(false),
            tape_speed: AtomicU32::new(1.0f32.to_bits()),
            tape_speed_smooth: AtomicU32::new(1.0f32.to_bits()),
            tape_tempo: AtomicU32::new(120.0f32.to_bits()),
            tape_rate_mode: AtomicU32::new(0),
            tape_rotate: AtomicU32::new(0.0f32.to_bits()),
            tape_glide: AtomicU32::new(0.0f32.to_bits()),
            tape_sos: AtomicU32::new(0.0f32.to_bits()),
            tape_reverse: AtomicBool::new(false),
            tape_freeze: AtomicBool::new(false),
            tape_keylock: AtomicBool::new(false),
            keylock_phase: AtomicU32::new(0.0f32.to_bits()),
            keylock_grain_a: AtomicU32::new(0.0f32.to_bits()),
            keylock_grain_b: AtomicU32::new(0.0f32.to_bits()),
            tape_monitor: AtomicBool::new(false),
            tape_overdub: AtomicBool::new(false),
            loop_start: AtomicU32::new(0.0f32.to_bits()),
            loop_length: AtomicU32::new(1.0f32.to_bits()),
            loop_xfade: AtomicU32::new(0.0f32.to_bits()),
            loop_enabled: AtomicBool::new(true),
            loop_mode: AtomicU32::new(0),
            loop_dir: AtomicI32::new(1),
            loop_start_last: AtomicU32::new(0),
            granular_type: AtomicU32::new(0),
            mosaic_pitch: AtomicU32::new(0.0f32.to_bits()),
            mosaic_pitch_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_rate: AtomicU32::new(0.5f32.to_bits()),
            mosaic_rate_smooth: AtomicU32::new(0.5f32.to_bits()),
            mosaic_size: AtomicU32::new(0.5f32.to_bits()),
            mosaic_size_smooth: AtomicU32::new(0.5f32.to_bits()),
            mosaic_contour: AtomicU32::new(0.5f32.to_bits()),
            mosaic_contour_smooth: AtomicU32::new(0.5f32.to_bits()),
            mosaic_warp: AtomicU32::new(0.0f32.to_bits()),
            mosaic_warp_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_spray: AtomicU32::new(0.0f32.to_bits()),
            mosaic_spray_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_pattern: AtomicU32::new(0.0f32.to_bits()),
            mosaic_pattern_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_wet: AtomicU32::new(0.0f32.to_bits()),
            mosaic_wet_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_detune: AtomicU32::new(0.0f32.to_bits()),
            mosaic_detune_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_rand_rate: AtomicU32::new(0.0f32.to_bits()),
            mosaic_rand_rate_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_rand_size: AtomicU32::new(0.0f32.to_bits()),
            mosaic_rand_size_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_sos: AtomicU32::new(0.0f32.to_bits()),
            mosaic_sos_smooth: AtomicU32::new(0.0f32.to_bits()),
            mosaic_enabled: AtomicBool::new(true),
            mosaic_buffer: Arc::new(Mutex::new(vec![
                vec![0.0; MOSAIC_BUFFER_SAMPLES];
                MOSAIC_BUFFER_CHANNELS
            ])),
            mosaic_write_pos: AtomicU32::new(0),
            mosaic_grain_start: AtomicU32::new(0),
            mosaic_grain_pos: AtomicU32::new(0.0f32.to_bits()),
            mosaic_grain_len: AtomicU32::new(0),
            mosaic_grain_wait: AtomicU32::new(0),
            mosaic_grain_pitch: AtomicU32::new(1.0f32.to_bits()),
            mosaic_rng_state: AtomicU32::new(0x1234_abcd),
            ring_cutoff: AtomicU32::new(0.5f32.to_bits()),
            ring_cutoff_smooth: AtomicU32::new(0.5f32.to_bits()),
            ring_resonance: AtomicU32::new(0.0f32.to_bits()),
            ring_resonance_smooth: AtomicU32::new(0.0f32.to_bits()),
            ring_decay: AtomicU32::new(0.0f32.to_bits()),
            ring_decay_smooth: AtomicU32::new(0.0f32.to_bits()),
            ring_decay_mode: AtomicU32::new(0),
            ring_pitch: AtomicU32::new(0.5f32.to_bits()),
            ring_pitch_smooth: AtomicU32::new(0.5f32.to_bits()),
            ring_tone: AtomicU32::new(0.5f32.to_bits()),
            ring_tone_smooth: AtomicU32::new(0.5f32.to_bits()),
            ring_tilt: AtomicU32::new(0.5f32.to_bits()),
            ring_tilt_smooth: AtomicU32::new(0.5f32.to_bits()),
            ring_slope: AtomicU32::new(0.5f32.to_bits()),
            ring_slope_smooth: AtomicU32::new(0.5f32.to_bits()),
            ring_wet: AtomicU32::new(0.0f32.to_bits()),
            ring_wet_smooth: AtomicU32::new(0.0f32.to_bits()),
            ring_detune: AtomicU32::new(0.0f32.to_bits()),
            ring_detune_smooth: AtomicU32::new(0.0f32.to_bits()),
            ring_detune_phase: AtomicU32::new(0.0f32.to_bits()),
            ring_enabled: AtomicBool::new(false),
            ring_low: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            ring_band: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            engine_type: AtomicU32::new(0),
            debug_logged: AtomicBool::new(false),
            sample_rate: AtomicU32::new(44_100),
        }
    }
}

pub struct GrainRust {
    params: Arc<GrainRustParams>,
    tracks: Arc<[Track; NUM_TRACKS]>,
    master_meters: Arc<MasterMeters>,
    visualizer: Arc<VisualizerState>,
    global_tempo: Arc<AtomicU32>,
    follow_host_tempo: Arc<AtomicBool>,
    metronome_enabled: Arc<AtomicBool>,
    metronome_count_in_ticks: Arc<AtomicU32>,
    metronome_count_in_playback: Arc<AtomicBool>,
    metronome_count_in_record: Arc<AtomicBool>,
    metronome_phase_samples: u32,
    metronome_click_remaining: u32,
}

#[derive(Params)]
pub struct GrainRustParams {
    #[id = "selected_track"]
    pub selected_track: IntParam,

    #[id = "gain"]
    pub gain: FloatParam,
}

impl Default for GrainRust {
    fn default() -> Self {
        let tracks = [
            Track::default(),
            Track::default(),
            Track::default(),
            Track::default(),
        ];
        
        Self {
            params: Arc::new(GrainRustParams::default()),
            tracks: Arc::new(tracks),
            master_meters: Arc::new(MasterMeters::default()),
            visualizer: Arc::new(VisualizerState::new()),
            global_tempo: Arc::new(AtomicU32::new(120.0f32.to_bits())),
            follow_host_tempo: Arc::new(AtomicBool::new(true)),
            metronome_enabled: Arc::new(AtomicBool::new(false)),
            metronome_count_in_ticks: Arc::new(AtomicU32::new(0)),
            metronome_count_in_playback: Arc::new(AtomicBool::new(false)),
            metronome_count_in_record: Arc::new(AtomicBool::new(false)),
            metronome_phase_samples: 0,
            metronome_click_remaining: 0,
        }
    }
}

impl VisualizerState {
    fn new() -> Self {
        Self {
            oscilloscope: Mutex::new(vec![0.0; OSCILLOSCOPE_SAMPLES]),
            spectrum: Mutex::new(vec![0.0; SPECTRUM_BINS]),
            vectorscope_x: Mutex::new(vec![0.0; VECTORSCOPE_POINTS]),
            vectorscope_y: Mutex::new(vec![0.0; VECTORSCOPE_POINTS]),
        }
    }
}

#[derive(Default)]
struct MasterMeters {
    left: AtomicU32,
    right: AtomicU32,
}

#[derive(Default)]
struct VisualizerState {
    oscilloscope: Mutex<Vec<f32>>,
    spectrum: Mutex<Vec<f32>>,
    vectorscope_x: Mutex<Vec<f32>>,
    vectorscope_y: Mutex<Vec<f32>>,
}

impl Default for GrainRustParams {
    fn default() -> Self {
        Self {
            selected_track: IntParam::new("Selected Track", 1, IntRange::Linear { min: 1, max: 4 }),
            gain: FloatParam::new(
                "Gain",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-70.0),
                    max: util::db_to_gain(6.0),
                    factor: FloatRange::gain_skew_factor(-70.0, 6.0),
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

pub enum GrainRustTask {
    LoadSample(usize, PathBuf),
    SaveProject(PathBuf),
    LoadProject(PathBuf),
}

struct PendingEngineLoad {
    track_idx: usize,
    engine_type: u32,
}

fn calculate_waveform_summary(samples: &[f32], summary: &mut [f32]) {
    if samples.is_empty() {
        for s in summary.iter_mut() { *s = 0.0; }
        return;
    }

    let num_bars = summary.len();
    let samples_per_bar = samples.len() / num_bars;

    if samples_per_bar == 0 {
        for i in 0..num_bars {
            summary[i] = samples.get(i).cloned().unwrap_or(0.0).abs();
        }
        return;
    }

    for i in 0..num_bars {
        let start = i * samples_per_bar;
        let end = (i + 1) * samples_per_bar;
        let mut max_amp: f32 = 0.0;
        for j in start..end {
            let amp = samples[j].abs();
            if amp > max_amp {
                max_amp = amp;
            }
        }
        summary[i] = max_amp;
    }
}

fn reset_track_for_engine(track: &Track, engine_type: u32) {
    track.engine_type.store(engine_type, Ordering::Relaxed);
    track.is_playing.store(false, Ordering::Relaxed);
    track.is_recording.store(false, Ordering::Relaxed);
    track.pending_play.store(false, Ordering::Relaxed);
    track.pending_record.store(false, Ordering::Relaxed);
    track.count_in_remaining.store(0, Ordering::Relaxed);
    track.play_pos.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.record_pos.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.level.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.level_smooth.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.is_muted.store(false, Ordering::Relaxed);
    track.tape_speed.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.tape_speed_smooth.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.tape_tempo.store(120.0f32.to_bits(), Ordering::Relaxed);
    track.tape_rate_mode.store(0, Ordering::Relaxed);
    track.tape_rotate.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.tape_glide.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.tape_sos.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.tape_reverse.store(false, Ordering::Relaxed);
    track.tape_freeze.store(false, Ordering::Relaxed);
    track.tape_keylock.store(false, Ordering::Relaxed);
    track.keylock_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.keylock_grain_a.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.keylock_grain_b.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.tape_monitor.store(false, Ordering::Relaxed);
    track.tape_overdub.store(false, Ordering::Relaxed);
    track.loop_start.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.loop_length.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.loop_xfade.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.loop_enabled.store(true, Ordering::Relaxed);
    track.loop_mode.store(0, Ordering::Relaxed);
    track.loop_dir.store(1, Ordering::Relaxed);
    track.granular_type.store(1, Ordering::Relaxed);
    track.mosaic_pitch.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_pitch_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_rate.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.mosaic_rate_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.mosaic_size.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.mosaic_size_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.mosaic_contour.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.mosaic_contour_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.mosaic_warp.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_warp_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_spray.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_spray_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_pattern.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_pattern_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_wet.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_wet_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_detune.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_detune_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_rand_rate.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_rand_rate_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_rand_size.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_rand_size_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_sos.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_sos_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_enabled.store(true, Ordering::Relaxed);
    track.mosaic_write_pos.store(0, Ordering::Relaxed);
    track.mosaic_grain_start.store(0, Ordering::Relaxed);
    track.mosaic_grain_pos.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_grain_len.store(0, Ordering::Relaxed);
    track.mosaic_grain_wait.store(0, Ordering::Relaxed);
    track.mosaic_grain_pitch.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.mosaic_rng_state.store(0x1234_abcd, Ordering::Relaxed);
    track.ring_cutoff.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_cutoff_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_resonance.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_resonance_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_decay.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_decay_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_decay_mode.store(0, Ordering::Relaxed);
    track.ring_pitch.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_pitch_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_tone.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_tone_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_tilt.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_tilt_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_slope.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_slope_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_wet.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_wet_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_detune.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_detune_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_detune_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_enabled.store(false, Ordering::Relaxed);
    for channel in 0..2 {
        track.ring_low[channel].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.ring_band[channel].store(0.0f32.to_bits(), Ordering::Relaxed);
    }
    if let Some(mut buffer) = track.mosaic_buffer.try_lock() {
        for channel in buffer.iter_mut() {
            channel.fill(0.0);
        }
    }
    track.sample_rate.store(44_100, Ordering::Relaxed);
    track.debug_logged.store(false, Ordering::Relaxed);

    {
        let mut samples = track.samples.lock();
        *samples = vec![vec![]; 2];
    }
    {
        let mut summary = track.waveform_summary.lock();
        summary.fill(0.0);
    }
    *track.sample_path.lock() = None;
}

impl Plugin for GrainRust {
    const NAME: &'static str = "GrainRust";
    const VENDOR: &'static str = "Zencoder";
    const URL: &'static str = "https://example.com";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            // Input-enabled layout for recording/monitoring.
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            // Generator-style layout.
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = GrainRustTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(SlintEditor {
            params: self.params.clone(),
            tracks: self.tracks.clone(),
            master_meters: self.master_meters.clone(),
            visualizer: self.visualizer.clone(),
            global_tempo: self.global_tempo.clone(),
            follow_host_tempo: self.follow_host_tempo.clone(),
            metronome_enabled: self.metronome_enabled.clone(),
            metronome_count_in_ticks: self.metronome_count_in_ticks.clone(),
            metronome_count_in_playback: self.metronome_count_in_playback.clone(),
            metronome_count_in_record: self.metronome_count_in_record.clone(),
            async_executor,
        }))
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let tracks = self.tracks.clone();
        let global_tempo = self.global_tempo.clone();
        Box::new(move |task| match task {
            GrainRustTask::LoadSample(track_idx, path) => {
                if track_idx >= NUM_TRACKS {
                    return;
                }
                
                match load_audio_file(&path) {
                    Ok((new_samples, sample_rate)) => {
                        let mut samples = tracks[track_idx].samples.lock();
                        let mut summary = tracks[track_idx].waveform_summary.lock();
                        let mut sample_path = tracks[track_idx].sample_path.lock();
                        
                        *samples = new_samples;
                        *sample_path = Some(path.clone());
                        tracks[track_idx]
                            .sample_rate
                            .store(sample_rate, Ordering::Relaxed);
                        if !samples.is_empty() {
                            calculate_waveform_summary(&samples[0], &mut summary);
                        } else {
                            summary.fill(0.0);
                        }
                        
                        nih_log!("Loaded sample: {:?}", path);
                    }
                    Err(e) => {
                        nih_log!("Failed to load sample: {:?}", e);
                    }
                }
            }
            GrainRustTask::SaveProject(path) => {
                let tempo =
                    f32::from_bits(global_tempo.load(Ordering::Relaxed));
                if let Err(err) = save_project(&tracks, tempo, &path) {
                    nih_log!("Failed to save project: {:?}", err);
                } else {
                    nih_log!("Saved project: {:?}", path);
                }
            }
            GrainRustTask::LoadProject(path) => {
                if let Err(err) = load_project(&tracks, &global_tempo, &path) {
                    nih_log!("Failed to load project: {:?}", err);
                } else {
                    nih_log!("Loaded project: {:?}", path);
                }
            }
        })
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut keep_alive = false;
        let mut global_tempo =
            f32::from_bits(self.global_tempo.load(Ordering::Relaxed)).clamp(20.0, 240.0);
        if self.follow_host_tempo.load(Ordering::Relaxed) {
            if let Some(tempo) = context.transport().tempo {
                let tempo = tempo as f32;
                if tempo.is_finite() {
                    global_tempo = tempo.clamp(20.0, 240.0);
                    self.global_tempo
                        .store(global_tempo.to_bits(), Ordering::Relaxed);
                }
            }
        }

        let buffer_samples = buffer.samples() as u32;
        let mut any_pending = false;
        if buffer_samples > 0 {
            let mut play_remaining = None;
            for track in self.tracks.iter() {
                if track.pending_play.load(Ordering::Relaxed) {
                    play_remaining = Some(track.count_in_remaining.load(Ordering::Relaxed));
                    break;
                }
            }
            if let Some(remaining) = play_remaining {
                any_pending = true;
                keep_alive = true;
                let new_remaining = remaining.saturating_sub(buffer_samples);
                for track in self.tracks.iter() {
                    if !track.pending_play.load(Ordering::Relaxed) {
                        continue;
                    }
                    if remaining == 0 {
                        track.is_playing.store(true, Ordering::Relaxed);
                        track.pending_play.store(false, Ordering::Relaxed);
                        track.count_in_remaining.store(0, Ordering::Relaxed);
                    } else {
                        track
                            .count_in_remaining
                            .store(new_remaining, Ordering::Relaxed);
                    }
                }
            }

            for track in self.tracks.iter() {
                let pending_record = track.pending_record.load(Ordering::Relaxed);
                if !pending_record {
                    continue;
                }
                any_pending = true;
                keep_alive = true;
                let remaining = track.count_in_remaining.load(Ordering::Relaxed);
                if remaining == 0 {
                    track.is_recording.store(true, Ordering::Relaxed);
                    track.pending_record.store(false, Ordering::Relaxed);
                } else {
                    let new_remaining = remaining.saturating_sub(buffer_samples);
                    track
                        .count_in_remaining
                        .store(new_remaining, Ordering::Relaxed);
                }
            }
        }

        // Handle recording for all tracks
        for track in self.tracks.iter() {
            if track.is_recording.load(Ordering::Relaxed) {
                keep_alive = true;
            }
            if track.is_recording.load(Ordering::Relaxed) {
                if let Some(mut samples) = track.samples.try_lock() {
                    let overdub = track.tape_overdub.load(Ordering::Relaxed);
                    let sos = f32::from_bits(track.tape_sos.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                    // Ensure we have enough channels
                    while samples.len() < buffer.channels() {
                        samples.push(vec![]);
                    }

                    let input = buffer.as_slice_immutable();
                    let loop_start_norm =
                        f32::from_bits(track.loop_start.load(Ordering::Relaxed)).clamp(0.0, 0.999);
                    let loop_length_norm =
                        f32::from_bits(track.loop_length.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                    let record_start = (loop_start_norm * RECORD_MAX_SAMPLES as f32) as usize;
                    let mut record_len =
                        (loop_length_norm * RECORD_MAX_SAMPLES as f32) as usize;
                    if record_len == 0 {
                        record_len = 1;
                    }
                    let record_end = (record_start + record_len).min(RECORD_MAX_SAMPLES);
                    let mut write_pos =
                        f32::from_bits(track.record_pos.load(Ordering::Relaxed))
                            .max(0.0) as usize;
                    if write_pos < record_start || write_pos >= record_end {
                        write_pos = record_start;
                    }
                    for channel_idx in 0..buffer.channels() {
                        let channel_data = &input[channel_idx];
                        let buf = &mut samples[channel_idx];
                        let mut write_idx = write_pos;
                        for sample in channel_data.iter() {
                            if write_idx >= record_end {
                                if overdub {
                                    write_idx = record_start;
                                } else {
                                    track.is_recording.store(false, Ordering::Relaxed);
                                    break;
                                }
                            }
                            if write_idx >= buf.len() {
                                track.is_recording.store(false, Ordering::Relaxed);
                                break;
                            }
                            if overdub {
                                let existing = buf[write_idx];
                                buf[write_idx] = existing * sos + *sample;
                            } else {
                                buf[write_idx] = *sample;
                            }
                            write_idx += 1;
                        }
                        if !track.is_recording.load(Ordering::Relaxed) {
                            break;
                        }
                        write_pos = write_idx;
                    }
                    track
                        .record_pos
                        .store((write_pos as f32).to_bits(), Ordering::Relaxed);
                }
            }
        }

        let any_playing = self
            .tracks
            .iter()
            .any(|track| track.is_playing.load(Ordering::Relaxed));
        let any_recording = self
            .tracks
            .iter()
            .any(|track| track.is_recording.load(Ordering::Relaxed));
        let mut monitor_level = 0.0;
        for track in self.tracks.iter() {
            if track.tape_monitor.load(Ordering::Relaxed) && !track.is_muted.load(Ordering::Relaxed)
            {
                monitor_level += f32::from_bits(track.level.load(Ordering::Relaxed));
            }
        }
        let monitor_level = monitor_level.clamp(0.0, 1.0);
        let any_monitoring = monitor_level > 0.0;
        if any_monitoring {
            if (monitor_level - 1.0).abs() > f32::EPSILON {
                for channel_samples in buffer.iter_samples() {
                    for sample in channel_samples {
                        *sample *= monitor_level;
                    }
                }
            }
        } else {
            for channel_samples in buffer.iter_samples() {
                for sample in channel_samples {
                    *sample = 0.0;
                }
            }
        }

        // Handle playback for all tracks
        for track in self.tracks.iter() {
            if track.is_recording.load(Ordering::Relaxed) { continue; } // Don't play if recording

            if track.is_playing.load(Ordering::Relaxed) {
                keep_alive = true;
                if let Some(samples) = track.samples.try_lock() {
                    if samples.is_empty() || samples[0].is_empty() {
                        track.is_playing.store(false, Ordering::Relaxed);
                        continue;
                    }

                    let num_samples = samples[0].len();
                    let num_channels = samples.len();
                    let num_buffer_samples = buffer.samples();
                    let mosaic_active =
                        track.granular_type.load(Ordering::Relaxed) == 1;
                    let mut mosaic_buffer = if mosaic_active {
                        track.mosaic_buffer.try_lock()
                    } else {
                        None
                    };
                    let mosaic_len = if mosaic_active {
                        let sr = track.sample_rate.load(Ordering::Relaxed).max(1) as usize;
                        let len = (sr * MOSAIC_BUFFER_SECONDS).min(MOSAIC_BUFFER_SAMPLES);
                        len.max(1)
                    } else {
                        0
                    };
                    let mut mosaic_write_pos = if mosaic_active && mosaic_len > 0 {
                        (track.mosaic_write_pos.load(Ordering::Relaxed) as usize) % mosaic_len
                    } else {
                        0
                    };
                    let target_mosaic_sos =
                        f32::from_bits(track.mosaic_sos.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                    let smooth_mosaic_sos = smooth_param(
                        f32::from_bits(track.mosaic_sos_smooth.load(Ordering::Relaxed)),
                        target_mosaic_sos,
                        num_buffer_samples,
                        track.sample_rate.load(Ordering::Relaxed).max(1) as f32,
                    );
                    track
                        .mosaic_sos_smooth
                        .store(smooth_mosaic_sos.to_bits(), Ordering::Relaxed);
                    let mut track_peak_left = 0.0f32;
                    let mut track_peak_right = 0.0f32;
                    let track_level =
                        f32::from_bits(track.level.load(Ordering::Relaxed));
                    let track_muted = track.is_muted.load(Ordering::Relaxed);
                    let tape_speed =
                        f32::from_bits(track.tape_speed.load(Ordering::Relaxed)).clamp(-4.0, 4.0);
                    let tape_tempo = global_tempo;
                    let tape_rate_mode = track.tape_rate_mode.load(Ordering::Relaxed);
                    let tape_freeze = track.tape_freeze.load(Ordering::Relaxed);
                    let tape_reverse = track.tape_reverse.load(Ordering::Relaxed);
                    let tape_keylock = track.tape_keylock.load(Ordering::Relaxed);
                    let mut smooth_speed =
                        f32::from_bits(track.tape_speed_smooth.load(Ordering::Relaxed));
                    let target_level = if track_muted { 0.0 } else { track_level };
                    let mut smooth_level =
                        f32::from_bits(track.level_smooth.load(Ordering::Relaxed));
                    let level_step = if num_buffer_samples > 0 {
                        (target_level - smooth_level) / num_buffer_samples as f32
                    } else {
                        0.0
                    };
                    let rate_factor = match tape_rate_mode {
                        1 => 1.0,
                        2 => 1.5,
                        3 => 2.0 / 3.0,
                        _ => 0.0,
                    };
                    let tempo_speed = if tape_rate_mode == 0 {
                        tape_speed
                    } else {
                        (tape_tempo / 120.0) * rate_factor
                    };
                    let target_speed = if tape_freeze { 0.0 } else { tempo_speed };
                    let glide =
                        f32::from_bits(track.tape_glide.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                    let glide_factor = 1.0 + glide * 20.0;
                    let speed_step = if num_buffer_samples > 0 {
                        (target_speed - smooth_speed)
                            / (num_buffer_samples as f32 * glide_factor)
                    } else {
                        0.0
                    };
                    let loop_enabled = track.loop_enabled.load(Ordering::Relaxed);
                    let loop_mode = track.loop_mode.load(Ordering::Relaxed);
                    let keylock_enabled = tape_keylock && loop_mode != 1 && loop_mode != 4;
                    let loop_active = loop_enabled && loop_mode != 2;
                    let loop_start_norm =
                        f32::from_bits(track.loop_start.load(Ordering::Relaxed))
                            .clamp(0.0, 0.999);
                    let loop_length_norm =
                        f32::from_bits(track.loop_length.load(Ordering::Relaxed))
                            .clamp(0.0, 1.0);
                    let loop_xfade_norm =
                        f32::from_bits(track.loop_xfade.load(Ordering::Relaxed))
                            .clamp(0.0, 0.5);
                    let output = buffer.as_slice();
                    let mut play_pos = f32::from_bits(track.play_pos.load(Ordering::Relaxed));
                    let mut keylock_phase =
                        f32::from_bits(track.keylock_phase.load(Ordering::Relaxed));
                    let mut keylock_grain_a =
                        f32::from_bits(track.keylock_grain_a.load(Ordering::Relaxed));
                    let mut keylock_grain_b =
                        f32::from_bits(track.keylock_grain_b.load(Ordering::Relaxed));

                    let rotate_norm =
                        f32::from_bits(track.tape_rotate.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                    let base_start = (loop_start_norm * num_samples as f32) as usize;
                    let rotate_offset = (rotate_norm * num_samples as f32) as usize;
                    let loop_start = (base_start + rotate_offset) % num_samples.max(1);
                    let mut loop_len = (loop_length_norm * num_samples as f32) as usize;
                    if loop_len == 0 {
                        loop_len = num_samples.saturating_sub(loop_start).max(1);
                    }
                    let loop_end = (loop_start + loop_len).min(num_samples);
                    let loop_len = loop_end.saturating_sub(loop_start).max(1);
                    let mut xfade_samples = (loop_xfade_norm * loop_len as f32) as usize;
                    if xfade_samples * 2 > loop_len {
                        xfade_samples = loop_len / 2;
                    }
                    let mut direction = match loop_mode {
                        1 => track.loop_dir.load(Ordering::Relaxed),
                        3 => -1,
                        _ => 1,
                    };
                    if tape_reverse {
                        direction *= -1;
                    }
                    if loop_mode == 5 && loop_active {
                        let last_start = track.loop_start_last.load(Ordering::Relaxed) as usize;
                        if last_start != loop_start {
                            play_pos = loop_start as f32;
                            if keylock_enabled {
                                keylock_phase = 0.0;
                                keylock_grain_a = play_pos;
                                keylock_grain_b =
                                    play_pos + direction as f32 * KEYLOCK_GRAIN_HOP as f32;
                            }
                        }
                        track
                            .loop_start_last
                            .store(loop_start as u32, Ordering::Relaxed);
                    }

                    if !track.debug_logged.swap(true, Ordering::Relaxed) {
                        let first_sample = samples.get(0).and_then(|ch| ch.get(0)).cloned().unwrap_or(0.0);
                        nih_log!(
                            "Playback debug: output_ch={}, buffer_samples={}, sample_len={}, first_sample={}",
                            output.len(),
                            num_buffer_samples,
                            num_samples,
                            first_sample
                        );
                    }

                    for sample_idx in 0..num_buffer_samples {
                        let mut pos = play_pos as isize;
                        if pos < 0 || pos as usize >= num_samples {
                            if loop_active {
                                if direction >= 0 {
                                    pos = loop_start as isize;
                                } else {
                                    pos = loop_end.saturating_sub(1) as isize;
                                }
                                play_pos = pos as f32;
                            } else {
                                track.is_playing.store(false, Ordering::Relaxed);
                                break;
                            }
                        }
                        let pos = pos as usize;
                        if loop_mode == 2 {
                            if direction >= 0 && pos >= loop_end {
                                track.is_playing.store(false, Ordering::Relaxed);
                                break;
                            }
                            if direction < 0 && pos <= loop_start {
                                track.is_playing.store(false, Ordering::Relaxed);
                                break;
                            }
                        }

                        if keylock_enabled {
                            let speed = smooth_speed + speed_step * sample_idx as f32;
                            let step = direction as f32 * speed;
                            let hop = KEYLOCK_GRAIN_HOP as f32;
                            let fade = (keylock_phase / hop).clamp(0.0, 1.0);
                            let read_a = keylock_grain_a + keylock_phase;
                            let read_b = keylock_grain_b + keylock_phase;

                            for channel_idx in 0..output.len() {
                                let src_channel = if num_channels == 1 {
                                    0
                                } else if channel_idx < num_channels {
                                    channel_idx
                                } else {
                                    continue;
                                };
                                let sample_a = sample_at_linear(
                                    &samples,
                                    src_channel,
                                    read_a,
                                    loop_start,
                                    loop_end,
                                    loop_active,
                                    num_samples,
                                );
                                let sample_b = sample_at_linear(
                                    &samples,
                                    src_channel,
                                    read_b,
                                    loop_start,
                                    loop_end,
                                    loop_active,
                                    num_samples,
                                );
                                let sample_value = sample_a * (1.0 - fade) + sample_b * fade;
                                let level = smooth_level + level_step * sample_idx as f32;
                                let out_value = sample_value * level;
                                output[channel_idx][sample_idx] += out_value;
                                if let Some(mosaic) = mosaic_buffer.as_mut() {
                                    if mosaic_len > 0 && channel_idx < mosaic.len() {
                                        let existing = mosaic[channel_idx][mosaic_write_pos];
                                        mosaic[channel_idx][mosaic_write_pos] =
                                            out_value * (1.0 - smooth_mosaic_sos)
                                                + existing * smooth_mosaic_sos;
                                    }
                                }
                                if channel_idx == 0 {
                                    let amp = out_value.abs();
                                    if amp > track_peak_left {
                                        track_peak_left = amp;
                                    }
                                } else if channel_idx == 1 {
                                    let amp = out_value.abs();
                                    if amp > track_peak_right {
                                        track_peak_right = amp;
                                    }
                                }
                            }

                            if mosaic_buffer.is_some() && mosaic_len > 0 {
                                mosaic_write_pos = (mosaic_write_pos + 1) % mosaic_len;
                            }

                            keylock_phase += 1.0;
                            if keylock_phase >= hop {
                                keylock_phase -= hop;
                                keylock_grain_a = keylock_grain_b;
                                keylock_grain_b += step * hop;
                            }
                            let keylock_pos = keylock_grain_a + keylock_phase;
                            play_pos = keylock_pos;
                            if loop_mode == 2 {
                                if direction >= 0 && play_pos >= loop_end as f32 {
                                    track.is_playing.store(false, Ordering::Relaxed);
                                    break;
                                }
                                if direction < 0 && play_pos <= loop_start as f32 {
                                    track.is_playing.store(false, Ordering::Relaxed);
                                    break;
                                }
                            }
                            if loop_active && loop_end > loop_start {
                                if loop_mode == 1 {
                                    if direction > 0 && play_pos as usize >= loop_end {
                                        direction = -1;
                                        play_pos = loop_end.saturating_sub(1) as f32;
                                    } else if direction < 0 && play_pos <= loop_start as f32 {
                                        direction = 1;
                                        play_pos = loop_start as f32;
                                    }
                                } else if direction > 0 && play_pos as usize >= loop_end {
                                    play_pos = loop_start as f32;
                                } else if direction < 0 && play_pos < loop_start as f32 {
                                    play_pos = loop_end.saturating_sub(1) as f32;
                                }
                            }
                            if loop_mode != 2 {
                                keylock_grain_a = wrap_loop_pos(
                                    keylock_grain_a,
                                    loop_start,
                                    loop_end,
                                    loop_active,
                                    num_samples,
                                );
                                keylock_grain_b = wrap_loop_pos(
                                    keylock_grain_b,
                                    loop_start,
                                    loop_end,
                                    loop_active,
                                    num_samples,
                                );
                                play_pos = wrap_loop_pos(
                                    play_pos,
                                    loop_start,
                                    loop_end,
                                    loop_active,
                                    num_samples,
                                );
                            }
                        } else {
                            for channel_idx in 0..output.len() {
                                let src_channel = if num_channels == 1 {
                                    0
                                } else if channel_idx < num_channels {
                                    channel_idx
                                } else {
                                    continue;
                                };
                                let mut sample_value = samples[src_channel][pos];
                            if loop_active && direction > 0 && xfade_samples > 0 {
                                    let xfade_start = loop_end.saturating_sub(xfade_samples);
                                    if pos >= xfade_start && loop_end > loop_start {
                                        let tail_idx = pos - xfade_start;
                                        let head_pos = loop_start + tail_idx;
                                        if head_pos < loop_end {
                                            let fade_in = tail_idx as f32 / xfade_samples as f32;
                                            let fade_out = 1.0 - fade_in;
                                            let head_sample = samples[src_channel][head_pos];
                                            sample_value =
                                                sample_value * fade_out + head_sample * fade_in;
                                        }
                                    }
                                }
                            let level = smooth_level + level_step * sample_idx as f32;
                            let out_value = sample_value * level;
                            output[channel_idx][sample_idx] += out_value;
                            if let Some(mosaic) = mosaic_buffer.as_mut() {
                                if mosaic_len > 0 && channel_idx < mosaic.len() {
                                    let existing = mosaic[channel_idx][mosaic_write_pos];
                                    mosaic[channel_idx][mosaic_write_pos] =
                                        out_value * (1.0 - smooth_mosaic_sos)
                                            + existing * smooth_mosaic_sos;
                                }
                            }
                            if channel_idx == 0 {
                                let amp = out_value.abs();
                                if amp > track_peak_left {
                                    track_peak_left = amp;
                                }
                            } else if channel_idx == 1 {
                                let amp = out_value.abs();
                                if amp > track_peak_right {
                                    track_peak_right = amp;
                                }
                            }
                        }

                        if mosaic_buffer.is_some() && mosaic_len > 0 {
                            mosaic_write_pos = (mosaic_write_pos + 1) % mosaic_len;
                        }

                            let speed = smooth_speed + speed_step * sample_idx as f32;
                            play_pos += direction as f32 * speed;
                            if loop_active && loop_end > loop_start {
                                if loop_mode == 1 {
                                    if direction > 0 && play_pos as usize >= loop_end {
                                        direction = -1;
                                        play_pos = loop_end.saturating_sub(1) as f32;
                                    } else if direction < 0 && play_pos <= loop_start as f32 {
                                        direction = 1;
                                        play_pos = loop_start as f32;
                                    }
                                } else if direction > 0 && play_pos as usize >= loop_end {
                                    play_pos = loop_start as f32;
                                } else if direction < 0 && play_pos < loop_start as f32 {
                                    play_pos = loop_end.saturating_sub(1) as f32;
                                }
                            }
                        }
                    }
                    
                    track.play_pos.store(play_pos.to_bits(), Ordering::Relaxed);
                    if mosaic_buffer.is_some() && mosaic_len > 0 {
                        track
                            .mosaic_write_pos
                            .store(mosaic_write_pos as u32, Ordering::Relaxed);
                    }
                    smooth_speed += speed_step * num_buffer_samples as f32;
                    track
                        .tape_speed_smooth
                        .store(smooth_speed.to_bits(), Ordering::Relaxed);
                    if tape_keylock {
                        track
                            .keylock_phase
                            .store(keylock_phase.to_bits(), Ordering::Relaxed);
                        track
                            .keylock_grain_a
                            .store(keylock_grain_a.to_bits(), Ordering::Relaxed);
                        track
                            .keylock_grain_b
                            .store(keylock_grain_b.to_bits(), Ordering::Relaxed);
                    }
                    if loop_mode == 1 {
                        track.loop_dir.store(direction, Ordering::Relaxed);
                    }
                    smooth_level += level_step * num_buffer_samples as f32;
                    track
                        .level_smooth
                        .store(smooth_level.to_bits(), Ordering::Relaxed);

                    if num_channels == 1 && buffer.channels() > 1 {
                        track_peak_right = track_peak_left;
                    }
                    let prev_left =
                        f32::from_bits(track.meter_left.load(Ordering::Relaxed));
                    let prev_right =
                        f32::from_bits(track.meter_right.load(Ordering::Relaxed));
                    let next_left = smooth_meter(prev_left, track_peak_left);
                    let next_right = smooth_meter(prev_right, track_peak_right);
                    track
                        .meter_left
                        .store(next_left.to_bits(), Ordering::Relaxed);
                    track
                        .meter_right
                        .store(next_right.to_bits(), Ordering::Relaxed);
                }
            } else {
                let prev_left =
                    f32::from_bits(track.meter_left.load(Ordering::Relaxed));
                let prev_right =
                    f32::from_bits(track.meter_right.load(Ordering::Relaxed));
                let next_left = smooth_meter(prev_left, 0.0);
                let next_right = smooth_meter(prev_right, 0.0);
                track
                    .meter_left
                    .store(next_left.to_bits(), Ordering::Relaxed);
                track
                    .meter_right
                    .store(next_right.to_bits(), Ordering::Relaxed);
            }
        }

        if any_monitoring {
            keep_alive = true;
        }

        let any_mosaic = self.tracks.iter().any(|track| {
            track.is_playing.load(Ordering::Relaxed)
                && track.granular_type.load(Ordering::Relaxed) == 1
                && track.mosaic_enabled.load(Ordering::Relaxed)
        });
        if any_mosaic {
            let num_buffer_samples = buffer.samples();
            let num_channels = buffer.channels();
            let mut global_wet = 0.0f32;
            for track in self.tracks.iter() {
                if track.is_playing.load(Ordering::Relaxed)
                    && track.granular_type.load(Ordering::Relaxed) == 1
                    && track.mosaic_enabled.load(Ordering::Relaxed)
                {
                    let sr = track.sample_rate.load(Ordering::Relaxed).max(1) as f32;
                    let target_wet =
                        f32::from_bits(track.mosaic_wet.load(Ordering::Relaxed))
                            .clamp(0.0, 1.0);
                    let smooth_wet = smooth_param(
                        f32::from_bits(track.mosaic_wet_smooth.load(Ordering::Relaxed)),
                        target_wet,
                        num_buffer_samples,
                        sr,
                    );
                    track
                        .mosaic_wet_smooth
                        .store(smooth_wet.to_bits(), Ordering::Relaxed);
                    let wet = smooth_wet.clamp(0.0, 1.0);
                    if wet > global_wet {
                        global_wet = wet;
                    }
                }
            }
            if global_wet > 0.0 {
                let dry_gain = 1.0 - global_wet;
                for channel_samples in buffer.iter_samples() {
                    for sample in channel_samples {
                        *sample *= dry_gain;
                    }
                }
            }

            for track in self.tracks.iter() {
                if !track.is_playing.load(Ordering::Relaxed) {
                    continue;
                }
                if track.granular_type.load(Ordering::Relaxed) != 1 {
                    continue;
                }
                if !track.mosaic_enabled.load(Ordering::Relaxed) {
                    continue;
                }
                let output = buffer.as_slice();
                let mosaic_buffer = match track.mosaic_buffer.try_lock() {
                    Some(buffer) => buffer,
                    None => continue,
                };
                if mosaic_buffer.is_empty() || num_buffer_samples == 0 {
                    continue;
                }
                let sr = track.sample_rate.load(Ordering::Relaxed).max(1) as usize;
                let mosaic_len = (sr * MOSAIC_BUFFER_SECONDS)
                    .min(MOSAIC_BUFFER_SAMPLES)
                    .max(1);
                let target_pitch =
                    f32::from_bits(track.mosaic_pitch.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_rate =
                    f32::from_bits(track.mosaic_rate.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_size =
                    f32::from_bits(track.mosaic_size.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_contour =
                    f32::from_bits(track.mosaic_contour.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_warp =
                    f32::from_bits(track.mosaic_warp.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_spray =
                    f32::from_bits(track.mosaic_spray.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_pattern =
                    f32::from_bits(track.mosaic_pattern.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_wet =
                    f32::from_bits(track.mosaic_wet.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_detune =
                    f32::from_bits(track.mosaic_detune.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_rand_rate =
                    f32::from_bits(track.mosaic_rand_rate.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let target_rand_size =
                    f32::from_bits(track.mosaic_rand_size.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                let mosaic_pitch = smooth_param(
                    f32::from_bits(track.mosaic_pitch_smooth.load(Ordering::Relaxed)),
                    target_pitch,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_rate = smooth_param(
                    f32::from_bits(track.mosaic_rate_smooth.load(Ordering::Relaxed)),
                    target_rate,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_size = smooth_param(
                    f32::from_bits(track.mosaic_size_smooth.load(Ordering::Relaxed)),
                    target_size,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_contour = smooth_param(
                    f32::from_bits(track.mosaic_contour_smooth.load(Ordering::Relaxed)),
                    target_contour,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_warp = smooth_param(
                    f32::from_bits(track.mosaic_warp_smooth.load(Ordering::Relaxed)),
                    target_warp,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_spray = smooth_param(
                    f32::from_bits(track.mosaic_spray_smooth.load(Ordering::Relaxed)),
                    target_spray,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_pattern = smooth_param(
                    f32::from_bits(track.mosaic_pattern_smooth.load(Ordering::Relaxed)),
                    target_pattern,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_wet = smooth_param(
                    f32::from_bits(track.mosaic_wet_smooth.load(Ordering::Relaxed)),
                    target_wet,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_detune = smooth_param(
                    f32::from_bits(track.mosaic_detune_smooth.load(Ordering::Relaxed)),
                    target_detune,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_rand_rate = smooth_param(
                    f32::from_bits(track.mosaic_rand_rate_smooth.load(Ordering::Relaxed)),
                    target_rand_rate,
                    num_buffer_samples,
                    sr as f32,
                );
                let mosaic_rand_size = smooth_param(
                    f32::from_bits(track.mosaic_rand_size_smooth.load(Ordering::Relaxed)),
                    target_rand_size,
                    num_buffer_samples,
                    sr as f32,
                );
                track
                    .mosaic_pitch_smooth
                    .store(mosaic_pitch.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_rate_smooth
                    .store(mosaic_rate.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_size_smooth
                    .store(mosaic_size.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_contour_smooth
                    .store(mosaic_contour.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_warp_smooth
                    .store(mosaic_warp.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_spray_smooth
                    .store(mosaic_spray.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_pattern_smooth
                    .store(mosaic_pattern.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_wet_smooth
                    .store(mosaic_wet.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_detune_smooth
                    .store(mosaic_detune.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_rand_rate_smooth
                    .store(mosaic_rand_rate.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_rand_size_smooth
                    .store(mosaic_rand_size.to_bits(), Ordering::Relaxed);
                let pitch_bipolar = mosaic_cc_bipolar(mosaic_pitch);
                let contour_bipolar = mosaic_cc_bipolar(mosaic_contour);
                let base_rate = MOSAIC_RATE_MIN + (MOSAIC_RATE_MAX - MOSAIC_RATE_MIN) * mosaic_rate;
                let base_size_ms =
                    MOSAIC_SIZE_MIN_MS + (MOSAIC_SIZE_MAX_MS - MOSAIC_SIZE_MIN_MS) * mosaic_size;
                let semitones = pitch_bipolar * MOSAIC_PITCH_SEMITONES;
                let base_pitch_ratio = 2.0f32.powf(semitones / 12.0);
                let recent_write_pos =
                    (track.mosaic_write_pos.load(Ordering::Relaxed) as usize) % mosaic_len;

                let mut grain_pos =
                    f32::from_bits(track.mosaic_grain_pos.load(Ordering::Relaxed));
                let mut grain_len = track.mosaic_grain_len.load(Ordering::Relaxed) as usize;
                let mut grain_wait = track.mosaic_grain_wait.load(Ordering::Relaxed) as usize;
                let mut grain_start =
                    (track.mosaic_grain_start.load(Ordering::Relaxed) as usize) % mosaic_len;
                let mut grain_pitch =
                    f32::from_bits(track.mosaic_grain_pitch.load(Ordering::Relaxed));
                let mut rng_state = track.mosaic_rng_state.load(Ordering::Relaxed);

                for sample_idx in 0..num_buffer_samples {
                    if grain_pos >= grain_len as f32 {
                        if grain_wait > 0 {
                            grain_wait = grain_wait.saturating_sub(1);
                            continue;
                        }
                        let rand_rate = next_mosaic_rand_unit(&mut rng_state) * 2.0 - 1.0;
                        let rand_size = next_mosaic_rand_unit(&mut rng_state) * 2.0 - 1.0;
                        let mut rate = base_rate * (1.0 + mosaic_rand_rate * rand_rate * 0.5);
                        rate = rate.clamp(MOSAIC_RATE_MIN, MOSAIC_RATE_MAX);
                        let mut size_ms = base_size_ms * (1.0 + mosaic_rand_size * rand_size * 0.5);
                        size_ms = size_ms.clamp(MOSAIC_SIZE_MIN_MS, MOSAIC_SIZE_MAX_MS);
                        grain_len = ((size_ms / 1000.0) * sr as f32).round() as usize;
                        grain_len = grain_len.clamp(1, mosaic_len);
                        let interval = (sr as f32 / rate).max(1.0);
                        let wait = (interval - grain_len as f32).max(0.0);
                        grain_wait = wait as usize;

                        let rand_pos = next_mosaic_rand_unit(&mut rng_state);
                        let recent_pos = recent_write_pos as f32 / mosaic_len as f32;
                        let mut pos = recent_pos * (1.0 - mosaic_pattern) + rand_pos * mosaic_pattern;
                        if mosaic_warp > 0.0 {
                            pos = pos.powf(1.0 + mosaic_warp * 2.0);
                        }
                        let spray = next_mosaic_rand_unit(&mut rng_state) * 2.0 - 1.0;
                        pos = (pos + spray * mosaic_spray * 0.25).clamp(0.0, 0.999999);
                        grain_start = (pos * mosaic_len as f32) as usize;

                        let detune = next_mosaic_rand_unit(&mut rng_state) * 2.0 - 1.0;
                        let detune_cents = detune * mosaic_detune * MOSAIC_DETUNE_CENTS;
                        grain_pitch =
                            base_pitch_ratio * 2.0f32.powf(detune_cents / 1200.0);

                        grain_pos = 0.0;
                    }
                    let read_pos = grain_start as f32 + grain_pos * grain_pitch;
                    let t = (grain_pos / grain_len as f32).clamp(0.0, 1.0);
                    let base_env = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
                    let curve = if contour_bipolar >= 0.0 {
                        1.0 + contour_bipolar * 4.0
                    } else {
                        1.0 / (1.0 + (-contour_bipolar) * 4.0)
                    };
                    let env = base_env.powf(curve);
                    for channel_idx in 0..num_channels {
                        let src_channel = if channel_idx < mosaic_buffer.len() {
                            channel_idx
                        } else {
                            0
                        };
                        let sample_value = sample_at_linear_ring(
                            &mosaic_buffer,
                            src_channel,
                            read_pos,
                        );
                        let wet = mosaic_wet;
                        output[channel_idx][sample_idx] +=
                            sample_value * env * MOSAIC_OUTPUT_GAIN * wet;
                    }
                    grain_pos += 1.0;
                }

                track
                    .mosaic_grain_pos
                    .store(grain_pos.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_grain_len
                    .store(grain_len as u32, Ordering::Relaxed);
                track
                    .mosaic_grain_wait
                    .store(grain_wait as u32, Ordering::Relaxed);
                track
                    .mosaic_grain_start
                    .store(grain_start as u32, Ordering::Relaxed);
                track
                    .mosaic_grain_pitch
                    .store(grain_pitch.to_bits(), Ordering::Relaxed);
                track
                    .mosaic_rng_state
                    .store(rng_state, Ordering::Relaxed);
            }
        }

        let ring_track_idx = self
            .params
            .selected_track
            .value()
            .saturating_sub(1) as usize;
        let ring_track_idx = ring_track_idx.min(NUM_TRACKS - 1);
        let ring_track = &self.tracks[ring_track_idx];
        if ring_track.ring_enabled.load(Ordering::Relaxed) {
            let num_buffer_samples = buffer.samples();
            if num_buffer_samples > 0 {
                let sr = ring_track.sample_rate.load(Ordering::Relaxed).max(1) as f32;
                let target_cutoff =
                    f32::from_bits(ring_track.ring_cutoff.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_resonance =
                    f32::from_bits(ring_track.ring_resonance.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_decay =
                    f32::from_bits(ring_track.ring_decay.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_pitch =
                    f32::from_bits(ring_track.ring_pitch.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_tone =
                    f32::from_bits(ring_track.ring_tone.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_tilt =
                    f32::from_bits(ring_track.ring_tilt.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_slope =
                    f32::from_bits(ring_track.ring_slope.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_wet =
                    f32::from_bits(ring_track.ring_wet.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let target_detune =
                    f32::from_bits(ring_track.ring_detune.load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);

                let ring_cutoff = smooth_param(
                    f32::from_bits(ring_track.ring_cutoff_smooth.load(Ordering::Relaxed)),
                    target_cutoff,
                    num_buffer_samples,
                    sr,
                );
                let ring_resonance = smooth_param(
                    f32::from_bits(ring_track.ring_resonance_smooth.load(Ordering::Relaxed)),
                    target_resonance,
                    num_buffer_samples,
                    sr,
                );
                let ring_decay = smooth_param(
                    f32::from_bits(ring_track.ring_decay_smooth.load(Ordering::Relaxed)),
                    target_decay,
                    num_buffer_samples,
                    sr,
                );
                let ring_pitch = smooth_param(
                    f32::from_bits(ring_track.ring_pitch_smooth.load(Ordering::Relaxed)),
                    target_pitch,
                    num_buffer_samples,
                    sr,
                );
                let ring_tone = smooth_param(
                    f32::from_bits(ring_track.ring_tone_smooth.load(Ordering::Relaxed)),
                    target_tone,
                    num_buffer_samples,
                    sr,
                );
                let ring_tilt = smooth_param(
                    f32::from_bits(ring_track.ring_tilt_smooth.load(Ordering::Relaxed)),
                    target_tilt,
                    num_buffer_samples,
                    sr,
                );
                let ring_slope = smooth_param(
                    f32::from_bits(ring_track.ring_slope_smooth.load(Ordering::Relaxed)),
                    target_slope,
                    num_buffer_samples,
                    sr,
                );
                let ring_wet = smooth_param(
                    f32::from_bits(ring_track.ring_wet_smooth.load(Ordering::Relaxed)),
                    target_wet,
                    num_buffer_samples,
                    sr,
                )
                .clamp(0.0, 1.0);
                let ring_detune = smooth_param(
                    f32::from_bits(ring_track.ring_detune_smooth.load(Ordering::Relaxed)),
                    target_detune,
                    num_buffer_samples,
                    sr,
                );

                ring_track
                    .ring_cutoff_smooth
                    .store(ring_cutoff.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_resonance_smooth
                    .store(ring_resonance.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_decay_smooth
                    .store(ring_decay.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_pitch_smooth
                    .store(ring_pitch.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_tone_smooth
                    .store(ring_tone.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_tilt_smooth
                    .store(ring_tilt.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_slope_smooth
                    .store(ring_slope.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_wet_smooth
                    .store(ring_wet.to_bits(), Ordering::Relaxed);
                ring_track
                    .ring_detune_smooth
                    .store(ring_detune.to_bits(), Ordering::Relaxed);

                if ring_wet > 0.0 {
                    let decay_mode = ring_track.ring_decay_mode.load(Ordering::Relaxed);
                    let decay_sec = 0.05 + ring_decay.clamp(0.0, 1.0) * 3.95;
                    let decay_factor = (-1.0 / (decay_sec * sr)).exp();
                    let pitch_bipolar = ring_pitch * 2.0 - 1.0;
                    let tone_bipolar = ring_tone * 2.0 - 1.0;
                    let tilt_bipolar = ring_tilt * 2.0 - 1.0;
                    let pitch_ratio =
                        2.0f32.powf((pitch_bipolar * RING_PITCH_SEMITONES) / 12.0);
                    let cutoff_hz = RING_CUTOFF_MIN_HZ
                        * (RING_CUTOFF_MAX_HZ / RING_CUTOFF_MIN_HZ).powf(ring_cutoff);
                    let cutoff_hz =
                        (cutoff_hz * pitch_ratio).clamp(20.0, sr * 0.45);
                    let q = 0.5 + ring_resonance * 12.0;
                    let r = 1.0 / (2.0 * q.max(0.001));
                    let slope = ring_slope.clamp(0.0, 1.0);

                    let num_channels = buffer.channels();
                    let output = buffer.as_slice();
                    let detune_phase =
                        f32::from_bits(ring_track.ring_detune_phase.load(Ordering::Relaxed));
                    let mut phase = detune_phase;
                    let detune_rate = RING_DETUNE_RATE_HZ / sr;
                    let detune_depth = ring_detune.clamp(0.0, 1.0) * RING_DETUNE_CENTS;
                    for channel_idx in 0..num_channels {
                        let mut low =
                            f32::from_bits(ring_track.ring_low[channel_idx].load(Ordering::Relaxed));
                        let mut band =
                            f32::from_bits(ring_track.ring_band[channel_idx].load(Ordering::Relaxed));
                        for sample_idx in 0..num_buffer_samples {
                            if decay_mode == 1 {
                                let tilt = tilt_bipolar;
                                let low_decay = (decay_factor * (1.0 - tilt * 0.35)).clamp(0.0, 1.0);
                                let band_decay = (decay_factor * (1.0 + tilt * 0.35)).clamp(0.0, 1.0);
                                low *= low_decay;
                                band *= band_decay;
                            }
                            let input = output[channel_idx][sample_idx];
                            let lfo = (phase * 2.0 * PI).sin();
                            let detune_cents =
                                detune_depth * lfo * if channel_idx == 0 { -1.0 } else { 1.0 };
                            let detune_ratio = 2.0f32.powf(detune_cents / 1200.0);
                            let detuned_hz =
                                (cutoff_hz * detune_ratio).clamp(20.0, sr * 0.45);
                            let mut g_detune = (PI * detuned_hz / sr).tan();
                            if !g_detune.is_finite() {
                                g_detune = 0.0;
                            } else if g_detune > 10.0 {
                                g_detune = 10.0;
                            }
                            let v1 = (input - low - r * band) * g_detune;
                            let v2 = band + v1;
                            low += v2;
                            band = v2;
                            low = low.clamp(-8.0, 8.0);
                            band = band.clamp(-8.0, 8.0);
                            let high = input - low - r * band;
                            let filtered = if slope < 0.5 {
                                let t = slope / 0.5;
                                low + (band - low) * t
                            } else {
                                let t = (slope - 0.5) / 0.5;
                                band + (high - band) * t
                            };
                            if decay_mode == 0 {
                                let tilt = tilt_bipolar;
                                let low_decay = (decay_factor * (1.0 - tilt * 0.35)).clamp(0.0, 1.0);
                                let band_decay = (decay_factor * (1.0 + tilt * 0.35)).clamp(0.0, 1.0);
                                low *= low_decay;
                                band *= band_decay;
                            }
                            let tone_mix = filtered + tone_bipolar * (high - low) * 0.5;
                            output[channel_idx][sample_idx] =
                                input * (1.0 - ring_wet) + tone_mix * ring_wet;
                            if !low.is_finite() || !band.is_finite() || !output[channel_idx][sample_idx].is_finite() {
                                low = 0.0;
                                band = 0.0;
                                output[channel_idx][sample_idx] = input;
                            }
                            if channel_idx == 0 {
                                phase += detune_rate;
                                if phase >= 1.0 {
                                    phase -= 1.0;
                                }
                            }
                        }
                        ring_track.ring_low[channel_idx]
                            .store(low.to_bits(), Ordering::Relaxed);
                        ring_track.ring_band[channel_idx]
                            .store(band.to_bits(), Ordering::Relaxed);
                    }
                    ring_track
                        .ring_detune_phase
                        .store(phase.to_bits(), Ordering::Relaxed);
                }
            }
        }

        let metronome_active = self.metronome_enabled.load(Ordering::Relaxed)
            && (any_playing || any_recording || any_pending);
        if metronome_active {
            let num_buffer_samples = buffer.samples();
            let output = buffer.as_slice();
            let sr = self.tracks[0].sample_rate.load(Ordering::Relaxed).max(1);
            let tempo = global_tempo.clamp(20.0, 240.0);
            let samples_per_beat =
                ((sr as f32 * 60.0) / tempo.max(1.0)).round().max(1.0) as u32;
            let click_len = ((sr as f32) * (METRONOME_CLICK_MS / 1000.0))
                .round()
                .max(1.0) as u32;
            let mut phase = self.metronome_phase_samples;
            let mut click_remaining = self.metronome_click_remaining;
            for sample_idx in 0..num_buffer_samples {
                if phase == 0 {
                    click_remaining = click_len;
                }
                if click_remaining > 0 {
                    let env = click_remaining as f32 / click_len as f32;
                    let click = METRONOME_CLICK_GAIN * env;
                    for channel_idx in 0..output.len() {
                        output[channel_idx][sample_idx] += click;
                    }
                    click_remaining = click_remaining.saturating_sub(1);
                }
                phase += 1;
                if phase >= samples_per_beat {
                    phase = 0;
                }
            }
            self.metronome_phase_samples = phase;
            self.metronome_click_remaining = click_remaining;
            keep_alive = true;
        }

        // Apply global gain
        for channel_samples in buffer.iter_samples() {
            let gain = self.params.gain.smoothed.next();

            for sample in channel_samples {
                *sample *= gain;
            }
        }

        for channel_samples in buffer.iter_samples() {
            for sample in channel_samples {
                if !sample.is_finite() {
                    *sample = 0.0;
                }
            }
        }

        // Update master output meters + visualizer data.
        if !buffer.is_empty() {
            let output = buffer.as_slice_immutable();
            let left = output.get(0).map(|ch| ch.as_ref()).unwrap_or(&[]);
            let right = output
                .get(1)
                .map(|ch| ch.as_ref())
                .unwrap_or(left);

            let mut peak_left = 0.0_f32;
            for sample in left {
                let amp = sample.abs();
                if amp > peak_left {
                    peak_left = amp;
                }
            }

            let mut peak_right = 0.0_f32;
            for sample in right {
                let amp = sample.abs();
                if amp > peak_right {
                    peak_right = amp;
                }
            }

            let prev_left =
                f32::from_bits(self.master_meters.left.load(Ordering::Relaxed));
            let prev_right =
                f32::from_bits(self.master_meters.right.load(Ordering::Relaxed));

            let next_left = smooth_meter(prev_left, peak_left);
            let next_right = smooth_meter(prev_right, peak_right);

            self.master_meters
                .left
                .store(next_left.to_bits(), Ordering::Relaxed);
            self.master_meters
                .right
                .store(next_right.to_bits(), Ordering::Relaxed);

            let total_samples = left.len().min(right.len());
            if total_samples > 0 {
                let scope_stride = (total_samples / OSCILLOSCOPE_SAMPLES).max(1);
                if let Some(mut scope) = self.visualizer.oscilloscope.try_lock() {
                    for (i, slot) in scope.iter_mut().enumerate() {
                        let idx = i * scope_stride;
                        *slot = left.get(idx).copied().unwrap_or(0.0);
                    }
                }

                let vector_stride = (total_samples / VECTORSCOPE_POINTS).max(1);
                if let (Some(mut xs), Some(mut ys)) = (
                    self.visualizer.vectorscope_x.try_lock(),
                    self.visualizer.vectorscope_y.try_lock(),
                ) {
                    for i in 0..VECTORSCOPE_POINTS {
                        let idx = i * vector_stride;
                        xs[i] = left.get(idx).copied().unwrap_or(0.0);
                        ys[i] = right.get(idx).copied().unwrap_or(0.0);
                    }
                }

                let window_len = SPECTRUM_WINDOW.min(total_samples);
                if window_len >= 2 {
                    let bins = SPECTRUM_BINS.min(window_len / 2);
                    if let Some(mut spectrum) = self.visualizer.spectrum.try_lock() {
                        for bin in 0..bins {
                            let mut re = 0.0_f32;
                            let mut im = 0.0_f32;
                            let bin_f = bin as f32;
                            let win_f = window_len as f32;
                            for i in 0..window_len {
                                let sample = left[i];
                                let phase = 2.0 * PI * bin_f * (i as f32) / win_f;
                                re += sample * phase.cos();
                                im -= sample * phase.sin();
                            }
                            let mag = (re * re + im * im).sqrt() / window_len as f32;
                            spectrum[bin] = mag.clamp(0.0, 1.0);
                        }
                        for bin in bins..SPECTRUM_BINS {
                            spectrum[bin] = 0.0;
                        }
                    }
                }
            }
        }

        if keep_alive {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}

fn wrap_loop_pos(
    mut pos: f32,
    loop_start: usize,
    loop_end: usize,
    loop_active: bool,
    num_samples: usize,
) -> f32 {
    if num_samples == 0 {
        return 0.0;
    }
    if loop_active && loop_end > loop_start {
        let loop_len = (loop_end - loop_start) as f32;
        let start = loop_start as f32;
        let end = loop_end as f32;
        while pos < start {
            pos += loop_len;
        }
        while pos >= end {
            pos -= loop_len;
        }
        pos
    } else {
        pos.clamp(0.0, (num_samples - 1) as f32)
    }
}

fn next_mosaic_rng(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

fn next_mosaic_rand_unit(state: &mut u32) -> f32 {
    let value = next_mosaic_rng(state);
    value as f32 / u32::MAX as f32
}

fn mosaic_cc_bipolar(value: f32) -> f32 {
    let cc = (value.clamp(0.0, 1.0) * 127.0).round();
    ((cc - 64.0) / 64.0).clamp(-1.0, 1.0)
}

fn sample_at_linear_ring(buffer: &[Vec<f32>], channel: usize, pos: f32) -> f32 {
    if buffer.is_empty() {
        return 0.0;
    }
    let channel = channel.min(buffer.len() - 1);
    let len = buffer[channel].len();
    if len == 0 {
        return 0.0;
    }
    let mut idx = pos.floor();
    if idx < 0.0 {
        idx = 0.0;
    }
    let idx0 = (idx as usize) % len;
    let idx1 = (idx0 + 1) % len;
    let frac = (pos - idx0 as f32).clamp(0.0, 1.0);
    let a = buffer[channel][idx0];
    let b = buffer[channel][idx1];
    a + (b - a) * frac
}

fn open_docs() {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.to_path_buf()));

    let mut candidates = Vec::new();
    if let Some(dir) = exe_dir.clone() {
        candidates.push(dir.join("documentation").join("index.html"));
        #[cfg(target_os = "macos")]
        {
            if let Some(contents) = dir.parent() {
                candidates.push(contents.join("Resources").join("documentation").join("index.html"));
            }
        }
    }
    candidates.push(std::env::current_dir().unwrap_or_default().join("documentation").join("index.html"));

    let doc_path = candidates.into_iter().find(|path| path.exists());
    let Some(doc_path) = doc_path else {
        eprintln!("Documentation not found. Expected documentation/index.html next to the app.");
        return;
    };

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", doc_path.to_string_lossy().as_ref()])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(doc_path)
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(doc_path)
            .spawn();
    }
}

fn smooth_meter(prev: f32, target: f32) -> f32 {
    let attack = 0.6;
    let release = 0.96;
    let next = if target > prev {
        prev + (target - prev) * attack
    } else {
        prev * release + target * (1.0 - release)
    };
    next.clamp(0.0, 1.0)
}

fn smooth_param(current: f32, target: f32, num_samples: usize, sample_rate: f32) -> f32 {
    let smoothing_samples =
        (sample_rate * (MOSAIC_PARAM_SMOOTH_MS / 1000.0)).max(1.0);
    let step = (target - current) / smoothing_samples;
    let mut next = current + step * num_samples as f32;
    if (target - current).signum() != (target - next).signum() {
        next = target;
    }
    next
}

fn count_in_samples(tempo: f32, sample_rate: u32, ticks: u32) -> u32 {
    if ticks == 0 {
        return 0;
    }
    let tempo = tempo.clamp(20.0, 300.0);
    let sr = sample_rate.max(1) as f32;
    let samples_per_beat = (sr * 60.0 / tempo.max(1.0)).max(1.0);
    let ticks = ticks.min(METRONOME_COUNT_IN_MAX_TICKS) as f32;
    (samples_per_beat * ticks).round().max(1.0) as u32
}

fn build_time_labels(duration_secs: f32) -> Vec<SharedString> {
    let d = duration_secs.max(0.0);
    let marks = [0.0, 0.25, 0.5, 0.75, 1.0];
    marks
        .iter()
        .map(|t| {
            let seconds = d * t;
            if seconds >= 10.0 {
                SharedString::from(format!("{:.0}s", seconds))
            } else {
                SharedString::from(format!("{:.1}s", seconds))
            }
        })
        .collect()
}

fn sample_at_linear(
    samples: &[Vec<f32>],
    channel: usize,
    pos: f32,
    loop_start: usize,
    loop_end: usize,
    loop_active: bool,
    num_samples: usize,
) -> f32 {
    if samples.is_empty() || num_samples == 0 {
        return 0.0;
    }
    let pos = wrap_loop_pos(pos, loop_start, loop_end, loop_active, num_samples);
    let idx0 = pos.floor() as usize;
    let frac = pos - idx0 as f32;
    let idx1 = if loop_active && loop_end > loop_start {
        let end = loop_end.min(num_samples);
        if idx0 + 1 < end {
            idx0 + 1
        } else {
            loop_start.min(num_samples.saturating_sub(1))
        }
    } else {
        (idx0 + 1).min(num_samples.saturating_sub(1))
    };
    let s0 = samples[channel][idx0];
    let s1 = samples[channel][idx1];
    s0 + (s1 - s0) * frac
}

fn load_audio_file(
    path: &std::path::Path,
) -> Result<(Vec<Vec<f32>>, u32), Box<dyn std::error::Error>> {
    use symphonia::core::audio::AudioBufferRef;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let meta_opts = MetadataOptions::default();
    let fmt_opts = FormatOptions::default();
    let probed = symphonia::default::get_probe().format(&hint, mss, &fmt_opts, &meta_opts)?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "no supported audio tracks")?;

    let dec_opts = DecoderOptions::default();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &dec_opts)?;

    let mut samples: Vec<Vec<f32>> = vec![];
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let track_id = track.id;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(Box::new(e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder.decode(&packet)?;

        if samples.is_empty() {
            samples = vec![vec![]; decoded.spec().channels.count()];
        }

        match decoded {
            AudioBufferRef::F32(buf) => {
                for (i, plane) in buf.planes().planes().iter().enumerate() {
                    samples[i].extend_from_slice(plane);
                }
            }
            _ => {
                let mut buf = decoded.make_equivalent::<f32>();
                decoded.convert(&mut buf);
                for (i, plane) in buf.planes().planes().iter().enumerate() {
                    samples[i].extend_from_slice(plane);
                }
            }
        }
    }

    Ok((samples, sample_rate))
}

fn save_track_sample(track: &Track, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let samples = track.samples.lock();
    if samples.is_empty() || samples[0].is_empty() {
        return Err("No sample data to save".into());
    }
    let num_channels = samples.len().max(1);
    let num_samples = samples[0].len();
    let spec = hound::WavSpec {
        channels: num_channels as u16,
        sample_rate: 44100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for i in 0..num_samples {
        for ch in 0..num_channels {
            let sample = samples.get(ch).and_then(|buf| buf.get(i)).copied().unwrap_or(0.0);
            writer.write_sample(sample)?;
        }
    }
    writer.finalize()?;
    Ok(())
}

fn default_one() -> f32 {
    1.0
}

fn default_tempo() -> f32 {
    120.0
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    version: u32,
    #[serde(default = "default_tempo")]
    global_tempo: f32,
    tracks: Vec<ProjectTrack>,
}

#[derive(Serialize, Deserialize)]
struct ProjectTrack {
    sample_path: Option<String>,
    level: f32,
    muted: bool,
    #[serde(default = "default_one")]
    tape_speed: f32,
    #[serde(default = "default_tempo")]
    tape_tempo: f32,
    #[serde(default)]
    tape_rate_mode: u32,
    #[serde(default)]
    tape_rotate: f32,
    #[serde(default)]
    tape_glide: f32,
    #[serde(default)]
    tape_sos: f32,
    #[serde(default)]
    tape_reverse: bool,
    #[serde(default)]
    tape_freeze: bool,
    #[serde(default)]
    tape_keylock: bool,
    #[serde(default)]
    tape_monitor: bool,
    #[serde(default)]
    tape_overdub: bool,
    loop_start: f32,
    loop_length: f32,
    loop_xfade: f32,
    loop_enabled: bool,
    #[serde(default)]
    loop_mode: u32,
}

fn save_project(
    tracks: &Arc<[Track; NUM_TRACKS]>,
    global_tempo: f32,
    path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut track_states = Vec::with_capacity(NUM_TRACKS);
    for track in tracks.iter() {
        let sample_path = track
            .sample_path
            .lock()
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        track_states.push(ProjectTrack {
            sample_path,
            level: f32::from_bits(track.level.load(Ordering::Relaxed)),
            muted: track.is_muted.load(Ordering::Relaxed),
            tape_speed: f32::from_bits(track.tape_speed.load(Ordering::Relaxed)),
            tape_tempo: global_tempo,
            tape_rate_mode: track.tape_rate_mode.load(Ordering::Relaxed),
            tape_rotate: f32::from_bits(track.tape_rotate.load(Ordering::Relaxed)),
            tape_glide: f32::from_bits(track.tape_glide.load(Ordering::Relaxed)),
            tape_sos: f32::from_bits(track.tape_sos.load(Ordering::Relaxed)),
            tape_reverse: track.tape_reverse.load(Ordering::Relaxed),
            tape_freeze: track.tape_freeze.load(Ordering::Relaxed),
            tape_keylock: track.tape_keylock.load(Ordering::Relaxed),
            tape_monitor: track.tape_monitor.load(Ordering::Relaxed),
            tape_overdub: track.tape_overdub.load(Ordering::Relaxed),
            loop_start: f32::from_bits(track.loop_start.load(Ordering::Relaxed)),
            loop_length: f32::from_bits(track.loop_length.load(Ordering::Relaxed)),
            loop_xfade: f32::from_bits(track.loop_xfade.load(Ordering::Relaxed)),
            loop_enabled: track.loop_enabled.load(Ordering::Relaxed),
            loop_mode: track.loop_mode.load(Ordering::Relaxed),
        });
    }

    let project = ProjectFile {
        version: 1,
        global_tempo,
        tracks: track_states,
    };
    let json = serde_json::to_string_pretty(&project)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn load_project(
    tracks: &Arc<[Track; NUM_TRACKS]>,
    global_tempo: &Arc<AtomicU32>,
    path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(path)?;
    let project: ProjectFile = serde_json::from_str(&json)?;
    let tempo = if project.global_tempo.is_finite() {
        project.global_tempo
    } else {
        default_tempo()
    };
    global_tempo.store(tempo.to_bits(), Ordering::Relaxed);
    for (track_idx, track_state) in project.tracks.iter().enumerate() {
        if track_idx >= NUM_TRACKS {
            break;
        }
        let track = &tracks[track_idx];
        track.level.store(track_state.level.to_bits(), Ordering::Relaxed);
        track
            .is_muted
            .store(track_state.muted, Ordering::Relaxed);
        track.tape_speed.store(track_state.tape_speed.to_bits(), Ordering::Relaxed);
        track.tape_speed_smooth.store(track_state.tape_speed.to_bits(), Ordering::Relaxed);
        track.tape_tempo.store(tempo.to_bits(), Ordering::Relaxed);
        track.tape_rate_mode.store(track_state.tape_rate_mode, Ordering::Relaxed);
        track.tape_rotate.store(track_state.tape_rotate.to_bits(), Ordering::Relaxed);
        track.tape_glide.store(track_state.tape_glide.to_bits(), Ordering::Relaxed);
        track.tape_sos.store(track_state.tape_sos.to_bits(), Ordering::Relaxed);
        track.tape_reverse.store(track_state.tape_reverse, Ordering::Relaxed);
        track.tape_freeze.store(track_state.tape_freeze, Ordering::Relaxed);
        track.tape_keylock.store(track_state.tape_keylock, Ordering::Relaxed);
        track.tape_monitor.store(track_state.tape_monitor, Ordering::Relaxed);
        track.tape_overdub.store(track_state.tape_overdub, Ordering::Relaxed);
        track.loop_start.store(
            track_state.loop_start.clamp(0.0, 0.999).to_bits(),
            Ordering::Relaxed,
        );
        track.loop_length.store(
            track_state.loop_length.clamp(0.0, 1.0).to_bits(),
            Ordering::Relaxed,
        );
        track.loop_xfade.store(
            track_state.loop_xfade.clamp(0.0, 0.5).to_bits(),
            Ordering::Relaxed,
        );
        track
            .loop_enabled
            .store(track_state.loop_enabled, Ordering::Relaxed);
        track
            .loop_mode
            .store(track_state.loop_mode, Ordering::Relaxed);
        track.loop_dir.store(1, Ordering::Relaxed);
        track.engine_type.store(1, Ordering::Relaxed);
        track.is_playing.store(false, Ordering::Relaxed);
        track.is_recording.store(false, Ordering::Relaxed);
        track.play_pos.store(0.0f32.to_bits(), Ordering::Relaxed);
        track.debug_logged.store(false, Ordering::Relaxed);

        let mut samples = track.samples.lock();
        let mut summary = track.waveform_summary.lock();
        let mut sample_path = track.sample_path.lock();
        if let Some(path_str) = &track_state.sample_path {
            let path = PathBuf::from(path_str);
            match load_audio_file(&path) {
                Ok((new_samples, sample_rate)) => {
                    *samples = new_samples;
                    *sample_path = Some(path);
                    track.sample_rate.store(sample_rate, Ordering::Relaxed);
                    if !samples.is_empty() {
                        calculate_waveform_summary(&samples[0], &mut summary);
                    }
                }
                Err(err) => {
                    nih_log!("Failed to load sample for track {}: {:?}", track_idx, err);
                    *samples = vec![vec![]; 2];
                    *summary = vec![0.0; WAVEFORM_SUMMARY_SIZE];
                    *sample_path = None;
                    track.sample_rate.store(44_100, Ordering::Relaxed);
                }
            }
        } else {
            *samples = vec![vec![]; 2];
            *summary = vec![0.0; WAVEFORM_SUMMARY_SIZE];
            *sample_path = None;
            track.sample_rate.store(44_100, Ordering::Relaxed);
        }
    }
    Ok(())
}

struct SlintEditor {
    params: Arc<GrainRustParams>,
    tracks: Arc<[Track; NUM_TRACKS]>,
    master_meters: Arc<MasterMeters>,
    visualizer: Arc<VisualizerState>,
    global_tempo: Arc<AtomicU32>,
    follow_host_tempo: Arc<AtomicBool>,
    metronome_enabled: Arc<AtomicBool>,
    metronome_count_in_ticks: Arc<AtomicU32>,
    metronome_count_in_playback: Arc<AtomicBool>,
    metronome_count_in_record: Arc<AtomicBool>,
    async_executor: AsyncExecutor<GrainRust>,
}

impl Editor for SlintEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let params = self.params.clone();
        let tracks = self.tracks.clone();
        let master_meters = self.master_meters.clone();
        let visualizer = self.visualizer.clone();
        let global_tempo = self.global_tempo.clone();
        let follow_host_tempo = self.follow_host_tempo.clone();
        let metronome_enabled = self.metronome_enabled.clone();
        let metronome_count_in_ticks = self.metronome_count_in_ticks.clone();
        let metronome_count_in_playback = self.metronome_count_in_playback.clone();
        let metronome_count_in_record = self.metronome_count_in_record.clone();
        let async_executor = self.async_executor.clone();

        let initial_size = default_window_size();
        let window_handle = baseview::Window::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: "GrainRust".to_string(),
                size: initial_size,
                scale: WindowScalePolicy::SystemScaleFactor,
                gl_config: None,
            },
            move |window| {
                SlintWindow::new(
                    window,
                    initial_size,
                    context,
                    params,
                    tracks,
                    master_meters,
                    visualizer,
                    global_tempo,
                    follow_host_tempo,
                    metronome_enabled,
                    metronome_count_in_ticks,
                    metronome_count_in_playback,
                    metronome_count_in_record,
                    async_executor,
                )
            },
        );

        Box::new(SlintEditorHandle { window: window_handle })
    }

    fn size(&self) -> (u32, u32) {
        let size = default_window_size();
        (size.width as u32, size.height as u32)
    }

    fn set_scale_factor(&self, _factor: f32) -> bool {
        false
    }

    fn param_values_changed(&self) {}

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {}

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}
}

struct SlintEditorHandle {
    window: WindowHandle,
}

unsafe impl Send for SlintEditorHandle {}

impl Drop for SlintEditorHandle {
    fn drop(&mut self) {
        self.window.close();
    }
}

struct SlintWindow {
    gui_context: Arc<dyn GuiContext>,
    params: Arc<GrainRustParams>,
    tracks: Arc<[Track; NUM_TRACKS]>,
    master_meters: Arc<MasterMeters>,
    visualizer: Arc<VisualizerState>,
    global_tempo: Arc<AtomicU32>,
    follow_host_tempo: Arc<AtomicBool>,
    metronome_enabled: Arc<AtomicBool>,
    metronome_count_in_ticks: Arc<AtomicU32>,
    metronome_count_in_playback: Arc<AtomicBool>,
    metronome_count_in_record: Arc<AtomicBool>,
    async_executor: AsyncExecutor<GrainRust>,
    slint_window: std::rc::Rc<MinimalSoftwareWindow>,
    ui: Box<GrainRustUI>,
    waveform_model: std::rc::Rc<VecModel<f32>>,
    oscilloscope_model: std::rc::Rc<VecModel<f32>>,
    spectrum_model: std::rc::Rc<VecModel<f32>>,
    vectorscope_x_model: std::rc::Rc<VecModel<f32>>,
    vectorscope_y_model: std::rc::Rc<VecModel<f32>>,
    sample_dialog_rx: std::sync::mpsc::Receiver<SampleDialogAction>,
    project_dialog_rx: std::sync::mpsc::Receiver<ProjectDialogAction>,
    sb_surface: softbuffer::Surface<SoftbufferWindowHandleAdapter, SoftbufferWindowHandleAdapter>,
    _sb_context: softbuffer::Context<SoftbufferWindowHandleAdapter>,
    physical_width: u32,
    physical_height: u32,
    scale_factor: f32,
    pixel_buffer: Vec<PremultipliedRgbaColor>,
    last_cursor: LogicalPosition,
}

impl SlintWindow {
    fn new(
        window: &mut BaseWindow,
        initial_size: baseview::Size,
        gui_context: Arc<dyn GuiContext>,
        params: Arc<GrainRustParams>,
        tracks: Arc<[Track; NUM_TRACKS]>,
        master_meters: Arc<MasterMeters>,
        visualizer: Arc<VisualizerState>,
        global_tempo: Arc<AtomicU32>,
        follow_host_tempo: Arc<AtomicBool>,
        metronome_enabled: Arc<AtomicBool>,
        metronome_count_in_ticks: Arc<AtomicU32>,
        metronome_count_in_playback: Arc<AtomicBool>,
        metronome_count_in_record: Arc<AtomicBool>,
        async_executor: AsyncExecutor<GrainRust>,
    ) -> Self {
        ensure_slint_platform();
        let (slint_window, ui) = create_slint_ui();
        let waveform_model =
            std::rc::Rc::new(VecModel::from(vec![0.0; WAVEFORM_SUMMARY_SIZE]));
        let oscilloscope_model =
            std::rc::Rc::new(VecModel::from(vec![0.0; OSCILLOSCOPE_SAMPLES]));
        let spectrum_model = std::rc::Rc::new(VecModel::from(vec![0.0; SPECTRUM_BINS]));
        let vectorscope_x_model =
            std::rc::Rc::new(VecModel::from(vec![0.0; VECTORSCOPE_POINTS]));
        let vectorscope_y_model =
            std::rc::Rc::new(VecModel::from(vec![0.0; VECTORSCOPE_POINTS]));
        ui.set_waveform(ModelRc::from(waveform_model.clone()));
        ui.set_oscilloscope(ModelRc::from(oscilloscope_model.clone()));
        ui.set_spectrum(ModelRc::from(spectrum_model.clone()));
        ui.set_vectorscope_x(ModelRc::from(vectorscope_x_model.clone()));
        ui.set_vectorscope_y(ModelRc::from(vectorscope_y_model.clone()));
        let (sample_dialog_tx, sample_dialog_rx) = mpsc::channel();
        let (project_dialog_tx, project_dialog_rx) = mpsc::channel();

        let mut scale_factor = 1.0_f32;
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MAXIMIZE};
            use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
            if let RawWindowHandle::Win32(handle) = window.raw_window_handle() {
                unsafe {
                    let dpi = GetDpiForWindow(handle.hwnd as isize) as f32;
                    if dpi > 0.0 {
                        scale_factor = dpi / 96.0;
                    }
                    ShowWindow(handle.hwnd as isize, SW_MAXIMIZE);
                }
            }
        }

        let logical_width = initial_size.width as f32;
        let logical_height = initial_size.height as f32;
        let physical_width = (logical_width * scale_factor).round() as u32;
        let physical_height = (logical_height * scale_factor).round() as u32;

        follow_host_tempo.store(
            gui_context.plugin_api() != PluginApi::Standalone,
            Ordering::Relaxed,
        );

        let target = baseview_window_to_surface_target(window);
        let sb_context =
            softbuffer::Context::new(target.clone()).expect("Failed to create softbuffer context");
        let mut sb_surface = softbuffer::Surface::new(&sb_context, target)
            .expect("Failed to create softbuffer surface");
        sb_surface
            .resize(
                std::num::NonZeroU32::new(physical_width).unwrap(),
                std::num::NonZeroU32::new(physical_height).unwrap(),
            )
            .unwrap();

        slint_window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor });
        slint_window.set_size(PhysicalSize::new(physical_width, physical_height));

        let output_devices = available_output_devices();
        let input_devices = available_input_devices();
        let sample_rates = vec![44100, 48000, 88200, 96000];
        let buffer_sizes = vec![256, 512, 1024, 2048, 4096];

        initialize_ui(
            &ui,
            &gui_context,
            &params,
            &tracks,
            &global_tempo,
            &follow_host_tempo,
            &metronome_enabled,
            &metronome_count_in_ticks,
            &metronome_count_in_playback,
            &metronome_count_in_record,
            &async_executor,
            &output_devices,
            &input_devices,
            &sample_rates,
            &buffer_sizes,
            sample_dialog_tx,
            project_dialog_tx,
        );

        Self {
            gui_context,
            params,
            tracks,
            master_meters,
            visualizer,
            global_tempo,
            follow_host_tempo,
            metronome_enabled,
            metronome_count_in_ticks,
            metronome_count_in_playback,
            metronome_count_in_record,
            async_executor,
            slint_window,
            ui,
            waveform_model,
            oscilloscope_model,
            spectrum_model,
            vectorscope_x_model,
            vectorscope_y_model,
            sample_dialog_rx,
            project_dialog_rx,
            sb_surface,
            _sb_context: sb_context,
            physical_width,
            physical_height,
            scale_factor,
            pixel_buffer: vec![PremultipliedRgbaColor::default(); (physical_width * physical_height) as usize],
            last_cursor: LogicalPosition::new(0.0, 0.0),
        }
    }

    fn dispatch_slint_event(&self, event: WindowEvent) {
        self.slint_window.dispatch_event(event);
    }

    fn update_ui_state(&mut self) {
        let track_idx = self
            .params
            .selected_track
            .value()
            .saturating_sub(1) as usize;
        let track_idx = track_idx.min(NUM_TRACKS - 1);

        let is_playing = self
            .tracks
            .iter()
            .any(|track| track.is_playing.load(Ordering::Relaxed));
        let is_recording = self.tracks[track_idx].is_recording.load(Ordering::Relaxed);
        let gain = self.params.gain.unmodulated_normalized_value();
        let track_level =
            f32::from_bits(self.tracks[track_idx].level.load(Ordering::Relaxed));
        let meter_left =
            f32::from_bits(self.master_meters.left.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let meter_right =
            f32::from_bits(self.master_meters.right.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let track_meter_left =
            f32::from_bits(self.tracks[track_idx].meter_left.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let track_meter_right =
            f32::from_bits(self.tracks[track_idx].meter_right.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let track_muted = self.tracks[track_idx].is_muted.load(Ordering::Relaxed);
        let tape_speed =
            f32::from_bits(self.tracks[track_idx].tape_speed.load(Ordering::Relaxed));
        let tape_tempo =
            f32::from_bits(self.global_tempo.load(Ordering::Relaxed));
        let metronome_enabled =
            self.metronome_enabled.load(Ordering::Relaxed);
        let metronome_count_in_ticks =
            self.metronome_count_in_ticks.load(Ordering::Relaxed);
        let metronome_count_in_playback =
            self.metronome_count_in_playback.load(Ordering::Relaxed);
        let metronome_count_in_record =
            self.metronome_count_in_record.load(Ordering::Relaxed);
        let tape_rate_mode =
            self.tracks[track_idx].tape_rate_mode.load(Ordering::Relaxed);
        let tape_rotate =
            f32::from_bits(self.tracks[track_idx].tape_rotate.load(Ordering::Relaxed));
        let tape_glide =
            f32::from_bits(self.tracks[track_idx].tape_glide.load(Ordering::Relaxed));
        let tape_sos =
            f32::from_bits(self.tracks[track_idx].tape_sos.load(Ordering::Relaxed));
        let tape_reverse =
            self.tracks[track_idx].tape_reverse.load(Ordering::Relaxed);
        let tape_freeze =
            self.tracks[track_idx].tape_freeze.load(Ordering::Relaxed);
        let tape_keylock =
            self.tracks[track_idx].tape_keylock.load(Ordering::Relaxed);
        let tape_monitor =
            self.tracks[track_idx].tape_monitor.load(Ordering::Relaxed);
        let tape_overdub =
            self.tracks[track_idx].tape_overdub.load(Ordering::Relaxed);
        let mosaic_pitch =
            f32::from_bits(self.tracks[track_idx].mosaic_pitch.load(Ordering::Relaxed));
        let mosaic_rate =
            f32::from_bits(self.tracks[track_idx].mosaic_rate.load(Ordering::Relaxed));
        let mosaic_size =
            f32::from_bits(self.tracks[track_idx].mosaic_size.load(Ordering::Relaxed));
        let mosaic_contour =
            f32::from_bits(self.tracks[track_idx].mosaic_contour.load(Ordering::Relaxed));
        let mosaic_warp =
            f32::from_bits(self.tracks[track_idx].mosaic_warp.load(Ordering::Relaxed));
        let mosaic_spray =
            f32::from_bits(self.tracks[track_idx].mosaic_spray.load(Ordering::Relaxed));
        let mosaic_pattern =
            f32::from_bits(self.tracks[track_idx].mosaic_pattern.load(Ordering::Relaxed));
        let mosaic_wet =
            f32::from_bits(self.tracks[track_idx].mosaic_wet.load(Ordering::Relaxed));
        let mosaic_detune =
            f32::from_bits(self.tracks[track_idx].mosaic_detune.load(Ordering::Relaxed));
        let mosaic_rand_rate =
            f32::from_bits(self.tracks[track_idx].mosaic_rand_rate.load(Ordering::Relaxed));
        let mosaic_rand_size =
            f32::from_bits(self.tracks[track_idx].mosaic_rand_size.load(Ordering::Relaxed));
        let mosaic_sos =
            f32::from_bits(self.tracks[track_idx].mosaic_sos.load(Ordering::Relaxed));
        let ring_cutoff =
            f32::from_bits(self.tracks[track_idx].ring_cutoff.load(Ordering::Relaxed));
        let ring_resonance =
            f32::from_bits(self.tracks[track_idx].ring_resonance.load(Ordering::Relaxed));
        let ring_decay =
            f32::from_bits(self.tracks[track_idx].ring_decay.load(Ordering::Relaxed));
        let ring_pitch =
            f32::from_bits(self.tracks[track_idx].ring_pitch.load(Ordering::Relaxed));
        let ring_tone =
            f32::from_bits(self.tracks[track_idx].ring_tone.load(Ordering::Relaxed));
        let ring_tilt =
            f32::from_bits(self.tracks[track_idx].ring_tilt.load(Ordering::Relaxed));
        let ring_slope =
            f32::from_bits(self.tracks[track_idx].ring_slope.load(Ordering::Relaxed));
        let ring_wet =
            f32::from_bits(self.tracks[track_idx].ring_wet.load(Ordering::Relaxed));
        let ring_detune =
            f32::from_bits(self.tracks[track_idx].ring_detune.load(Ordering::Relaxed));
        let loop_start =
            f32::from_bits(self.tracks[track_idx].loop_start.load(Ordering::Relaxed));
        let loop_length =
            f32::from_bits(self.tracks[track_idx].loop_length.load(Ordering::Relaxed));
        let loop_xfade =
            f32::from_bits(self.tracks[track_idx].loop_xfade.load(Ordering::Relaxed));
        let loop_enabled =
            self.tracks[track_idx].loop_enabled.load(Ordering::Relaxed);
        let loop_mode = self.tracks[track_idx].loop_mode.load(Ordering::Relaxed);
        let mosaic_enabled = self.tracks[track_idx].mosaic_enabled.load(Ordering::Relaxed);
        let ring_enabled = self.tracks[track_idx].ring_enabled.load(Ordering::Relaxed);
        let ring_decay_mode = self.tracks[track_idx].ring_decay_mode.load(Ordering::Relaxed);
        let engine_loaded = self.tracks[track_idx].engine_type.load(Ordering::Relaxed) != 0;

        let play_pos = f32::from_bits(self.tracks[track_idx].play_pos.load(Ordering::Relaxed));
        let total_samples = if let Some(samples) = self.tracks[track_idx].samples.try_lock() {
            samples.get(0).map(|ch| ch.len()).unwrap_or(0)
        } else {
            0
        };
        let sample_rate = self.tracks[track_idx].sample_rate.load(Ordering::Relaxed).max(1);
        let duration_secs = total_samples as f32 / sample_rate as f32;
        let playhead_index = if total_samples > 0 {
            ((play_pos / total_samples as f32) * WAVEFORM_SUMMARY_SIZE as f32) as i32
        } else {
            0
        };

        let waveform = if let Some(summary) = self.tracks[track_idx].waveform_summary.try_lock() {
            summary.clone()
        } else {
            vec![0.0; WAVEFORM_SUMMARY_SIZE]
        };

        let oscilloscope = if let Some(scope) = self.visualizer.oscilloscope.try_lock() {
            scope.clone()
        } else {
            vec![0.0; OSCILLOSCOPE_SAMPLES]
        };
        let spectrum = if let Some(spec) = self.visualizer.spectrum.try_lock() {
            spec.clone()
        } else {
            vec![0.0; SPECTRUM_BINS]
        };
        let vectorscope_x = if let Some(points) = self.visualizer.vectorscope_x.try_lock() {
            points.clone()
        } else {
            vec![0.0; VECTORSCOPE_POINTS]
        };
        let vectorscope_y = if let Some(points) = self.visualizer.vectorscope_y.try_lock() {
            points.clone()
        } else {
            vec![0.0; VECTORSCOPE_POINTS]
        };

        self.ui.set_selected_track((track_idx + 1) as i32);
        self.ui.set_is_playing(is_playing);
        self.ui.set_is_recording(is_recording);
        self.ui.set_gain(gain);
        self.ui.set_track_level(track_level);
        self.ui.set_track_muted(track_muted);
        self.ui.set_meter_left(meter_left);
        self.ui.set_meter_right(meter_right);
        self.ui.set_track_meter_left(track_meter_left);
        self.ui.set_track_meter_right(track_meter_right);
        self.ui.set_tape_speed(tape_speed);
        self.ui.set_tape_tempo(tape_tempo);
        self.ui.set_tempo_label(SharedString::from(format!("{tape_tempo:.0} BPM")));
        self.ui.set_tape_rate_mode(tape_rate_mode as i32);
        self.ui.set_tape_rotate(tape_rotate);
        self.ui.set_tape_glide(tape_glide);
        self.ui.set_tape_sos(tape_sos);
        self.ui.set_tape_reverse(tape_reverse);
        self.ui.set_tape_freeze(tape_freeze);
        self.ui.set_tape_keylock(tape_keylock);
        self.ui.set_tape_monitor(tape_monitor);
        self.ui.set_tape_overdub(tape_overdub);
        self.ui.set_mosaic_pitch(mosaic_pitch);
        self.ui.set_mosaic_rate(mosaic_rate);
        self.ui.set_mosaic_size(mosaic_size);
        self.ui.set_mosaic_contour(mosaic_contour);
        self.ui.set_mosaic_warp(mosaic_warp);
        self.ui.set_mosaic_spray(mosaic_spray);
        self.ui.set_mosaic_pattern(mosaic_pattern);
        self.ui.set_mosaic_wet(mosaic_wet);
        self.ui.set_mosaic_detune(mosaic_detune);
        self.ui.set_mosaic_rand_rate(mosaic_rand_rate);
        self.ui.set_mosaic_rand_size(mosaic_rand_size);
        self.ui.set_mosaic_sos(mosaic_sos);
        self.ui.set_ring_cutoff(ring_cutoff);
        self.ui.set_ring_resonance(ring_resonance);
        self.ui.set_ring_decay(ring_decay);
        self.ui.set_ring_pitch(ring_pitch);
        self.ui.set_ring_tone(ring_tone);
        self.ui.set_ring_tilt(ring_tilt);
        self.ui.set_ring_slope(ring_slope);
        self.ui.set_ring_wet(ring_wet);
        self.ui.set_ring_detune(ring_detune);
        self.ui.set_loop_start(loop_start);
        self.ui.set_loop_length(loop_length);
        self.ui.set_loop_xfade(loop_xfade);
        self.ui.set_loop_enabled(loop_enabled);
        self.ui.set_loop_mode(loop_mode as i32);
        self.ui.set_mosaic_enabled(mosaic_enabled);
        self.ui.set_ring_enabled(ring_enabled);
        self.ui.set_ring_decay_mode(ring_decay_mode as i32);
        self.ui.set_engine_loaded(engine_loaded);
        self.ui.set_metronome_enabled(metronome_enabled);
        self.ui
            .set_metronome_count_in(metronome_count_in_ticks as f32);
        self.ui
            .set_metronome_count_in_label(SharedString::from(format!(
                "Count-in: {metronome_count_in_ticks} ticks"
            )));
        self.ui
            .set_metronome_count_playback(metronome_count_in_playback);
        self.ui
            .set_metronome_count_record(metronome_count_in_record);

        self.ui.set_playhead_index(playhead_index);
        self.waveform_model.set_vec(waveform);
        self.oscilloscope_model.set_vec(oscilloscope);
        self.spectrum_model.set_vec(spectrum);
        self.vectorscope_x_model.set_vec(vectorscope_x);
        self.vectorscope_y_model.set_vec(vectorscope_y);
        self.ui
            .set_waveform_time_labels(ModelRc::new(VecModel::from(build_time_labels(
                duration_secs,
            ))));
    }

    fn render(&mut self) {
        let required_len = (self.physical_width * self.physical_height) as usize;
        if self.pixel_buffer.len() != required_len {
            self.pixel_buffer = vec![PremultipliedRgbaColor::default(); required_len];
        }

        self.slint_window.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixel_buffer, self.physical_width as usize);
        });

        let mut buffer = self.sb_surface.buffer_mut().unwrap();
        for (dst, src) in buffer.iter_mut().zip(self.pixel_buffer.iter()) {
            let value = (src.blue as u32)
                | ((src.green as u32) << 8)
                | ((src.red as u32) << 16)
                | ((src.alpha as u32) << 24);
            *dst = value;
        }
        buffer.present().unwrap();
    }

    fn resize(&mut self, window_info: baseview::WindowInfo) {
        self.scale_factor = window_info.scale() as f32;
        let _logical = window_info.logical_size();
        let physical = window_info.physical_size();

        self.physical_width = physical.width;
        self.physical_height = physical.height;

        self.slint_window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: self.scale_factor,
        });
        self.slint_window.set_size(PhysicalSize::new(
            self.physical_width,
            self.physical_height,
        ));

        let _ = self.sb_surface.resize(
            std::num::NonZeroU32::new(self.physical_width).unwrap(),
            std::num::NonZeroU32::new(self.physical_height).unwrap(),
        );
    }
}

impl BaseWindowHandler for SlintWindow {
    fn on_frame(&mut self, _window: &mut BaseWindow) {
        while let Ok(action) = self.sample_dialog_rx.try_recv() {
            match action {
                SampleDialogAction::Load { track_idx, path } => {
                    if track_idx < NUM_TRACKS {
                        if let Some(path) = path {
                            self.async_executor
                                .execute_background(GrainRustTask::LoadSample(track_idx, path));
                        }
                    }
                }
                SampleDialogAction::Save { track_idx, path } => {
                    if track_idx < NUM_TRACKS {
                        if let Err(err) = save_track_sample(&self.tracks[track_idx], &path) {
                            nih_log!("Failed to save sample: {:?}", err);
                        } else {
                            nih_log!("Saved sample: {:?}", path);
                        }
                    }
                }
            }
        }
        while let Ok(action) = self.project_dialog_rx.try_recv() {
            match action {
                ProjectDialogAction::Save(path) => {
                    self.async_executor
                        .execute_background(GrainRustTask::SaveProject(path));
                }
                ProjectDialogAction::Load(path) => {
                    self.async_executor
                        .execute_background(GrainRustTask::LoadProject(path));
                }
            }
        }
        platform::update_timers_and_animations();
        self.update_ui_state();
        self.slint_window.request_redraw();
        self.render();
    }

    fn on_event(&mut self, _window: &mut BaseWindow, event: BaseEvent) -> BaseEventStatus {
        match event {
            BaseEvent::Window(event) => match event {
                baseview::WindowEvent::Resized(info) => {
                    self.resize(info);
                    BaseEventStatus::Captured
                }
                baseview::WindowEvent::Focused => {
                    self.dispatch_slint_event(WindowEvent::WindowActiveChanged(true));
                    BaseEventStatus::Captured
                }
                baseview::WindowEvent::Unfocused => {
                    self.dispatch_slint_event(WindowEvent::WindowActiveChanged(false));
                    BaseEventStatus::Captured
                }
                baseview::WindowEvent::WillClose => {
                    self.dispatch_slint_event(WindowEvent::CloseRequested);
                    BaseEventStatus::Captured
                }
            },
            BaseEvent::Mouse(event) => {
                match event {
                    baseview::MouseEvent::CursorMoved { position, .. } => {
                        let cursor = LogicalPosition::new(position.x as f32, position.y as f32);
                        self.last_cursor = cursor;
                        self.dispatch_slint_event(WindowEvent::PointerMoved { position: cursor });
                    }
                    baseview::MouseEvent::ButtonPressed { button, .. } => {
                        if let Some(button) = map_mouse_button(button) {
                            self.dispatch_slint_event(WindowEvent::PointerPressed {
                                position: self.last_cursor,
                                button,
                            });
                        }
                    }
                    baseview::MouseEvent::ButtonReleased { button, .. } => {
                        if let Some(button) = map_mouse_button(button) {
                            self.dispatch_slint_event(WindowEvent::PointerReleased {
                                position: self.last_cursor,
                                button,
                            });
                        }
                    }
                    baseview::MouseEvent::WheelScrolled { delta, .. } => {
                        let (dx, dy) = match delta {
                            baseview::ScrollDelta::Lines { x, y } => (x * 32.0, y * 32.0),
                            baseview::ScrollDelta::Pixels { x, y } => (x, y),
                        };
                        self.dispatch_slint_event(WindowEvent::PointerScrolled {
                            position: self.last_cursor,
                            delta_x: dx,
                            delta_y: dy,
                        });
                    }
                    baseview::MouseEvent::CursorLeft => {
                        self.dispatch_slint_event(WindowEvent::PointerExited);
                    }
                    _ => {}
                }
                BaseEventStatus::Captured
            }
            BaseEvent::Keyboard(_) => BaseEventStatus::Ignored,
        }
    }
}

fn initialize_ui(
    ui: &GrainRustUI,
    gui_context: &Arc<dyn GuiContext>,
    params: &Arc<GrainRustParams>,
    tracks: &Arc<[Track; NUM_TRACKS]>,
    global_tempo: &Arc<AtomicU32>,
    _follow_host_tempo: &Arc<AtomicBool>,
    metronome_enabled: &Arc<AtomicBool>,
    metronome_count_in_ticks: &Arc<AtomicU32>,
    metronome_count_in_playback: &Arc<AtomicBool>,
    metronome_count_in_record: &Arc<AtomicBool>,
    _async_executor: &AsyncExecutor<GrainRust>,
    output_devices: &[String],
    input_devices: &[String],
    sample_rates: &[u32],
    buffer_sizes: &[u32],
    sample_dialog_tx: std::sync::mpsc::Sender<SampleDialogAction>,
    project_dialog_tx: std::sync::mpsc::Sender<ProjectDialogAction>,
) {
    ui.set_output_devices(ModelRc::new(VecModel::from(
        output_devices
            .iter()
            .map(|device| SharedString::from(device.as_str()))
            .collect::<Vec<_>>(),
    )));
    ui.set_input_devices(ModelRc::new(VecModel::from(
        input_devices
            .iter()
            .map(|device| SharedString::from(device.as_str()))
            .collect::<Vec<_>>(),
    )));

    ui.set_sample_rates(ModelRc::new(VecModel::from(
        sample_rates
            .iter()
            .map(|rate| SharedString::from(rate.to_string()))
            .collect::<Vec<_>>(),
    )));

    ui.set_buffer_sizes(ModelRc::new(VecModel::from(
        buffer_sizes
            .iter()
            .map(|size| SharedString::from(size.to_string()))
            .collect::<Vec<_>>(),
    )));
    ui.set_loop_modes(ModelRc::new(VecModel::from(vec![
        SharedString::from("Forward"),
        SharedString::from("Ping-Pong"),
        SharedString::from("One-Shot"),
        SharedString::from("Reverse"),
        SharedString::from("Random Start"),
        SharedString::from("Jump To"),
    ])));
    ui.set_visualizer_modes(ModelRc::new(VecModel::from(vec![
        SharedString::from("Oscilloscope"),
        SharedString::from("Spectrum"),
        SharedString::from("Vectorscope"),
    ])));
    ui.set_tape_rate_modes(ModelRc::new(VecModel::from(vec![
        SharedString::from("Free"),
        SharedString::from("Straight"),
        SharedString::from("Dotted"),
        SharedString::from("Triplet"),
    ])));
    ui.set_ring_decay_modes(ModelRc::new(VecModel::from(vec![
        SharedString::from("Sustain"),
        SharedString::from("Choke"),
    ])));
    ui.set_engine_types(ModelRc::new(VecModel::from(vec![
        SharedString::from("Tape"),
    ])));
    ui.set_engine_index(0);
    ui.set_engine_confirm_text(SharedString::from(
        "Loading a new engine will clear unsaved data for this track. Continue?",
    ));

    let output_device_index = current_arg_value("--output-device")
        .and_then(|name| output_devices.iter().position(|device| device == &name))
        .unwrap_or(0);
    let input_device_index = current_arg_value("--input-device")
        .and_then(|name| input_devices.iter().position(|device| device == &name))
        .unwrap_or(0);
    let sample_rate_index = current_arg_value("--sample-rate")
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(|rate| sample_rates.iter().position(|candidate| *candidate == rate))
        .unwrap_or(1);
    let buffer_size_index = current_arg_value("--period-size")
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(|size| buffer_sizes.iter().position(|candidate| *candidate == size))
        .unwrap_or(3);

    ui.set_output_device_index(output_device_index as i32);
    ui.set_input_device_index(input_device_index as i32);
    ui.set_sample_rate_index(sample_rate_index as i32);
    ui.set_buffer_size_index(buffer_size_index as i32);

    let ui_weak = ui.as_weak();
    let pending_engine = Arc::new(Mutex::new(None::<PendingEngineLoad>));

    let gui_context_select = Arc::clone(gui_context);
    let params_select = Arc::clone(params);
    ui.on_select_track(move |track: i32| {
        let track = track.max(1) as usize;
        let setter = ParamSetter::new(gui_context_select.as_ref());
        let normalized = params_select.selected_track.preview_normalized(track as i32);
        setter.begin_set_parameter(&params_select.selected_track);
        setter.set_parameter_normalized(&params_select.selected_track, normalized);
        setter.end_set_parameter(&params_select.selected_track);
    });

    let tracks_engine = Arc::clone(tracks);
    let params_engine = Arc::clone(params);
    let ui_engine = ui_weak.clone();
    let pending_engine_load = Arc::clone(&pending_engine);
    ui.on_load_engine(move || {
            let track_idx = params_engine.selected_track.value().saturating_sub(1) as usize;
            if track_idx >= NUM_TRACKS {
                return;
            }
        let engine_index = if let Some(ui) = ui_engine.upgrade() {
            ui.get_engine_index()
        } else {
            0
        };
            let engine_type = match engine_index {
                0 => 1,
                _ => 0,
            };
            if engine_type == 0 {
                return;
            }
            let has_engine = tracks_engine[track_idx].engine_type.load(Ordering::Relaxed) != 0;
            if has_engine {
                if let Some(ui) = ui_engine.upgrade() {
                    ui.set_engine_confirm_text(SharedString::from(
                        "Loading a new engine will clear unsaved data for this track. Continue?",
                    ));
                    ui.set_show_engine_confirm(true);
                }
                *pending_engine_load.lock() = Some(PendingEngineLoad {
                    track_idx,
                    engine_type,
                });
            } else {
            reset_track_for_engine(&tracks_engine[track_idx], engine_type);
                if let Some(ui) = ui_engine.upgrade() {
                    ui.set_engine_loaded(true);
                }
            }
        });

    let gui_context_gain = Arc::clone(gui_context);
    let params_gain = Arc::clone(params);
    ui.on_gain_changed(move |value| {
        let setter = ParamSetter::new(gui_context_gain.as_ref());
        setter.begin_set_parameter(&params_gain.gain);
        setter.set_parameter_normalized(&params_gain.gain, value);
        setter.end_set_parameter(&params_gain.gain);
    });

    let tracks_play = Arc::clone(tracks);
    let global_tempo_play = Arc::clone(global_tempo);
    let metronome_enabled_play = Arc::clone(metronome_enabled);
    let metronome_count_in_ticks_play = Arc::clone(metronome_count_in_ticks);
    let metronome_count_in_playback_for_play =
        Arc::clone(metronome_count_in_playback);
    ui.on_toggle_play(move || {
        let any_playing = tracks_play
            .iter()
            .any(|track| track.is_playing.load(Ordering::Relaxed));
        let any_pending = tracks_play
            .iter()
            .any(|track| track.pending_play.load(Ordering::Relaxed));
        if any_playing || any_pending {
            for track in tracks_play.iter() {
                track.is_playing.store(false, Ordering::Relaxed);
                track.pending_play.store(false, Ordering::Relaxed);
                track.count_in_remaining.store(0, Ordering::Relaxed);
            }
            return;
        }

        let tempo = f32::from_bits(global_tempo_play.load(Ordering::Relaxed)).clamp(20.0, 240.0);
        let count_in_ticks = metronome_count_in_ticks_play.load(Ordering::Relaxed);
        let use_count_in = metronome_enabled_play.load(Ordering::Relaxed)
            && metronome_count_in_playback_for_play.load(Ordering::Relaxed)
            && count_in_ticks > 0;
        for track in tracks_play.iter() {
            let loop_enabled = track.loop_enabled.load(Ordering::Relaxed);
            let loop_mode = track.loop_mode.load(Ordering::Relaxed);
            let loop_start_norm =
                f32::from_bits(track.loop_start.load(Ordering::Relaxed)).clamp(0.0, 0.999);
            let rotate_norm =
                f32::from_bits(track.tape_rotate.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let loop_start = if loop_enabled {
                if let Some(samples) = track.samples.try_lock() {
                    let len = samples.get(0).map(|ch| ch.len()).unwrap_or(0);
                    let base_start = (loop_start_norm * len as f32) as usize;
                    let rotate_offset = (rotate_norm * len as f32) as usize;
                    ((base_start + rotate_offset) % len.max(1)) as f32
                } else {
                    0.0
                }
            } else {
                0.0
            };
            let direction = if loop_mode == 3 { -1 } else { 1 };
            track.loop_dir.store(direction, Ordering::Relaxed);
            if loop_mode == 4 {
                if let Some(samples) = track.samples.try_lock() {
                    let len = samples.get(0).map(|ch| ch.len()).unwrap_or(0);
                    let loop_len =
                        (f32::from_bits(track.loop_length.load(Ordering::Relaxed)) * len as f32)
                            as usize;
                    let loop_len = loop_len.max(1);
                    let loop_end = (loop_start as usize + loop_len).min(len).max(1);
                    let loop_start_usize = loop_start as usize;
                    if loop_end > loop_start_usize {
                        let rand_pos =
                            loop_start_usize + fastrand::usize(..(loop_end - loop_start_usize));
                        track.play_pos.store((rand_pos as f32).to_bits(), Ordering::Relaxed);
                    } else {
                        track.play_pos.store(loop_start.to_bits(), Ordering::Relaxed);
                    }
                } else {
                    track.play_pos.store(loop_start.to_bits(), Ordering::Relaxed);
                }
            } else {
                track.play_pos.store(loop_start.to_bits(), Ordering::Relaxed);
            }
            track
                .loop_start_last
                .store(loop_start as u32, Ordering::Relaxed);
            let mut direction = if loop_mode == 3 { -1 } else { 1 };
            if track.tape_reverse.load(Ordering::Relaxed) {
                direction *= -1;
            }
            let start_pos = f32::from_bits(track.play_pos.load(Ordering::Relaxed));
            track
                .keylock_phase
                .store(0.0f32.to_bits(), Ordering::Relaxed);
            track
                .keylock_grain_a
                .store(start_pos.to_bits(), Ordering::Relaxed);
            track.keylock_grain_b.store(
                (start_pos + direction as f32 * KEYLOCK_GRAIN_HOP as f32).to_bits(),
                Ordering::Relaxed,
            );
            track.debug_logged.store(false, Ordering::Relaxed);
            if use_count_in {
                let sr = track.sample_rate.load(Ordering::Relaxed).max(1);
                let count_in_samples = count_in_samples(tempo, sr, count_in_ticks);
                track
                    .count_in_remaining
                    .store(count_in_samples, Ordering::Relaxed);
                track.pending_play.store(true, Ordering::Relaxed);
                track.is_playing.store(false, Ordering::Relaxed);
            } else {
                track.pending_play.store(false, Ordering::Relaxed);
                track.count_in_remaining.store(0, Ordering::Relaxed);
                track.is_playing.store(true, Ordering::Relaxed);
            }
        }
    });

    let tracks_record = Arc::clone(tracks);
    let params_record = Arc::clone(params);
    let global_tempo_record = Arc::clone(global_tempo);
    let metronome_enabled_record = Arc::clone(metronome_enabled);
    let metronome_count_in_ticks_record = Arc::clone(metronome_count_in_ticks);
    let metronome_count_in_record_enabled = Arc::clone(metronome_count_in_record);
    ui.on_toggle_record(move || {
        let track_idx = params_record.selected_track.value().saturating_sub(1) as usize;
        if track_idx >= NUM_TRACKS {
            return;
        }
        let recording = tracks_record[track_idx]
            .is_recording
            .load(Ordering::Relaxed);
        let pending = tracks_record[track_idx]
            .pending_record
            .load(Ordering::Relaxed);
        if recording || pending {
            tracks_record[track_idx]
                .is_recording
                .store(false, Ordering::Relaxed);
            tracks_record[track_idx]
                .pending_record
                .store(false, Ordering::Relaxed);
            tracks_record[track_idx]
                .count_in_remaining
                .store(0, Ordering::Relaxed);
            if recording {
                if let (Some(samples), Some(mut summary)) = (
                    tracks_record[track_idx].samples.try_lock(),
                    tracks_record[track_idx].waveform_summary.try_lock(),
                ) {
                    if !samples.is_empty() {
                        calculate_waveform_summary(&samples[0], &mut summary);
                        tracks_record[track_idx]
                            .sample_rate
                            .store(RECORD_MAX_SAMPLE_RATE as u32, Ordering::Relaxed);
                    }
                }
            }
            return;
        }

        if let Some(mut samples) = tracks_record[track_idx].samples.try_lock() {
            let overdub = tracks_record[track_idx].tape_overdub.load(Ordering::Relaxed);
            if !overdub {
                for channel in samples.iter_mut() {
                    channel.clear();
                    channel.resize(RECORD_MAX_SAMPLES, 0.0);
                }
                *tracks_record[track_idx].sample_path.lock() = None;
                tracks_record[track_idx]
                    .record_pos
                    .store(0.0f32.to_bits(), Ordering::Relaxed);
            } else {
                let play_pos = tracks_record[track_idx].play_pos.load(Ordering::Relaxed);
                tracks_record[track_idx]
                    .record_pos
                    .store(play_pos, Ordering::Relaxed);
            }
            tracks_record[track_idx]
                .is_playing
                .store(false, Ordering::Relaxed);

            let tempo =
                f32::from_bits(global_tempo_record.load(Ordering::Relaxed)).clamp(20.0, 240.0);
            let count_in_ticks = metronome_count_in_ticks_record.load(Ordering::Relaxed);
            let use_count_in = metronome_enabled_record.load(Ordering::Relaxed)
                && metronome_count_in_record_enabled.load(Ordering::Relaxed)
                && count_in_ticks > 0;
            if use_count_in {
                let sr = tracks_record[track_idx].sample_rate.load(Ordering::Relaxed).max(1);
                let count_in_samples = count_in_samples(tempo, sr, count_in_ticks);
                tracks_record[track_idx]
                    .count_in_remaining
                    .store(count_in_samples, Ordering::Relaxed);
                tracks_record[track_idx]
                    .pending_record
                    .store(true, Ordering::Relaxed);
                tracks_record[track_idx]
                    .is_recording
                    .store(false, Ordering::Relaxed);
            } else {
                tracks_record[track_idx]
                    .pending_record
                    .store(false, Ordering::Relaxed);
                tracks_record[track_idx]
                    .count_in_remaining
                    .store(0, Ordering::Relaxed);
                tracks_record[track_idx]
                    .is_recording
                    .store(true, Ordering::Relaxed);
            }
        }
    });

    let params_load = Arc::clone(params);
    let sample_dialog_tx_load = sample_dialog_tx.clone();
    ui.on_load_sample(move || {
        let track_idx = params_load.selected_track.value().saturating_sub(1) as usize;
        if track_idx >= NUM_TRACKS {
            return;
        }
        let sample_dialog_tx = sample_dialog_tx_load.clone();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .add_filter("Audio", &["wav", "flac", "mp3", "ogg"])
                .pick_file();
            let _ = sample_dialog_tx.send(SampleDialogAction::Load { track_idx, path });
        });
    });

    let params_save = Arc::clone(params);
    let sample_dialog_tx_save = sample_dialog_tx.clone();
    ui.on_save_sample(move || {
        let track_idx = params_save.selected_track.value().saturating_sub(1) as usize;
        if track_idx >= NUM_TRACKS {
            return;
        }
        let sample_dialog_tx = sample_dialog_tx_save.clone();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .add_filter("WAV", &["wav"])
                .save_file();
            if let Some(path) = path {
                let _ = sample_dialog_tx.send(SampleDialogAction::Save { track_idx, path });
            }
        });
    });

    let tracks_level = Arc::clone(tracks);
    let params_level = Arc::clone(params);
    ui.on_track_level_changed(move |value| {
        let track_idx = params_level.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_level[track_idx]
                .level
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mute = Arc::clone(tracks);
    let params_mute = Arc::clone(params);
    ui.on_toggle_track_mute(move || {
        let track_idx = params_mute.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let muted = tracks_mute[track_idx].is_muted.load(Ordering::Relaxed);
            tracks_mute[track_idx]
                .is_muted
                .store(!muted, Ordering::Relaxed);
        }
    });

    let tracks_loop = Arc::clone(tracks);
    let params_loop = Arc::clone(params);
    ui.on_loop_start_changed(move |value| {
        let track_idx = params_loop.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_loop[track_idx]
                .loop_start
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_loop = Arc::clone(tracks);
    let params_loop = Arc::clone(params);
    ui.on_loop_length_changed(move |value| {
        let track_idx = params_loop.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_loop[track_idx]
                .loop_length
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_loop = Arc::clone(tracks);
    let params_loop = Arc::clone(params);
    ui.on_loop_xfade_changed(move |value| {
        let track_idx = params_loop.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_loop[track_idx]
                .loop_xfade
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_loop = Arc::clone(tracks);
    let params_loop = Arc::clone(params);
    ui.on_toggle_loop_enabled(move || {
        let track_idx = params_loop.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let enabled = tracks_loop[track_idx].loop_enabled.load(Ordering::Relaxed);
            tracks_loop[track_idx]
                .loop_enabled
                .store(!enabled, Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_tape_speed_changed(move |value| {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_tape[track_idx]
                .tape_speed
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let global_tempo = Arc::clone(global_tempo);
    ui.on_tape_tempo_changed(move |value| {
        global_tempo.store(value.to_bits(), Ordering::Relaxed);
        for track in tracks_tape.iter() {
            track.tape_tempo.store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let metronome_enabled = Arc::clone(metronome_enabled);
    ui.on_toggle_metronome(move || {
        let enabled = metronome_enabled.load(Ordering::Relaxed);
        metronome_enabled.store(!enabled, Ordering::Relaxed);
    });

    let metronome_count_in_ticks = Arc::clone(metronome_count_in_ticks);
    ui.on_metronome_count_in_changed(move |value| {
        let ticks = value.round().clamp(0.0, METRONOME_COUNT_IN_MAX_TICKS as f32) as u32;
        metronome_count_in_ticks.store(ticks, Ordering::Relaxed);
    });

    let metronome_count_in_playback_toggle =
        Arc::clone(metronome_count_in_playback);
    ui.on_toggle_metronome_count_playback(move || {
        let enabled = metronome_count_in_playback_toggle.load(Ordering::Relaxed);
        metronome_count_in_playback_toggle.store(!enabled, Ordering::Relaxed);
    });

    let metronome_count_in_record_toggle =
        Arc::clone(metronome_count_in_record);
    ui.on_toggle_metronome_count_record(move || {
        let enabled = metronome_count_in_record_toggle.load(Ordering::Relaxed);
        metronome_count_in_record_toggle.store(!enabled, Ordering::Relaxed);
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_tape_rate_mode_selected(move |index| {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let mode = index.clamp(0, 3) as u32;
            tracks_tape[track_idx]
                .tape_rate_mode
                .store(mode, Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_tape_rotate_changed(move |value| {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_tape[track_idx]
                .tape_rotate
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_tape_glide_changed(move |value| {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_tape[track_idx]
                .tape_glide
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_tape_sos_changed(move |value| {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_tape[track_idx]
                .tape_sos
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_pitch_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_pitch
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_rate_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_rate
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_size_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_size
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_contour_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_contour
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_warp_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_warp
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_spray_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_spray
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_pattern_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_pattern
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_wet_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_wet
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_detune_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_detune
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_rand_rate_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_rand_rate
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_rand_size_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_rand_size
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_mosaic_sos_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_sos
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });


    let tracks_mosaic = Arc::clone(tracks);
    let params_mosaic = Arc::clone(params);
    ui.on_toggle_mosaic_enabled(move || {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let enabled = tracks_mosaic[track_idx].mosaic_enabled.load(Ordering::Relaxed);
            tracks_mosaic[track_idx]
                .mosaic_enabled
                .store(!enabled, Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_cutoff_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_cutoff
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_resonance_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_resonance
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_decay_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_decay
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_decay_mode_selected(move |index| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let mode = index.clamp(0, 1) as u32;
            tracks_ring[track_idx]
                .ring_decay_mode
                .store(mode, Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_pitch_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_pitch
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_tone_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_tone
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_tilt_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_tilt
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_slope_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_slope
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_wet_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_wet
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_detune_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_detune
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_toggle_ring_enabled(move || {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let enabled = tracks_ring[track_idx].ring_enabled.load(Ordering::Relaxed);
            tracks_ring[track_idx]
                .ring_enabled
                .store(!enabled, Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_toggle_tape_reverse(move || {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let reversed = tracks_tape[track_idx].tape_reverse.load(Ordering::Relaxed);
            tracks_tape[track_idx]
                .tape_reverse
                .store(!reversed, Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_toggle_tape_freeze(move || {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let frozen = tracks_tape[track_idx].tape_freeze.load(Ordering::Relaxed);
            tracks_tape[track_idx]
                .tape_freeze
                .store(!frozen, Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_toggle_tape_keylock(move || {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let keylock = tracks_tape[track_idx].tape_keylock.load(Ordering::Relaxed);
            let enabled = !keylock;
            tracks_tape[track_idx]
                .tape_keylock
                .store(enabled, Ordering::Relaxed);
            if enabled {
                let mut direction = tracks_tape[track_idx].loop_dir.load(Ordering::Relaxed);
                if direction == 0 {
                    direction = 1;
                }
                if tracks_tape[track_idx].tape_reverse.load(Ordering::Relaxed) {
                    direction *= -1;
                }
                let play_pos = f32::from_bits(
                    tracks_tape[track_idx].play_pos.load(Ordering::Relaxed),
                );
                tracks_tape[track_idx]
                    .keylock_phase
                    .store(0.0f32.to_bits(), Ordering::Relaxed);
                tracks_tape[track_idx]
                    .keylock_grain_a
                    .store(play_pos.to_bits(), Ordering::Relaxed);
                tracks_tape[track_idx].keylock_grain_b.store(
                    (play_pos + direction as f32 * KEYLOCK_GRAIN_HOP as f32).to_bits(),
                    Ordering::Relaxed,
                );
            }
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_toggle_tape_monitor(move || {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let monitor = tracks_tape[track_idx].tape_monitor.load(Ordering::Relaxed);
            tracks_tape[track_idx]
                .tape_monitor
                .store(!monitor, Ordering::Relaxed);
        }
    });

    let tracks_tape = Arc::clone(tracks);
    let params_tape = Arc::clone(params);
    ui.on_toggle_tape_overdub(move || {
        let track_idx = params_tape.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let overdub = tracks_tape[track_idx].tape_overdub.load(Ordering::Relaxed);
            tracks_tape[track_idx]
                .tape_overdub
                .store(!overdub, Ordering::Relaxed);
        }
    });


    let tracks_loop = Arc::clone(tracks);
    let params_loop = Arc::clone(params);
    ui.on_loop_mode_selected(move |index| {
        let track_idx = params_loop.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
        let mode = index.clamp(0, 5) as u32;
            tracks_loop[track_idx]
                .loop_mode
                .store(mode, Ordering::Relaxed);
            tracks_loop[track_idx].loop_dir.store(1, Ordering::Relaxed);
        }
    });

    let ui_visualizer = ui_weak.clone();
    ui.on_visualizer_mode_selected(move |index| {
        if let Some(ui) = ui_visualizer.upgrade() {
            ui.set_visualizer_mode(index);
        }
    });

    let tracks_engine_confirm = Arc::clone(tracks);
    let ui_engine_confirm = ui_weak.clone();
    let pending_engine_confirm = Arc::clone(&pending_engine);
    ui.on_confirm_engine_load(move || {
        if let Some(pending) = pending_engine_confirm.lock().take() {
            if pending.track_idx < NUM_TRACKS {
                reset_track_for_engine(
                    &tracks_engine_confirm[pending.track_idx],
                    pending.engine_type,
                );
            }
        }
        if let Some(ui) = ui_engine_confirm.upgrade() {
            ui.set_show_engine_confirm(false);
        }
    });

    let ui_engine_cancel = ui_weak.clone();
    let pending_engine_cancel = Arc::clone(&pending_engine);
    ui.on_cancel_engine_load(move || {
        *pending_engine_cancel.lock() = None;
        if let Some(ui) = ui_engine_cancel.upgrade() {
            ui.set_show_engine_confirm(false);
        }
    });

    ui.on_quit(|| {
        std::process::exit(0);
    });

    let ui_toggle = ui_weak.clone();
    ui.on_toggle_settings(move || {
        if let Some(ui) = ui_toggle.upgrade() {
            ui.set_show_settings(!ui.get_show_settings());
        }
    });

    ui.on_open_docs(|| {
        open_docs();
    });

    let ui_output_device = ui_weak.clone();
    ui.on_output_device_selected(move |index| {
        if let Some(ui) = ui_output_device.upgrade() {
            ui.set_output_device_index(index);
        }
    });

    let ui_input_device = ui_weak.clone();
    ui.on_input_device_selected(move |index| {
        if let Some(ui) = ui_input_device.upgrade() {
            ui.set_input_device_index(index);
        }
    });

    let ui_sample_rate = ui_weak.clone();
    ui.on_sample_rate_selected(move |index| {
        if let Some(ui) = ui_sample_rate.upgrade() {
            ui.set_sample_rate_index(index);
        }
    });

    let ui_buffer_size = ui_weak.clone();
    ui.on_buffer_size_selected(move |index| {
        if let Some(ui) = ui_buffer_size.upgrade() {
            ui.set_buffer_size_index(index);
        }
    });

    let ui_refresh = ui_weak.clone();
    ui.on_refresh_devices(move || {
        let Some(ui) = ui_refresh.upgrade() else { return; };
        let devices = available_output_devices();
        let inputs = available_input_devices();
        let model = ModelRc::new(VecModel::from(
            devices
                .iter()
                .map(|device| SharedString::from(device.as_str()))
                .collect::<Vec<_>>(),
        ));
        ui.set_output_devices(model);
        if ui.get_output_device_index() >= devices.len() as i32 {
            ui.set_output_device_index(0);
        }
        let input_model = ModelRc::new(VecModel::from(
            inputs
                .iter()
                .map(|device| SharedString::from(device.as_str()))
                .collect::<Vec<_>>(),
        ));
        ui.set_input_devices(input_model);
        if ui.get_input_device_index() >= inputs.len() as i32 {
            ui.set_input_device_index(0);
        }
    });

    let ui_apply = ui_weak.clone();
    let sample_rates = sample_rates.to_vec();
    let buffer_sizes = buffer_sizes.to_vec();
    let gui_context_apply = Arc::clone(gui_context);
    ui.on_apply_settings(move || {
        let Some(ui) = ui_apply.upgrade() else { return; };
        if gui_context_apply.plugin_api() != PluginApi::Standalone {
            ui.set_settings_status("Audio settings are only available in standalone.".into());
            return;
        }
        let output_devices = available_output_devices();
        let input_devices = available_input_devices();
        let output_device = output_devices.get(ui.get_output_device_index() as usize);
        let input_device = input_devices.get(ui.get_input_device_index() as usize);
        let sample_rate = sample_rates.get(ui.get_sample_rate_index() as usize).copied();
        let buffer_size = buffer_sizes.get(ui.get_buffer_size_index() as usize).copied();
        if let Err(err) = restart_with_audio_settings(
            output_device,
            input_device,
            sample_rate,
            buffer_size,
        ) {
            ui.set_settings_status(format!("Failed to restart audio: {err}").into());
        }
    });

    ui.on_save_project({
        let project_dialog_tx = project_dialog_tx.clone();
        move || {
            let project_dialog_tx = project_dialog_tx.clone();
            std::thread::spawn(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("GrainRust Project", &["json"])
                    .save_file();
                if let Some(path) = path {
                    let _ = project_dialog_tx.send(ProjectDialogAction::Save(path));
                }
            });
        }
    });

    ui.on_load_project({
        let project_dialog_tx = project_dialog_tx.clone();
        move || {
            let project_dialog_tx = project_dialog_tx.clone();
            std::thread::spawn(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("GrainRust Project", &["json"])
                    .pick_file();
                if let Some(path) = path {
                    let _ = project_dialog_tx.send(ProjectDialogAction::Load(path));
                }
            });
        }
    });
}

#[derive(Clone)]
enum ProjectDialogAction {
    Save(PathBuf),
    Load(PathBuf),
}

#[derive(Clone)]
enum SampleDialogAction {
    Load { track_idx: usize, path: Option<PathBuf> },
    Save { track_idx: usize, path: PathBuf },
}

struct SlintPlatform {
    start_time: Instant,
}

impl Platform for SlintPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        SLINT_WINDOW_SLOT.with(|slot| {
            *slot.borrow_mut() = Some(window.clone());
        });
        Ok(window)
    }

    fn duration_since_start(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }
}

fn ensure_slint_platform() {
    static SET_PLATFORM: Once = Once::new();
    SET_PLATFORM.call_once(|| {
        let _ = platform::set_platform(Box::new(SlintPlatform {
            start_time: Instant::now(),
        }));
    });
}

fn create_slint_ui() -> (std::rc::Rc<MinimalSoftwareWindow>, Box<GrainRustUI>) {
    SLINT_WINDOW_SLOT.with(|slot| {
        *slot.borrow_mut() = None;
    });
    let ui = Box::new(GrainRustUI::new().expect("Failed to create Slint UI"));
    let window = SLINT_WINDOW_SLOT.with(|slot| {
        slot.borrow_mut()
            .take()
            .expect("Slint window adapter not created")
    });
    (window, ui)
}

thread_local! {
    static SLINT_WINDOW_SLOT: RefCell<Option<std::rc::Rc<MinimalSoftwareWindow>>> =
        RefCell::new(None);
}

/// This version of `baseview` uses a different version of `raw_window_handle than NIH-plug, so we
/// need to adapt it ourselves.
struct ParentWindowHandleAdapter(nih_plug::editor::ParentWindowHandle);

unsafe impl HasRawWindowHandle for ParentWindowHandleAdapter {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.0 {
            ParentWindowHandle::X11Window(window) => {
                let mut handle = raw_window_handle::XcbWindowHandle::empty();
                handle.window = window;
                RawWindowHandle::Xcb(handle)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut handle = raw_window_handle::AppKitWindowHandle::empty();
                handle.ns_view = ns_view;
                RawWindowHandle::AppKit(handle)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut handle = raw_window_handle::Win32WindowHandle::empty();
                handle.hwnd = hwnd;
                RawWindowHandle::Win32(handle)
            }
        }
    }
}

/// Softbuffer uses raw_window_handle v6, but baseview uses raw_window_handle v5, so we need to
/// adapt it ourselves.
#[derive(Clone)]
struct SoftbufferWindowHandleAdapter {
    raw_display_handle: raw_window_handle_06::RawDisplayHandle,
    raw_window_handle: raw_window_handle_06::RawWindowHandle,
}

impl raw_window_handle_06::HasDisplayHandle for SoftbufferWindowHandleAdapter {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle_06::DisplayHandle<'_>, raw_window_handle_06::HandleError> {
        unsafe {
            Ok(raw_window_handle_06::DisplayHandle::borrow_raw(
                self.raw_display_handle,
            ))
        }
    }
}

impl raw_window_handle_06::HasWindowHandle for SoftbufferWindowHandleAdapter {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle_06::WindowHandle<'_>, raw_window_handle_06::HandleError> {
        unsafe {
            Ok(raw_window_handle_06::WindowHandle::borrow_raw(
                self.raw_window_handle,
            ))
        }
    }
}

fn baseview_window_to_surface_target(
    window: &baseview::Window<'_>,
) -> SoftbufferWindowHandleAdapter {
    use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};

    let raw_display_handle = window.raw_display_handle();
    let raw_window_handle = window.raw_window_handle();

    SoftbufferWindowHandleAdapter {
        raw_display_handle: match raw_display_handle {
            raw_window_handle::RawDisplayHandle::AppKit(_) => {
                raw_window_handle_06::RawDisplayHandle::AppKit(
                    raw_window_handle_06::AppKitDisplayHandle::new(),
                )
            }
            raw_window_handle::RawDisplayHandle::Xlib(handle) => {
                raw_window_handle_06::RawDisplayHandle::Xlib(
                    raw_window_handle_06::XlibDisplayHandle::new(
                        std::ptr::NonNull::new(handle.display),
                        handle.screen,
                    ),
                )
            }
            raw_window_handle::RawDisplayHandle::Xcb(handle) => {
                raw_window_handle_06::RawDisplayHandle::Xcb(
                    raw_window_handle_06::XcbDisplayHandle::new(
                        std::ptr::NonNull::new(handle.connection),
                        handle.screen,
                    ),
                )
            }
            raw_window_handle::RawDisplayHandle::Windows(_) => {
                raw_window_handle_06::RawDisplayHandle::Windows(
                    raw_window_handle_06::WindowsDisplayHandle::new(),
                )
            }
            _ => todo!(),
        },
        raw_window_handle: match raw_window_handle {
            raw_window_handle::RawWindowHandle::AppKit(handle) => {
                raw_window_handle_06::RawWindowHandle::AppKit(
                    raw_window_handle_06::AppKitWindowHandle::new(
                        std::ptr::NonNull::new(handle.ns_view).unwrap(),
                    ),
                )
            }
            raw_window_handle::RawWindowHandle::Xlib(handle) => {
                raw_window_handle_06::RawWindowHandle::Xlib(
                    raw_window_handle_06::XlibWindowHandle::new(handle.window),
                )
            }
            raw_window_handle::RawWindowHandle::Xcb(handle) => {
                raw_window_handle_06::RawWindowHandle::Xcb(
                    raw_window_handle_06::XcbWindowHandle::new(
                        std::num::NonZeroU32::new(handle.window).unwrap(),
                    ),
                )
            }
            raw_window_handle::RawWindowHandle::Win32(handle) => {
                let mut raw_handle = raw_window_handle_06::Win32WindowHandle::new(
                    std::num::NonZeroIsize::new(handle.hwnd as isize).unwrap(),
                );

                raw_handle.hinstance = std::num::NonZeroIsize::new(handle.hinstance as isize);

                raw_window_handle_06::RawWindowHandle::Win32(raw_handle)
            }
            _ => todo!(),
        },
    }
}

fn map_mouse_button(button: baseview::MouseButton) -> Option<PointerEventButton> {
    match button {
        baseview::MouseButton::Left => Some(PointerEventButton::Left),
        baseview::MouseButton::Right => Some(PointerEventButton::Right),
        baseview::MouseButton::Middle => Some(PointerEventButton::Middle),
        _ => Some(PointerEventButton::Other),
    }
}

impl Vst3Plugin for GrainRust {
    const VST3_CLASS_ID: [u8; 16] = *b"GrainRustZencode";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Sampler];
}

impl ClapPlugin for GrainRust {
    const CLAP_ID: &'static str = "com.zencoder.grainrust";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A granular sampler plugin");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Sampler, ClapFeature::Stereo];
}

nih_export_vst3!(GrainRust);
nih_export_clap!(GrainRust);

fn current_arg_value(flag: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
    }
    None
}

fn available_output_devices() -> Vec<String> {
    let host = {
        #[cfg(target_os = "windows")]
        {
            cpal::host_from_id(cpal::HostId::Wasapi).unwrap_or_else(|_| cpal::default_host())
        }
        #[cfg(not(target_os = "windows"))]
        {
            cpal::default_host()
        }
    };

    match host.output_devices() {
        Ok(devices) => devices.filter_map(|device| device.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn available_input_devices() -> Vec<String> {
    let host = {
        #[cfg(target_os = "windows")]
        {
            cpal::host_from_id(cpal::HostId::Wasapi).unwrap_or_else(|_| cpal::default_host())
        }
        #[cfg(not(target_os = "windows"))]
        {
            cpal::default_host()
        }
    };

    match host.input_devices() {
        Ok(devices) => devices.filter_map(|device| device.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn restart_with_audio_settings(
    output_device: Option<&String>,
    input_device: Option<&String>,
    sample_rate: Option<u32>,
    buffer_size: Option<u32>,
) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let mut cmd = ProcessCommand::new(exe);

    if let Some(device) = output_device {
        cmd.arg("--output-device").arg(device);
    }
    if let Some(device) = input_device {
        cmd.arg("--input-device").arg(device);
    }
    if let Some(rate) = sample_rate {
        cmd.arg("--sample-rate").arg(rate.to_string());
    }
    if let Some(size) = buffer_size {
        cmd.arg("--period-size").arg(size.to_string());
    }

    cmd.spawn().map_err(|err| err.to_string())?;
    std::process::exit(0);
}
