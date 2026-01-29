/**
 * TLBX-1 - A Rust-based audio toolbox.
 * Copyright (C) 2026 Richard Bakos @ Resonance Designs.
 * Author: Richard Bakos <info@resonancedesigns.dev>
 * Website: https://resonancedesigns.dev
 * Version: 0.1.16
 * Component: Core Logic
 */

use nih_plug::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait};
use parking_lot::Mutex;
use slint::{
    Image, LogicalPosition, ModelRc, PhysicalSize, Rgba8Pixel, SharedPixelBuffer, SharedString,
    VecModel,
};
use slint::platform::{
    self, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use baseview::{
    Event as BaseEvent, EventStatus as BaseEventStatus, Window as BaseWindow, WindowHandle,
    WindowHandler as BaseWindowHandler, WindowOpenOptions, WindowScalePolicy,
};
use keyboard_types::{Key, KeyState};
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use raw_window_handle_06 as raw_window_handle_06;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::fs;

#[derive(Serialize, Deserialize, Default)]
struct TrackData {
    engine_type: u32,
    params: HashMap<String, f32>,
    sequence: Vec<bool>,
    sample_path: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct ProjectData {
    title: String,
    description: String,
    bpm: f32,
    master_gain: f32,
    master_filter: f32,
    master_comp: f32,
    tracks: Vec<String>, // Paths to .trk files relative to project root
}

#[derive(Clone, Copy, Default)]
struct PendingProjectParams {
    gain: f32,
    master_filter: f32,
    master_comp: f32,
}
use std::process::Command as ProcessCommand;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::cell::RefCell;
use std::sync::mpsc;
use std::sync::{Arc, Once};
use std::time::Instant;
use std::f32::consts::PI;
use fundsp::hacker32::{
    AudioUnit, Tanh, bandpass, highpass, lowpass, moog, noise, shape, sine,
};

pub const NUM_TRACKS: usize = 4;
pub const SYNDRM_PAGE_SIZE: usize = 16;
pub const SYNDRM_PAGES: usize = 8;
pub const SYNDRM_STEPS: usize = SYNDRM_PAGE_SIZE * SYNDRM_PAGES;
pub const SYNDRM_LANES: usize = 2;
pub const SYNDRM_FILTER_TYPES: u32 = 4;
pub const WAVEFORM_SUMMARY_SIZE: usize = 100;
pub const RECORD_MAX_SECONDS: usize = 30;
pub const RECORD_MAX_SAMPLE_RATE: usize = 48_000;
pub const RECORD_MAX_SAMPLES: usize = RECORD_MAX_SECONDS * RECORD_MAX_SAMPLE_RATE;
pub const MOSAIC_BUFFER_SECONDS: usize = 4;
pub const MOSAIC_BUFFER_SAMPLES: usize = MOSAIC_BUFFER_SECONDS * RECORD_MAX_SAMPLE_RATE;
pub const MOSAIC_BUFFER_CHANNELS: usize = 2;
pub const MOSAIC_OUTPUT_GAIN: f32 = 1.5;
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
const RING_LFO_RATE_MIN_HZ: f32 = 0.1;
const RING_LFO_RATE_MAX_HZ: f32 = 12.0;
const ANIMATE_LFO_RATE_MIN_HZ: f32 = 0.01;
const ANIMATE_LFO_RATE_MAX_HZ: f32 = 20.0;
const METRONOME_CLICK_MS: f32 = 12.0;
const METRONOME_CLICK_GAIN: f32 = 0.25;
const METRONOME_COUNT_IN_MAX_TICKS: u32 = 8;
const KEYLOCK_GRAIN_SIZE: usize = 256;
const KEYLOCK_GRAIN_HOP: usize = KEYLOCK_GRAIN_SIZE / 2;
const OSCILLOSCOPE_SAMPLES: usize = 256;
const SPECTRUM_BINS: usize = 48;
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

include!(concat!(env!("OUT_DIR"), "/tlbx1.rs"));

#[derive(Clone)]
struct VideoFrame {
    timestamp: f32,
    data: Arc<Vec<u8>>,
}

struct VideoCache {
    frames: Vec<VideoFrame>,
    width: u32,
    height: u32,
    fps: f32,
}

struct Track {
    /// Audio data for the track. Each channel is a Vec of f32.
    samples: Arc<Mutex<Vec<Vec<f32>>>>,
    /// Last loaded sample path, if any.
    sample_path: Arc<Mutex<Option<PathBuf>>>,
    /// Pre-calculated waveform summary for fast drawing.
    waveform_summary: Arc<Mutex<Vec<f32>>>,
    /// Cached video frames for the tape engine, if loaded.
    video_cache: Arc<Mutex<Option<VideoCache>>>,
    /// Whether a video stream is loaded for this track.
    video_enabled: AtomicBool,
    /// Video frame width.
    video_width: AtomicU32,
    /// Video frame height.
    video_height: AtomicU32,
    /// Video frame rate (fps) stored as f32 bits.
    video_fps: AtomicU32,
    /// Video cache version (increments on load).
    video_cache_id: AtomicU32,
    /// Whether the track is currently recording.
    is_recording: AtomicBool,
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
    /// Pending sync request for straight tape playback.
    tape_sync_requested: AtomicBool,
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
    /// Trigger start position as normalized 0..1.
    trigger_start: AtomicU32,
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
    /// Mosaic spatial random pan amount.
    mosaic_spatial: AtomicU32,
    /// Smoothed mosaic spatial random pan amount.
    mosaic_spatial_smooth: AtomicU32,
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
    /// Mosaic pan position for the active grain.
    mosaic_grain_pan: AtomicU32,
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
    /// Ring waves depth (normalized 0..1).
    ring_waves: AtomicU32,
    /// Smoothed ring waves depth.
    ring_waves_smooth: AtomicU32,
    /// Ring waves rate (normalized 0..1).
    ring_waves_rate: AtomicU32,
    /// Smoothed ring waves rate.
    ring_waves_rate_smooth: AtomicU32,
    /// Ring waves rate mode (0 = free, 1 = straight, 2 = dotted, 3 = triplet).
    ring_waves_rate_mode: AtomicU32,
    /// Ring waves LFO phase.
    ring_waves_phase: AtomicU32,
    /// Ring noise depth (normalized 0..1).
    ring_noise: AtomicU32,
    /// Smoothed ring noise depth.
    ring_noise_smooth: AtomicU32,
    /// Ring noise rate (normalized 0..1).
    ring_noise_rate: AtomicU32,
    /// Smoothed ring noise rate.
    ring_noise_rate_smooth: AtomicU32,
    /// Ring noise rate mode (0 = free, 1 = straight, 2 = dotted, 3 = triplet).
    ring_noise_rate_mode: AtomicU32,
    /// Ring noise phase.
    ring_noise_phase: AtomicU32,
    /// Ring noise current value (-1..1).
    ring_noise_value: AtomicU32,
    /// Ring noise RNG state.
    ring_noise_rng: AtomicU32,
    /// Ring scale mode (0 = chromatic, 1 = major, 2 = minor).
    ring_scale: AtomicU32,
    /// Ring detune LFO phase.
    ring_detune_phase: AtomicU32,
    /// Ring filter enabled.
    ring_enabled: AtomicBool,
    /// Ring filter low-pass state per channel.
    ring_low: [AtomicU32; 2],
    /// Ring filter band-pass state per channel.
    ring_band: [AtomicU32; 2],
    /// Animate slot types (0 = wavetable, 1 = sample).
    animate_slot_types: [AtomicU32; 4],
    /// Animate slot wavetable indices.
    animate_slot_wavetables: [AtomicU32; 4],
    /// Animate slot sample indices.
    animate_slot_samples: [AtomicU32; 4],
    /// Animate slot coarse pitch.
    animate_slot_coarse: [AtomicU32; 4],
    /// Animate slot fine pitch.
    animate_slot_fine: [AtomicU32; 4],
    /// Animate slot level.
    animate_slot_level: [AtomicU32; 4],
    /// Smoothed animate slot level.
    animate_slot_level_smooth: [AtomicU32; 4],
    /// Animate slot pan.
    animate_slot_pan: [AtomicU32; 4],
    /// Smoothed animate slot pan.
    animate_slot_pan_smooth: [AtomicU32; 4],
    /// Animate wavetable LFO amount.
    animate_slot_wt_lfo_amount: [AtomicU32; 4],
    /// Animate wavetable LFO shape.
    animate_slot_wt_lfo_shape: [AtomicU32; 4],
    /// Animate wavetable LFO rate.
    animate_slot_wt_lfo_rate: [AtomicU32; 4],
    /// Animate wavetable LFO sync.
    animate_slot_wt_lfo_sync: [AtomicBool; 4],
    /// Animate wavetable LFO division.
    animate_slot_wt_lfo_division: [AtomicU32; 4],
    /// Animate wavetable LFO phase.
    animate_slot_wt_lfo_phase: [AtomicU32; 4],
    /// Animate wavetable LFO sample-and-hold value.
    animate_slot_wt_lfo_snh: [AtomicU32; 4],
    /// Animate sample start (normalized 0..1).
    animate_slot_sample_start: [AtomicU32; 4],
    /// Animate sample loop start (normalized 0..1).
    animate_slot_loop_start: [AtomicU32; 4],
    /// Animate sample loop end (normalized 0..1).
    animate_slot_loop_end: [AtomicU32; 4],
    /// Animate slot filter type (0 = lp24, 1 = lp12, 2 = hp, 3 = bp).
    animate_slot_filter_type: [AtomicU32; 4],
    /// Animate slot filter cutoff (normalized 0..1).
    animate_slot_filter_cutoff: [AtomicU32; 4],
    /// Animate slot filter resonance (normalized 0..1).
    animate_slot_filter_resonance: [AtomicU32; 4],
    /// Animate slot filter state v1 (per voice).
    animate_slot_filter_v1: [[AtomicU32; 4]; 10],
    /// Animate slot filter state v2 (per voice).
    animate_slot_filter_v2: [[AtomicU32; 4]; 10],
    /// Animate slot filter state v1 stage 2 (per voice).
    animate_slot_filter_v1_stage2: [[AtomicU32; 4]; 10],
    /// Animate slot filter state v2 stage 2 (per voice).
    animate_slot_filter_v2_stage2: [[AtomicU32; 4]; 10],
    /// Animate vector position X (0..1).
    animate_vector_x: AtomicU32,
    /// Animate vector position Y (0..1).
    animate_vector_y: AtomicU32,
    /// Smoothed animate vector position X.
    animate_vector_x_smooth: AtomicU32,
    /// Smoothed animate vector position Y.
    animate_vector_y_smooth: AtomicU32,
    /// Animate vector LFO X waveform (0 = sine, 1 = triangle, 2 = square, 3 = saw, 4 = sample&hold).
    animate_lfo_x_waveform: AtomicU32,
    /// Animate vector LFO X sync to BPM.
    animate_lfo_x_sync: AtomicBool,
    /// Animate vector LFO X tempo division index.
    animate_lfo_x_division: AtomicU32,
    /// Animate vector LFO X rate (normalized 0..1, free mode).
    animate_lfo_x_rate: AtomicU32,
    /// Animate vector LFO X amount (normalized 0..1).
    animate_lfo_x_amount: AtomicU32,
    /// Animate vector LFO X phase (0..1).
    animate_lfo_x_phase: AtomicU32,
    /// Animate vector LFO X sample-and-hold value (-1..1).
    animate_lfo_x_snh: AtomicU32,
    /// Animate vector LFO Y waveform (0 = sine, 1 = triangle, 2 = square, 3 = saw, 4 = sample&hold).
    animate_lfo_y_waveform: AtomicU32,
    /// Animate vector LFO Y sync to BPM.
    animate_lfo_y_sync: AtomicBool,
    /// Animate vector LFO Y tempo division index.
    animate_lfo_y_division: AtomicU32,
    /// Animate vector LFO Y rate (normalized 0..1, free mode).
    animate_lfo_y_rate: AtomicU32,
    /// Animate vector LFO Y amount (normalized 0..1).
    animate_lfo_y_amount: AtomicU32,
    /// Animate vector LFO Y phase (0..1).
    animate_lfo_y_phase: AtomicU32,
    /// Animate vector LFO Y sample-and-hold value (-1..1).
    animate_lfo_y_snh: AtomicU32,
    /// Animate LFO RNG state.
    animate_lfo_rng_state: AtomicU32,
    /// Animate sequencer grid (10 rows * 16 steps).
    animate_sequencer_grid: Arc<[AtomicBool; 160]>,
    /// Animate sequencer current step.
    animate_sequencer_step: AtomicI32,
    /// Animate sequencer phase in samples.
    animate_sequencer_phase: AtomicU32,
    /// Animate slot oscillator phases (0..1) for each voice.
    animate_slot_phases: [[AtomicU32; 4]; 10],
    /// Animate slot sample playback positions (in samples) for each voice.
    animate_slot_sample_pos: [[AtomicU32; 4]; 10],
    /// Animate amp envelope stage (0 = idle, 1 = attack, 2 = decay, 3 = sustain, 4 = release) for each voice.
    animate_amp_stage: [AtomicU32; 10],
    /// Animate amp envelope level (0..1) for each voice.
    animate_amp_level: [AtomicU32; 10],
    /// Animate keybed trigger note (MIDI note).
    animate_keybed_note: AtomicI32,
    /// Animate keybed trigger flag.
    animate_keybed_trigger: AtomicBool,
    /// Animate keybed hold (sustain).
    animate_keybed_hold: AtomicBool,
    /// Animate keybed amp envelope stage.
    animate_keybed_amp_stage: AtomicU32,
    /// Animate keybed amp envelope level.
    animate_keybed_amp_level: AtomicU32,
    /// Animate keybed slot oscillator phases.
    animate_keybed_slot_phases: [AtomicU32; 4],
    /// Animate keybed slot sample playback positions.
    animate_keybed_slot_sample_pos: [AtomicU32; 4],
    /// Animate keybed filter state v1 per slot.
    animate_keybed_filter_v1: [AtomicU32; 4],
    /// Animate keybed filter state v2 per slot.
    animate_keybed_filter_v2: [AtomicU32; 4],
    /// Animate keybed filter state v1 stage 2 per slot.
    animate_keybed_filter_v1_stage2: [AtomicU32; 4],
    /// Animate keybed filter state v2 stage 2 per slot.
    animate_keybed_filter_v2_stage2: [AtomicU32; 4],
    /// SynDRM kick pitch (normalized 0..1).
    kick_pitch: AtomicU32,
    /// SynDRM kick decay (normalized 0..1).
    kick_decay: AtomicU32,
    /// SynDRM kick attack (normalized 0..1).
    kick_attack: AtomicU32,
    /// SynDRM kick pitch envelope amount (normalized 0..1).
    kick_pitch_env_amount: AtomicU32,
    /// SynDRM kick drive (normalized 0..1).
    kick_drive: AtomicU32,
    /// SynDRM kick output level (normalized 0..1).
    kick_level: AtomicU32,
    /// SynDRM kick filter type (0 = Moog LP, 1 = LP, 2 = HP, 3 = BP).
    kick_filter_type: AtomicU32,
    /// SynDRM kick filter cutoff (normalized 0..1).
    kick_filter_cutoff: AtomicU32,
    /// SynDRM kick filter resonance (normalized 0..1).
    kick_filter_resonance: AtomicU32,
    /// SynDRM kick filter pre-drive toggle.
    kick_filter_pre_drive: AtomicBool,
    /// SynDRM kick sequencer grid (128 steps).
    kick_sequencer_grid: Arc<[AtomicBool; SYNDRM_STEPS]>,
    /// SynDRM kick sequencer current step.
    kick_sequencer_step: AtomicI32,
    /// SynDRM kick sequencer phase in samples.
    kick_sequencer_phase: AtomicU32,
    /// SynDRM kick oscillator phase (0..1).
    kick_phase: AtomicU32,
    /// SynDRM kick amplitude envelope (0..1).
    kick_env: AtomicU32,
    /// SynDRM kick pitch envelope (0..1).
    kick_pitch_env: AtomicU32,
    /// SynDRM kick attack remaining samples.
    /// SynDRM snare tone (normalized 0..1).
    snare_tone: AtomicU32,
    /// SynDRM snare decay (normalized 0..1).
    snare_decay: AtomicU32,
    /// SynDRM snare snappy/noise mix (normalized 0..1).
    snare_snappy: AtomicU32,
    /// SynDRM snare attack (normalized 0..1).
    snare_attack: AtomicU32,
    /// SynDRM snare drive (normalized 0..1).
    snare_drive: AtomicU32,
    /// SynDRM snare output level (normalized 0..1).
    snare_level: AtomicU32,
    /// SynDRM snare filter type (0 = Moog LP, 1 = LP, 2 = HP, 3 = BP).
    snare_filter_type: AtomicU32,
    /// SynDRM snare filter cutoff (normalized 0..1).
    snare_filter_cutoff: AtomicU32,
    /// SynDRM snare filter resonance (normalized 0..1).
    snare_filter_resonance: AtomicU32,
    /// SynDRM snare filter pre-drive toggle.
    snare_filter_pre_drive: AtomicBool,
    /// SynDRM snare sequencer grid (128 steps).
    snare_sequencer_grid: Arc<[AtomicBool; SYNDRM_STEPS]>,
    /// SynDRM snare sequencer current step.
    snare_sequencer_step: AtomicI32,
    /// SynDRM snare sequencer phase in samples.
    snare_sequencer_phase: AtomicU32,
    /// SynDRM snare oscillator phase (0..1).
    snare_phase: AtomicU32,
    /// SynDRM snare tone envelope (0..1).
    snare_env: AtomicU32,
    /// SynDRM snare noise envelope (0..1).
    snare_noise_env: AtomicU32,
    /// SynDRM snare attack remaining samples.
    snare_attack_remaining: AtomicU32,
    /// SynDRM snare noise RNG state.
    snare_noise_rng: AtomicU32,
    kick_attack_remaining: AtomicU32,
    /// SynDRM sequencer page (0..7).
    syndrm_page: AtomicU32,
    /// SynDRM step editor lane (0 = kick, 1 = snare).
    syndrm_edit_lane: AtomicU32,
    /// SynDRM step editor step index (0..127).
    syndrm_edit_step: AtomicU32,
    /// SynDRM step hold mode (true = hold override).
    syndrm_step_hold: AtomicBool,
    /// SynDRM RNG state for sequencer randomization.
    syndrm_rng_state: AtomicU32,
    /// SynDRM kick step override enabled.
    kick_step_override_enabled: Arc<[AtomicBool; SYNDRM_STEPS]>,
    /// SynDRM kick step pitch override.
    kick_step_pitch: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM kick step decay override.
    kick_step_decay: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM kick step attack override.
    kick_step_attack: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM kick step drive override.
    kick_step_drive: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM kick step level override.
    kick_step_level: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM kick step filter type override.
    kick_step_filter_type: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM kick step filter cutoff override.
    kick_step_filter_cutoff: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM kick step filter resonance override.
    kick_step_filter_resonance: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step override enabled.
    snare_step_override_enabled: Arc<[AtomicBool; SYNDRM_STEPS]>,
    /// SynDRM snare step tone override.
    snare_step_tone: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step decay override.
    snare_step_decay: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step snappy override.
    snare_step_snappy: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step attack override.
    snare_step_attack: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step drive override.
    snare_step_drive: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step level override.
    snare_step_level: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step filter type override.
    snare_step_filter_type: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step filter cutoff override.
    snare_step_filter_cutoff: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// SynDRM snare step filter resonance override.
    snare_step_filter_resonance: Arc<[AtomicU32; SYNDRM_STEPS]>,
    /// Void Seed base frequency.
    void_base_freq: AtomicU32,
    /// Smoothed void base frequency.
    void_base_freq_smooth: AtomicU32,
    /// Void Seed chaos depth (X).
    void_chaos_depth: AtomicU32,
    /// Smoothed void chaos depth.
    void_chaos_depth_smooth: AtomicU32,
    /// Void Seed entropy (Y).
    void_entropy: AtomicU32,
    /// Smoothed void entropy.
    void_entropy_smooth: AtomicU32,
    /// Whether the void seed engine is enabled and active.
    void_enabled: AtomicBool,
    /// Void Seed feedback.
    void_feedback: AtomicU32,
    /// Smoothed void feedback.
    void_feedback_smooth: AtomicU32,
    /// Void Seed diffusion (wet).
    void_diffusion: AtomicU32,
    /// Smoothed void diffusion.
    void_diffusion_smooth: AtomicU32,
    /// Void Seed modulation rate.
    void_mod_rate: AtomicU32,
    /// Smoothed void modulation rate.
    void_mod_rate_smooth: AtomicU32,
    /// Void Seed level.
    void_level: AtomicU32,
    /// Smoothed void level.
    void_level_smooth: AtomicU32,
    /// Void Seed oscillator phases.
    void_osc_phases: [AtomicU32; 12],
    /// Void Seed detune LFO phases.
    void_lfo_phases: [AtomicU32; 12],
    /// Void Seed detune LFO frequencies.
    void_lfo_freqs: [AtomicU32; 12],
    /// Void Seed chaos LFO phase.
    void_lfo_chaos_phase: AtomicU32,
    /// Void Seed filter state v1.
    void_filter_v1: [AtomicU32; 2],
    /// Void Seed filter state v2.
    void_filter_v2: [AtomicU32; 2],
    /// Void Seed internal gain (for ramping).
    void_internal_gain: AtomicU32,
    /// Void Seed delay buffer.
    void_delay_buffer: Arc<Mutex<[Vec<f32>; 2]>>,
    /// Void Seed delay write position.
    void_delay_write_pos: AtomicU32,
    /// Engine type loaded for this track (0 = none, 1 = tape, 2 = animate, 3 = syndrm, 4 = voidseed).
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
            video_cache: Arc::new(Mutex::new(None)),
            video_enabled: AtomicBool::new(false),
            video_width: AtomicU32::new(0),
            video_height: AtomicU32::new(0),
            video_fps: AtomicU32::new(0.0f32.to_bits()),
            video_cache_id: AtomicU32::new(0),
            is_recording: AtomicBool::new(false),
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
            tape_sync_requested: AtomicBool::new(false),
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
            trigger_start: AtomicU32::new(0.0f32.to_bits()),
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
            mosaic_spatial: AtomicU32::new(0.0f32.to_bits()),
            mosaic_spatial_smooth: AtomicU32::new(0.0f32.to_bits()),
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
            mosaic_grain_pan: AtomicU32::new(0.0f32.to_bits()),
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
            ring_waves: AtomicU32::new(0.0f32.to_bits()),
            ring_waves_smooth: AtomicU32::new(0.0f32.to_bits()),
            ring_waves_rate: AtomicU32::new(0.5f32.to_bits()),
            ring_waves_rate_smooth: AtomicU32::new(0.5f32.to_bits()),
            ring_waves_rate_mode: AtomicU32::new(0),
            ring_waves_phase: AtomicU32::new(0.0f32.to_bits()),
            ring_noise: AtomicU32::new(0.0f32.to_bits()),
            ring_noise_smooth: AtomicU32::new(0.0f32.to_bits()),
            ring_noise_rate: AtomicU32::new(0.5f32.to_bits()),
            ring_noise_rate_smooth: AtomicU32::new(0.5f32.to_bits()),
            ring_noise_rate_mode: AtomicU32::new(0),
            ring_noise_phase: AtomicU32::new(0.0f32.to_bits()),
            ring_noise_value: AtomicU32::new(0.0f32.to_bits()),
            ring_noise_rng: AtomicU32::new(0x1357_2468),
            ring_scale: AtomicU32::new(0),
            ring_detune_phase: AtomicU32::new(0.0f32.to_bits()),
            ring_enabled: AtomicBool::new(false),
            ring_low: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            ring_band: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_types: std::array::from_fn(|_| AtomicU32::new(0)),
            animate_slot_wavetables: std::array::from_fn(|_| AtomicU32::new(0)),
            animate_slot_samples: std::array::from_fn(|_| AtomicU32::new(0)),
            animate_slot_coarse: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_fine: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_level: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            animate_slot_level_smooth: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            animate_slot_pan: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_pan_smooth: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_wt_lfo_amount: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_wt_lfo_shape: std::array::from_fn(|_| AtomicU32::new(0)),
            animate_slot_wt_lfo_rate: std::array::from_fn(|_| AtomicU32::new(0.5f32.to_bits())),
            animate_slot_wt_lfo_sync: std::array::from_fn(|_| AtomicBool::new(false)),
            animate_slot_wt_lfo_division: std::array::from_fn(|_| AtomicU32::new(0)),
            animate_slot_wt_lfo_phase: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_wt_lfo_snh: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_sample_start: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_loop_start: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_loop_end: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            animate_slot_filter_type: std::array::from_fn(|_| AtomicU32::new(0)),
            animate_slot_filter_cutoff: std::array::from_fn(|_| AtomicU32::new(0.5f32.to_bits())),
            animate_slot_filter_resonance: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_slot_filter_v1: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))
            }),
            animate_slot_filter_v2: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))
            }),
            animate_slot_filter_v1_stage2: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))
            }),
            animate_slot_filter_v2_stage2: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))
            }),
            animate_vector_x: AtomicU32::new(0.5f32.to_bits()),
            animate_vector_y: AtomicU32::new(0.5f32.to_bits()),
            animate_vector_x_smooth: AtomicU32::new(0.5f32.to_bits()),
            animate_vector_y_smooth: AtomicU32::new(0.5f32.to_bits()),
            animate_lfo_x_waveform: AtomicU32::new(0),
            animate_lfo_x_sync: AtomicBool::new(false),
            animate_lfo_x_division: AtomicU32::new(0),
            animate_lfo_x_rate: AtomicU32::new(0.5f32.to_bits()),
            animate_lfo_x_amount: AtomicU32::new(0.0f32.to_bits()),
            animate_lfo_x_phase: AtomicU32::new(0.0f32.to_bits()),
            animate_lfo_x_snh: AtomicU32::new(0.0f32.to_bits()),
            animate_lfo_y_waveform: AtomicU32::new(0),
            animate_lfo_y_sync: AtomicBool::new(false),
            animate_lfo_y_division: AtomicU32::new(0),
            animate_lfo_y_rate: AtomicU32::new(0.5f32.to_bits()),
            animate_lfo_y_amount: AtomicU32::new(0.0f32.to_bits()),
            animate_lfo_y_phase: AtomicU32::new(0.0f32.to_bits()),
            animate_lfo_y_snh: AtomicU32::new(0.0f32.to_bits()),
            animate_lfo_rng_state: AtomicU32::new(0x2468_ace1),
            animate_sequencer_grid: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
            animate_sequencer_step: AtomicI32::new(-1),
            animate_sequencer_phase: AtomicU32::new(0),
            animate_slot_phases: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))),
            animate_slot_sample_pos: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))),
            animate_amp_stage: std::array::from_fn(|_| AtomicU32::new(0)),
            animate_amp_level: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_keybed_note: AtomicI32::new(60),
            animate_keybed_trigger: AtomicBool::new(false),
            animate_keybed_hold: AtomicBool::new(false),
            animate_keybed_amp_stage: AtomicU32::new(0),
            animate_keybed_amp_level: AtomicU32::new(0.0f32.to_bits()),
            animate_keybed_slot_phases: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_keybed_slot_sample_pos: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_keybed_filter_v1: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_keybed_filter_v2: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_keybed_filter_v1_stage2: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            animate_keybed_filter_v2_stage2: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            kick_pitch: AtomicU32::new(0.5f32.to_bits()),
            kick_decay: AtomicU32::new(0.5f32.to_bits()),
            kick_attack: AtomicU32::new(0.0f32.to_bits()),
            kick_pitch_env_amount: AtomicU32::new(0.0f32.to_bits()),
            kick_drive: AtomicU32::new(0.0f32.to_bits()),
            kick_level: AtomicU32::new(1.0f32.to_bits()),
            kick_filter_type: AtomicU32::new(0),
            kick_filter_cutoff: AtomicU32::new(0.6f32.to_bits()),
            kick_filter_resonance: AtomicU32::new(0.2f32.to_bits()),
            kick_filter_pre_drive: AtomicBool::new(true),
            kick_sequencer_grid: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
            kick_sequencer_step: AtomicI32::new(-1),
            kick_sequencer_phase: AtomicU32::new(0),
            kick_phase: AtomicU32::new(0.0f32.to_bits()),
            kick_env: AtomicU32::new(0.0f32.to_bits()),
            kick_pitch_env: AtomicU32::new(0.0f32.to_bits()),
            kick_attack_remaining: AtomicU32::new(0),
            snare_tone: AtomicU32::new(0.5f32.to_bits()),
            snare_decay: AtomicU32::new(0.4f32.to_bits()),
            snare_snappy: AtomicU32::new(0.6f32.to_bits()),
            snare_attack: AtomicU32::new(0.0f32.to_bits()),
            snare_drive: AtomicU32::new(0.0f32.to_bits()),
            snare_level: AtomicU32::new(0.8f32.to_bits()),
            snare_filter_type: AtomicU32::new(0),
            snare_filter_cutoff: AtomicU32::new(0.6f32.to_bits()),
            snare_filter_resonance: AtomicU32::new(0.2f32.to_bits()),
            snare_filter_pre_drive: AtomicBool::new(true),
            snare_sequencer_grid: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
            snare_sequencer_step: AtomicI32::new(-1),
            snare_sequencer_phase: AtomicU32::new(0),
            snare_phase: AtomicU32::new(0.0f32.to_bits()),
            snare_env: AtomicU32::new(0.0f32.to_bits()),
            snare_noise_env: AtomicU32::new(0.0f32.to_bits()),
            snare_attack_remaining: AtomicU32::new(0),
            snare_noise_rng: AtomicU32::new(0xdead_beef),
            syndrm_page: AtomicU32::new(0),
            syndrm_edit_lane: AtomicU32::new(0),
            syndrm_edit_step: AtomicU32::new(0),
            syndrm_step_hold: AtomicBool::new(false),
            syndrm_rng_state: AtomicU32::new(0x81c3_5f27),
            kick_step_override_enabled: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
            kick_step_pitch: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.5f32.to_bits()))),
            kick_step_decay: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.5f32.to_bits()))),
            kick_step_attack: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))),
            kick_step_drive: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))),
            kick_step_level: Arc::new(std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits()))),
            kick_step_filter_type: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            kick_step_filter_cutoff: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.6f32.to_bits()))),
            kick_step_filter_resonance: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.2f32.to_bits()))),
            snare_step_override_enabled: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
            snare_step_tone: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.5f32.to_bits()))),
            snare_step_decay: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.4f32.to_bits()))),
            snare_step_snappy: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.6f32.to_bits()))),
            snare_step_attack: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))),
            snare_step_drive: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits()))),
            snare_step_level: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.8f32.to_bits()))),
            snare_step_filter_type: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            snare_step_filter_cutoff: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.6f32.to_bits()))),
            snare_step_filter_resonance: Arc::new(std::array::from_fn(|_| AtomicU32::new(0.2f32.to_bits()))),
            void_base_freq: AtomicU32::new(40.0f32.to_bits()),
            void_base_freq_smooth: AtomicU32::new(40.0f32.to_bits()),
            void_enabled: AtomicBool::new(false),
            void_chaos_depth: AtomicU32::new(0.5f32.to_bits()),
            void_chaos_depth_smooth: AtomicU32::new(0.5f32.to_bits()),
            void_entropy: AtomicU32::new(0.2f32.to_bits()),
            void_entropy_smooth: AtomicU32::new(0.2f32.to_bits()),
            void_feedback: AtomicU32::new(0.8f32.to_bits()),
            void_feedback_smooth: AtomicU32::new(0.8f32.to_bits()),
            void_diffusion: AtomicU32::new(0.5f32.to_bits()),
            void_diffusion_smooth: AtomicU32::new(0.5f32.to_bits()),
            void_mod_rate: AtomicU32::new(0.1f32.to_bits()),
            void_mod_rate_smooth: AtomicU32::new(0.1f32.to_bits()),
            void_level: AtomicU32::new(0.8f32.to_bits()),
            void_level_smooth: AtomicU32::new(0.8f32.to_bits()),
            void_osc_phases: Default::default(),
            void_lfo_phases: Default::default(),
            void_lfo_freqs: [
                AtomicU32::new(0.05f32.to_bits()),
                AtomicU32::new(0.12f32.to_bits()),
                AtomicU32::new(0.07f32.to_bits()),
                AtomicU32::new(0.15f32.to_bits()),
                AtomicU32::new(0.03f32.to_bits()),
                AtomicU32::new(0.18f32.to_bits()),
                AtomicU32::new(0.09f32.to_bits()),
                AtomicU32::new(0.11f32.to_bits()),
                AtomicU32::new(0.04f32.to_bits()),
                AtomicU32::new(0.14f32.to_bits()),
                AtomicU32::new(0.06f32.to_bits()),
                AtomicU32::new(0.17f32.to_bits()),
            ],
            void_lfo_chaos_phase: AtomicU32::new(0),
            void_filter_v1: Default::default(),
            void_filter_v2: Default::default(),
            void_internal_gain: AtomicU32::new(0.0f32.to_bits()),
            void_delay_buffer: Arc::new(Mutex::new([vec![0.0; 65536], vec![0.0; 65536]])),
            void_delay_write_pos: AtomicU32::new(0),
            engine_type: AtomicU32::new(0),
            debug_logged: AtomicBool::new(false),
            sample_rate: AtomicU32::new(44_100),
        }
    }
}

pub struct TLBX1 {
    params: Arc<TLBX1Params>,
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
    master_step_phase: f32,
    master_step_index: i32,
    master_step_count: i64,
    animate_library: Arc<AnimateLibrary>,
    master_fx: MasterFxState,
    sample_rate: AtomicU32,
    pending_project_params: Arc<Mutex<Option<PendingProjectParams>>>,
    track_buffer: Vec<Vec<f32>>,
    syndrm_dsp: [SynDRMDspState; NUM_TRACKS],
}

struct SynDRMDspState {
    sample_rate: f32,
    kick_osc: Box<dyn AudioUnit>,
    kick_drive: Box<dyn AudioUnit>,
    snare_osc: Box<dyn AudioUnit>,
    snare_noise: Box<dyn AudioUnit>,
    snare_drive: Box<dyn AudioUnit>,
    kick_filter_moog: Box<dyn AudioUnit>,
    kick_filter_lp: Box<dyn AudioUnit>,
    kick_filter_hp: Box<dyn AudioUnit>,
    kick_filter_bp: Box<dyn AudioUnit>,
    snare_filter_moog: Box<dyn AudioUnit>,
    snare_filter_lp: Box<dyn AudioUnit>,
    snare_filter_hp: Box<dyn AudioUnit>,
    snare_filter_bp: Box<dyn AudioUnit>,
}

impl SynDRMDspState {
    fn new() -> Self {
        Self {
            sample_rate: 0.0,
            kick_osc: Box::new(sine()),
            kick_drive: Box::new(shape(Tanh(1.0))),
            snare_osc: Box::new(sine()),
            snare_noise: Box::new(noise()),
            snare_drive: Box::new(shape(Tanh(1.0))),
            kick_filter_moog: Box::new(moog()),
            kick_filter_lp: Box::new(lowpass()),
            kick_filter_hp: Box::new(highpass()),
            kick_filter_bp: Box::new(bandpass()),
            snare_filter_moog: Box::new(moog()),
            snare_filter_lp: Box::new(lowpass()),
            snare_filter_hp: Box::new(highpass()),
            snare_filter_bp: Box::new(bandpass()),
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() < f32::EPSILON {
            return;
        }
        self.sample_rate = sample_rate;
        let sr = sample_rate as f64;
        self.kick_osc.set_sample_rate(sr);
        self.kick_drive.set_sample_rate(sr);
        self.snare_osc.set_sample_rate(sr);
        self.snare_noise.set_sample_rate(sr);
        self.snare_drive.set_sample_rate(sr);
        self.kick_filter_moog.set_sample_rate(sr);
        self.kick_filter_lp.set_sample_rate(sr);
        self.kick_filter_hp.set_sample_rate(sr);
        self.kick_filter_bp.set_sample_rate(sr);
        self.snare_filter_moog.set_sample_rate(sr);
        self.snare_filter_lp.set_sample_rate(sr);
        self.snare_filter_hp.set_sample_rate(sr);
        self.snare_filter_bp.set_sample_rate(sr);
    }
}

struct AnimateLibrary {
    wavetable_paths: Vec<PathBuf>,
    sample_paths: Vec<PathBuf>,
    wavetables: Mutex<Vec<Option<Arc<Vec<f32>>>>>,
    samples: Mutex<Vec<Option<Arc<Vec<Vec<f32>>>>>>,
}

#[derive(Params)]
pub struct TLBX1Params {
    #[id = "selected_track"]
    pub selected_track: IntParam,

    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "master_filter"]
    pub master_filter: FloatParam,

    #[id = "master_comp"]
    pub master_comp: FloatParam,
}

impl AnimateLibrary {
    fn load() -> Self {
        fn scan_dir(dir: &Path, paths: &mut Vec<PathBuf>) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                let mut sorted_entries: Vec<_> = entries.flatten().collect();
                sorted_entries.sort_by_key(|e| e.file_name());

                for entry in sorted_entries {
                    let path = entry.path();
                    if path.is_dir() {
                        scan_dir(&path, paths);
                    } else if path.extension().map_or(false, |ext| ext == "wav" || ext == "mp3") {
                        paths.push(path);
                    }
                }
            }
        }

        let mut wavetable_paths = Vec::new();
        let mut sample_paths = Vec::new();

        scan_dir(Path::new("src/library/factory/wavetables"), &mut wavetable_paths);
        scan_dir(Path::new("src/library/factory/samples"), &mut sample_paths);

        let wavetables = vec![None; wavetable_paths.len()];
        let samples = vec![None; sample_paths.len()];

        Self {
            wavetable_paths,
            sample_paths,
            wavetables: Mutex::new(wavetables),
            samples: Mutex::new(samples),
        }
    }

    fn ensure_wavetable_loaded(&self, idx: usize) -> Option<Arc<Vec<f32>>> {
        if idx >= self.wavetable_paths.len() {
            return None;
        }
        if let Some(cache) = self.wavetables.try_lock() {
            if let Some(existing) = cache.get(idx).and_then(|entry| entry.clone()) {
                return Some(existing);
            }
        }
        let path = self.wavetable_paths.get(idx)?.clone();
        let data = load_audio_file(&path).ok();
        let wavetable = data.and_then(|(data, _)| data.get(0).cloned());
        if let Some(wt) = wavetable {
            let arc = Arc::new(wt);
            if let Some(mut cache) = self.wavetables.try_lock() {
                if let Some(entry) = cache.get_mut(idx) {
                    *entry = Some(Arc::clone(&arc));
                }
            }
            return Some(arc);
        }
        None
    }

    fn ensure_sample_loaded(&self, idx: usize) -> Option<Arc<Vec<Vec<f32>>>> {
        if idx >= self.sample_paths.len() {
            return None;
        }
        if let Some(cache) = self.samples.try_lock() {
            if let Some(existing) = cache.get(idx).and_then(|entry| entry.clone()) {
                return Some(existing);
            }
        }
        let path = self.sample_paths.get(idx)?.clone();
        let data = load_audio_file(&path).ok();
        if let Some((data, _)) = data {
            let arc = Arc::new(data);
            if let Some(mut cache) = self.samples.try_lock() {
                if let Some(entry) = cache.get_mut(idx) {
                    *entry = Some(Arc::clone(&arc));
                }
            }
            return Some(arc);
        }
        None
    }

    fn get_wavetable_cached(&self, idx: usize) -> Option<Arc<Vec<f32>>> {
        if idx >= self.wavetable_paths.len() {
            return None;
        }
        self.wavetables
            .try_lock()
            .and_then(|cache| cache.get(idx).and_then(|entry| entry.clone()))
    }

    fn get_sample_cached(&self, idx: usize) -> Option<Arc<Vec<Vec<f32>>>> {
        if idx >= self.sample_paths.len() {
            return None;
        }
        self.samples
            .try_lock()
            .and_then(|cache| cache.get(idx).and_then(|entry| entry.clone()))
    }
}

impl Default for TLBX1 {
    fn default() -> Self {
        let tracks = [
            Track::default(),
            Track::default(),
            Track::default(),
            Track::default(),
        ];
        
        Self {
            params: Arc::new(TLBX1Params::default()),
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
            master_step_phase: 0.0,
            master_step_index: 0,
            master_step_count: 0,
            animate_library: Arc::new(AnimateLibrary::load()),
            master_fx: MasterFxState::default(),
            sample_rate: AtomicU32::new(44100),
            pending_project_params: Arc::new(Mutex::new(None)),
            track_buffer: vec![vec![0.0; 1024]; 2],
            syndrm_dsp: std::array::from_fn(|_| SynDRMDspState::new()),
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

struct MasterFxState {
    // SVF filter state: [channel_idx]
    filter_low: [f32; 2],
    filter_band: [f32; 2],
    // Compressor envelope follower
    comp_env: f32,
}

impl Default for MasterFxState {
    fn default() -> Self {
        Self {
            filter_low: [0.0; 2],
            filter_band: [0.0; 2],
            comp_env: 0.0,
        }
    }
}

#[derive(Default)]
struct VisualizerState {
    oscilloscope: Mutex<Vec<f32>>,
    spectrum: Mutex<Vec<f32>>,
    vectorscope_x: Mutex<Vec<f32>>,
    vectorscope_y: Mutex<Vec<f32>>,
}

impl Default for TLBX1Params {
    fn default() -> Self {
        Self {
            selected_track: IntParam::new("Selected Track", 1, IntRange::Linear { min: 1, max: 4 }),
            gain: FloatParam::new(
                "Master Gain",
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

            master_filter: FloatParam::new(
                "Master DJ Filter",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(Arc::new(|v| {
                if v < 0.49 {
                    format!("HP {:.0} Hz", 20.0 + (1.0 - v / 0.5) * 2000.0)
                } else if v > 0.51 {
                    format!("LP {:.0} Hz", 20000.0 - ((v - 0.5) / 0.5) * 19000.0)
                } else {
                    "Neutral".to_string()
                }
            })),

            master_comp: FloatParam::new(
                "Master Compression",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

pub enum TLBX1Task {
    LoadSample(usize, PathBuf),
    SaveProject {
        path: PathBuf,
        title: String,
        description: String,
    },
    LoadProject(PathBuf),
    ExportProjectZip {
        path: PathBuf,
        title: String,
        description: String,
    },
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
    track.video_enabled.store(false, Ordering::Relaxed);
    track.video_width.store(0, Ordering::Relaxed);
    track.video_height.store(0, Ordering::Relaxed);
    track.video_fps.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.video_cache_id.fetch_add(1, Ordering::Relaxed);
    *track.video_cache.lock() = None;
    track.tape_speed.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.tape_speed_smooth.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.tape_tempo.store(120.0f32.to_bits(), Ordering::Relaxed);
    track.tape_rate_mode.store(0, Ordering::Relaxed);
    track.tape_sync_requested.store(false, Ordering::Relaxed);
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
    track.trigger_start.store(0.0f32.to_bits(), Ordering::Relaxed);
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
    track.mosaic_spatial.store(0.0f32.to_bits(), Ordering::Relaxed);
    track
        .mosaic_spatial_smooth
        .store(0.0f32.to_bits(), Ordering::Relaxed);
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
    track.mosaic_grain_pan.store(0.0f32.to_bits(), Ordering::Relaxed);
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
    track.ring_waves.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_waves_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_waves_rate.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_waves_rate_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_waves_rate_mode.store(0, Ordering::Relaxed);
    track.ring_waves_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_noise.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_noise_smooth.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_noise_rate.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_noise_rate_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.ring_noise_rate_mode.store(0, Ordering::Relaxed);
    track.ring_noise_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_noise_value.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.ring_noise_rng.store(0x1357_2468, Ordering::Relaxed);
    track.ring_scale.store(0, Ordering::Relaxed);
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
    for i in 0..4 {
        track.animate_slot_types[i].store(0, Ordering::Relaxed);
        track.animate_slot_wavetables[i].store(0, Ordering::Relaxed);
        track.animate_slot_samples[i].store(0, Ordering::Relaxed);
        track.animate_slot_coarse[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.animate_slot_fine[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.animate_slot_level[i].store(1.0f32.to_bits(), Ordering::Relaxed);
        track.animate_slot_level_smooth[i].store(1.0f32.to_bits(), Ordering::Relaxed);
        track.animate_slot_pan[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.animate_slot_pan_smooth[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_wt_lfo_amount[i]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_wt_lfo_shape[i]
            .store(0, Ordering::Relaxed);
        track
            .animate_slot_wt_lfo_rate[i]
            .store(0.5f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_wt_lfo_sync[i]
            .store(false, Ordering::Relaxed);
        track
            .animate_slot_wt_lfo_division[i]
            .store(0, Ordering::Relaxed);
        track
            .animate_slot_wt_lfo_phase[i]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_wt_lfo_snh[i]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_sample_start[i]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_loop_start[i]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_loop_end[i]
            .store(1.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_filter_type[i]
            .store(0, Ordering::Relaxed);
        track
            .animate_slot_filter_cutoff[i]
            .store(0.5f32.to_bits(), Ordering::Relaxed);
        track
            .animate_slot_filter_resonance[i]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
    }
    track.animate_vector_x.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.animate_vector_y.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.animate_vector_x_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.animate_vector_y_smooth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_x_waveform.store(0, Ordering::Relaxed);
    track.animate_lfo_x_sync.store(false, Ordering::Relaxed);
    track.animate_lfo_x_division.store(0, Ordering::Relaxed);
    track.animate_lfo_x_rate.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_x_amount.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_x_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_x_snh.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_y_waveform.store(0, Ordering::Relaxed);
    track.animate_lfo_y_sync.store(false, Ordering::Relaxed);
    track.animate_lfo_y_division.store(0, Ordering::Relaxed);
    track.animate_lfo_y_rate.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_y_amount.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_y_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_y_snh.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.animate_lfo_rng_state.store(0x2468_ace1, Ordering::Relaxed);
    for i in 0..160 {
        track.animate_sequencer_grid[i].store(false, Ordering::Relaxed);
    }
    track.animate_sequencer_step.store(-1, Ordering::Relaxed);
    track.animate_sequencer_phase.store(0, Ordering::Relaxed);
    for voice in 0..10 {
        for slot in 0..4 {
            track.animate_slot_phases[voice][slot].store(0.0f32.to_bits(), Ordering::Relaxed);
            track.animate_slot_sample_pos[voice][slot].store(0.0f32.to_bits(), Ordering::Relaxed);
            track.animate_slot_filter_v1[voice][slot].store(0.0f32.to_bits(), Ordering::Relaxed);
            track.animate_slot_filter_v2[voice][slot].store(0.0f32.to_bits(), Ordering::Relaxed);
            track
                .animate_slot_filter_v1_stage2[voice][slot]
                .store(0.0f32.to_bits(), Ordering::Relaxed);
            track
                .animate_slot_filter_v2_stage2[voice][slot]
                .store(0.0f32.to_bits(), Ordering::Relaxed);
        }
        track.animate_amp_stage[voice].store(0, Ordering::Relaxed);
        track.animate_amp_level[voice].store(0.0f32.to_bits(), Ordering::Relaxed);
    }
    track.animate_keybed_note.store(60, Ordering::Relaxed);
    track.animate_keybed_trigger.store(false, Ordering::Relaxed);
    track.animate_keybed_hold.store(false, Ordering::Relaxed);
    track.animate_keybed_amp_stage.store(0, Ordering::Relaxed);
    track.animate_keybed_amp_level.store(0.0f32.to_bits(), Ordering::Relaxed);
    for slot in 0..4 {
        track
            .animate_keybed_slot_phases[slot]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_keybed_slot_sample_pos[slot]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_keybed_filter_v1[slot]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_keybed_filter_v2[slot]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_keybed_filter_v1_stage2[slot]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        track
            .animate_keybed_filter_v2_stage2[slot]
            .store(0.0f32.to_bits(), Ordering::Relaxed);
    }
    track.kick_pitch.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.kick_decay.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.kick_attack.store(0.0f32.to_bits(), Ordering::Relaxed);
    track
        .kick_pitch_env_amount
        .store(0.0f32.to_bits(), Ordering::Relaxed);
    track.kick_drive.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.kick_level.store(1.0f32.to_bits(), Ordering::Relaxed);
    track.kick_filter_type.store(0, Ordering::Relaxed);
    track.kick_filter_cutoff.store(0.6f32.to_bits(), Ordering::Relaxed);
    track.kick_filter_resonance.store(0.2f32.to_bits(), Ordering::Relaxed);
    track.kick_filter_pre_drive.store(true, Ordering::Relaxed);
    for i in 0..SYNDRM_STEPS {
        track.kick_sequencer_grid[i].store(false, Ordering::Relaxed);
    }
    track.kick_sequencer_step.store(-1, Ordering::Relaxed);
    track.kick_sequencer_phase.store(0, Ordering::Relaxed);
    track.kick_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.kick_env.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.kick_pitch_env.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.kick_attack_remaining.store(0, Ordering::Relaxed);
    track.snare_tone.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.snare_decay.store(0.4f32.to_bits(), Ordering::Relaxed);
    track.snare_snappy.store(0.6f32.to_bits(), Ordering::Relaxed);
    track.snare_attack.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.snare_drive.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.snare_level.store(0.8f32.to_bits(), Ordering::Relaxed);
    track.snare_filter_type.store(0, Ordering::Relaxed);
    track.snare_filter_cutoff.store(0.6f32.to_bits(), Ordering::Relaxed);
    track.snare_filter_resonance.store(0.2f32.to_bits(), Ordering::Relaxed);
    track.snare_filter_pre_drive.store(true, Ordering::Relaxed);
    for i in 0..SYNDRM_STEPS {
        track.snare_sequencer_grid[i].store(false, Ordering::Relaxed);
    }
    track.snare_sequencer_step.store(-1, Ordering::Relaxed);
    track.snare_sequencer_phase.store(0, Ordering::Relaxed);
    track.snare_phase.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.snare_env.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.snare_noise_env.store(0.0f32.to_bits(), Ordering::Relaxed);
    track.snare_attack_remaining.store(0, Ordering::Relaxed);
    track.snare_noise_rng.store(0xdead_beef, Ordering::Relaxed);
    track.syndrm_page.store(0, Ordering::Relaxed);
    track.syndrm_edit_lane.store(0, Ordering::Relaxed);
    track.syndrm_edit_step.store(0, Ordering::Relaxed);
    track.syndrm_step_hold.store(false, Ordering::Relaxed);
    track.syndrm_rng_state.store(0x81c3_5f27, Ordering::Relaxed);
    for i in 0..SYNDRM_STEPS {
        track.kick_step_override_enabled[i].store(false, Ordering::Relaxed);
        track.kick_step_pitch[i].store(0.5f32.to_bits(), Ordering::Relaxed);
        track.kick_step_decay[i].store(0.5f32.to_bits(), Ordering::Relaxed);
        track.kick_step_attack[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.kick_step_drive[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.kick_step_level[i].store(1.0f32.to_bits(), Ordering::Relaxed);
        track.kick_step_filter_type[i].store(0, Ordering::Relaxed);
        track.kick_step_filter_cutoff[i].store(0.6f32.to_bits(), Ordering::Relaxed);
        track.kick_step_filter_resonance[i].store(0.2f32.to_bits(), Ordering::Relaxed);
        track.snare_step_override_enabled[i].store(false, Ordering::Relaxed);
        track.snare_step_tone[i].store(0.5f32.to_bits(), Ordering::Relaxed);
        track.snare_step_decay[i].store(0.4f32.to_bits(), Ordering::Relaxed);
        track.snare_step_snappy[i].store(0.6f32.to_bits(), Ordering::Relaxed);
        track.snare_step_attack[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.snare_step_drive[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.snare_step_level[i].store(0.8f32.to_bits(), Ordering::Relaxed);
        track.snare_step_filter_type[i].store(0, Ordering::Relaxed);
        track.snare_step_filter_cutoff[i].store(0.6f32.to_bits(), Ordering::Relaxed);
        track.snare_step_filter_resonance[i].store(0.2f32.to_bits(), Ordering::Relaxed);
    }

    track.void_base_freq.store(40.0f32.to_bits(), Ordering::Relaxed);
    track.void_chaos_depth.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.void_entropy.store(0.2f32.to_bits(), Ordering::Relaxed);
    track.void_feedback.store(0.8f32.to_bits(), Ordering::Relaxed);
    track.void_diffusion.store(0.5f32.to_bits(), Ordering::Relaxed);
    track.void_mod_rate.store(0.1f32.to_bits(), Ordering::Relaxed);
    track.void_level.store(0.8f32.to_bits(), Ordering::Relaxed);
    track
        .void_base_freq_smooth
        .store(40.0f32.to_bits(), Ordering::Relaxed);
    track
        .void_chaos_depth_smooth
        .store(0.5f32.to_bits(), Ordering::Relaxed);
    track
        .void_entropy_smooth
        .store(0.2f32.to_bits(), Ordering::Relaxed);
    track
        .void_feedback_smooth
        .store(0.8f32.to_bits(), Ordering::Relaxed);
    track
        .void_diffusion_smooth
        .store(0.5f32.to_bits(), Ordering::Relaxed);
    track
        .void_mod_rate_smooth
        .store(0.1f32.to_bits(), Ordering::Relaxed);
    track
        .void_level_smooth
        .store(0.8f32.to_bits(), Ordering::Relaxed);
    track.void_internal_gain.store(0.0f32.to_bits(), Ordering::Relaxed);
    for i in 0..12 {
        track.void_osc_phases[i].store(0.0f32.to_bits(), Ordering::Relaxed);
        track.void_lfo_phases[i].store(0.0f32.to_bits(), Ordering::Relaxed);
    }
    track.void_lfo_chaos_phase.store(0, Ordering::Relaxed);
    track.void_filter_v1[0].store(0, Ordering::Relaxed);
    track.void_filter_v1[1].store(0, Ordering::Relaxed);
    track.void_filter_v2[0].store(0, Ordering::Relaxed);
    track.void_filter_v2[1].store(0, Ordering::Relaxed);
    if let Some(mut buffer) = track.void_delay_buffer.try_lock() {
        buffer[0].fill(0.0);
        buffer[1].fill(0.0);
    }
    track.void_delay_write_pos.store(0, Ordering::Relaxed);

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

impl TLBX1 {
    fn process_animate(
        track: &Track,
        track_output: &mut [Vec<f32>],
        num_buffer_samples: usize,
        global_tempo: &AtomicU32,
        animate_library: &AnimateLibrary,
        master_step: i32,
        master_phase: f32,
        samples_per_step: f32,
        transport_running: bool,
    ) {
        let sr = track.sample_rate.load(Ordering::Relaxed).max(1) as f32;
        let tempo_bits = global_tempo.load(Ordering::Relaxed);
        let tempo_raw = f32::from_bits(tempo_bits);
        let tempo = if tempo_raw.is_finite() {
            tempo_raw.clamp(20.0, 240.0)
        } else {
            120.0
        };

        // Sequencer timing
        let mut sequencer_phase = if transport_running {
            master_phase
        } else {
            f32::from_bits(track.animate_sequencer_phase.load(Ordering::Relaxed))
        };
        let mut current_step = if transport_running {
            master_step
        } else {
            track.animate_sequencer_step.load(Ordering::Relaxed)
        };
        if transport_running {
            track
                .animate_sequencer_step
                .store(current_step, Ordering::Relaxed);
        }

        // Animate Parameters
        let target_x = f32::from_bits(track.animate_vector_x.load(Ordering::Relaxed));
        let target_y = f32::from_bits(track.animate_vector_y.load(Ordering::Relaxed));
        let mut x_smooth = f32::from_bits(track.animate_vector_x_smooth.load(Ordering::Relaxed));
        let mut y_smooth = f32::from_bits(track.animate_vector_y_smooth.load(Ordering::Relaxed));
        let lfo_x_waveform = track.animate_lfo_x_waveform.load(Ordering::Relaxed);
        let lfo_x_sync = track.animate_lfo_x_sync.load(Ordering::Relaxed);
        let lfo_x_division = track.animate_lfo_x_division.load(Ordering::Relaxed);
        let lfo_x_rate =
            f32::from_bits(track.animate_lfo_x_rate.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let lfo_x_amount =
            f32::from_bits(track.animate_lfo_x_amount.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut lfo_x_phase =
            f32::from_bits(track.animate_lfo_x_phase.load(Ordering::Relaxed));
        let mut lfo_x_snh = f32::from_bits(track.animate_lfo_x_snh.load(Ordering::Relaxed));
        let lfo_y_waveform = track.animate_lfo_y_waveform.load(Ordering::Relaxed);
        let lfo_y_sync = track.animate_lfo_y_sync.load(Ordering::Relaxed);
        let lfo_y_division = track.animate_lfo_y_division.load(Ordering::Relaxed);
        let lfo_y_rate =
            f32::from_bits(track.animate_lfo_y_rate.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let lfo_y_amount =
            f32::from_bits(track.animate_lfo_y_amount.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut lfo_y_phase =
            f32::from_bits(track.animate_lfo_y_phase.load(Ordering::Relaxed));
        let mut lfo_y_snh = f32::from_bits(track.animate_lfo_y_snh.load(Ordering::Relaxed));
        let mut lfo_rng_state = track.animate_lfo_rng_state.load(Ordering::Relaxed);

        let lfo_x_rate_hz = if lfo_x_sync {
            let beats = lfo_division_beats(lfo_x_division);
            (tempo / 60.0) / beats
        } else {
            ANIMATE_LFO_RATE_MIN_HZ
                * (ANIMATE_LFO_RATE_MAX_HZ / ANIMATE_LFO_RATE_MIN_HZ).powf(lfo_x_rate)
        };
        let lfo_y_rate_hz = if lfo_y_sync {
            let beats = lfo_division_beats(lfo_y_division);
            (tempo / 60.0) / beats
        } else {
            ANIMATE_LFO_RATE_MIN_HZ
                * (ANIMATE_LFO_RATE_MAX_HZ / ANIMATE_LFO_RATE_MIN_HZ).powf(lfo_y_rate)
        };
        let wt_lfo_amount = [
            f32::from_bits(
                track.animate_slot_wt_lfo_amount[0].load(Ordering::Relaxed),
            )
            .clamp(0.0, 1.0),
            f32::from_bits(
                track.animate_slot_wt_lfo_amount[1].load(Ordering::Relaxed),
            )
            .clamp(0.0, 1.0),
            f32::from_bits(
                track.animate_slot_wt_lfo_amount[2].load(Ordering::Relaxed),
            )
            .clamp(0.0, 1.0),
            f32::from_bits(
                track.animate_slot_wt_lfo_amount[3].load(Ordering::Relaxed),
            )
            .clamp(0.0, 1.0),
        ];
        let wt_lfo_shape = [
            track.animate_slot_wt_lfo_shape[0].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_shape[1].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_shape[2].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_shape[3].load(Ordering::Relaxed),
        ];
        let wt_lfo_rate = [
            f32::from_bits(track.animate_slot_wt_lfo_rate[0].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_wt_lfo_rate[1].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_wt_lfo_rate[2].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_wt_lfo_rate[3].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
        ];
        let wt_lfo_sync = [
            track.animate_slot_wt_lfo_sync[0].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_sync[1].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_sync[2].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_sync[3].load(Ordering::Relaxed),
        ];
        let wt_lfo_division = [
            track.animate_slot_wt_lfo_division[0].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_division[1].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_division[2].load(Ordering::Relaxed),
            track.animate_slot_wt_lfo_division[3].load(Ordering::Relaxed),
        ];
        let mut wt_lfo_phase = [
            f32::from_bits(track.animate_slot_wt_lfo_phase[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_slot_wt_lfo_phase[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_slot_wt_lfo_phase[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_slot_wt_lfo_phase[3].load(Ordering::Relaxed)),
        ];
        let mut wt_lfo_snh = [
            f32::from_bits(track.animate_slot_wt_lfo_snh[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_slot_wt_lfo_snh[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_slot_wt_lfo_snh[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_slot_wt_lfo_snh[3].load(Ordering::Relaxed)),
        ];
        let wt_lfo_rate_hz = [
            if wt_lfo_sync[0] {
                let beats = lfo_division_beats(wt_lfo_division[0]);
                (tempo / 60.0) / beats
            } else {
                ANIMATE_LFO_RATE_MIN_HZ
                    * (ANIMATE_LFO_RATE_MAX_HZ / ANIMATE_LFO_RATE_MIN_HZ).powf(wt_lfo_rate[0])
            },
            if wt_lfo_sync[1] {
                let beats = lfo_division_beats(wt_lfo_division[1]);
                (tempo / 60.0) / beats
            } else {
                ANIMATE_LFO_RATE_MIN_HZ
                    * (ANIMATE_LFO_RATE_MAX_HZ / ANIMATE_LFO_RATE_MIN_HZ).powf(wt_lfo_rate[1])
            },
            if wt_lfo_sync[2] {
                let beats = lfo_division_beats(wt_lfo_division[2]);
                (tempo / 60.0) / beats
            } else {
                ANIMATE_LFO_RATE_MIN_HZ
                    * (ANIMATE_LFO_RATE_MAX_HZ / ANIMATE_LFO_RATE_MIN_HZ).powf(wt_lfo_rate[2])
            },
            if wt_lfo_sync[3] {
                let beats = lfo_division_beats(wt_lfo_division[3]);
                (tempo / 60.0) / beats
            } else {
                ANIMATE_LFO_RATE_MIN_HZ
                    * (ANIMATE_LFO_RATE_MAX_HZ / ANIMATE_LFO_RATE_MIN_HZ).powf(wt_lfo_rate[3])
            },
        ];
        let sample_start = [
            f32::from_bits(track.animate_slot_sample_start[0].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_sample_start[1].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_sample_start[2].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_sample_start[3].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
        ];
        let loop_start = [
            f32::from_bits(track.animate_slot_loop_start[0].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_loop_start[1].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_loop_start[2].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_loop_start[3].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
        ];
        let loop_end = [
            f32::from_bits(track.animate_slot_loop_end[0].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_loop_end[1].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_loop_end[2].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
            f32::from_bits(track.animate_slot_loop_end[3].load(Ordering::Relaxed))
                .clamp(0.0, 1.0),
        ];

        let attack = 0.01f32;
        let decay = 0.1f32;
        let sustain = 0.8f32;
        let release = 0.3f32;

        let mut amp_levels = [0.0f32; 10];
        let mut amp_stages = [0u32; 10];
        for i in 0..10 {
            amp_levels[i] = f32::from_bits(track.animate_amp_level[i].load(Ordering::Relaxed));
            amp_stages[i] = track.animate_amp_stage[i].load(Ordering::Relaxed);
        }

        let mut keybed_amp_level =
            f32::from_bits(track.animate_keybed_amp_level.load(Ordering::Relaxed));
        let mut keybed_amp_stage = track.animate_keybed_amp_stage.load(Ordering::Relaxed);
        let mut keybed_phases = [
            f32::from_bits(track.animate_keybed_slot_phases[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_slot_phases[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_slot_phases[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_slot_phases[3].load(Ordering::Relaxed)),
        ];
        let mut keybed_sample_pos = [
            f32::from_bits(track.animate_keybed_slot_sample_pos[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_slot_sample_pos[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_slot_sample_pos[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_slot_sample_pos[3].load(Ordering::Relaxed)),
        ];
        let mut keybed_filter_v1 = [
            f32::from_bits(track.animate_keybed_filter_v1[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v1[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v1[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v1[3].load(Ordering::Relaxed)),
        ];
        let mut keybed_filter_v2 = [
            f32::from_bits(track.animate_keybed_filter_v2[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v2[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v2[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v2[3].load(Ordering::Relaxed)),
        ];
        let mut keybed_filter_v1_stage2 = [
            f32::from_bits(track.animate_keybed_filter_v1_stage2[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v1_stage2[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v1_stage2[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v1_stage2[3].load(Ordering::Relaxed)),
        ];
        let mut keybed_filter_v2_stage2 = [
            f32::from_bits(track.animate_keybed_filter_v2_stage2[0].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v2_stage2[1].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v2_stage2[2].load(Ordering::Relaxed)),
            f32::from_bits(track.animate_keybed_filter_v2_stage2[3].load(Ordering::Relaxed)),
        ];

        let keybed_triggered = track
            .animate_keybed_trigger
            .swap(false, Ordering::Relaxed);
        let mut keybed_note = track.animate_keybed_note.load(Ordering::Relaxed);
        let mut keybed_freq = (440.0
            * 2.0_f32.powf((keybed_note as f32 - 69.0) / 12.0))
            .max(1.0);

        if keybed_triggered {
            keybed_note = track.animate_keybed_note.load(Ordering::Relaxed);
            keybed_freq = (440.0
                * 2.0_f32.powf((keybed_note as f32 - 69.0) / 12.0))
                .max(1.0);
            keybed_amp_stage = 1;
            keybed_amp_level = 0.0;
            for slot in 0..4 {
                if track.animate_slot_types[slot].load(Ordering::Relaxed) == 1 {
                    let smp_idx =
                        track.animate_slot_samples[slot].load(Ordering::Relaxed) as usize;
                    if let Some(smp) = animate_library.get_sample_cached(smp_idx) {
                        let len = smp.get(0).map(|ch| ch.len()).unwrap_or(0);
                        if len > 0 {
                            let start = (sample_start[slot]
                                * (len.saturating_sub(1) as f32))
                                .round()
                                .clamp(0.0, (len.saturating_sub(1)) as f32);
                            keybed_sample_pos[slot] = start;
                        }
                    }
                }
            }
        }

        
        // Pre-calculate envelope coefficients
        let calc_coef = |time_secs: f32| -> f32 {
            if time_secs <= 0.001 { 0.0 }
            else { (-1.0 / (time_secs * sr)).exp() }
        };

        let coef_attack = calc_coef(attack);
        let coef_decay = calc_coef(decay);
        let coef_release = calc_coef(release);
        let keybed_hold = track.animate_keybed_hold.load(Ordering::Relaxed);
        let keybed_sustain = if keybed_hold { 1.0f32 } else { 0.0f32 };

        let frequencies = [
            65.41, 73.42, 82.41, 98.00, 110.00, // C2, D2, E2, G2, A2
            130.81, 146.83, 164.81, 196.00, 220.00, // C3, D3, E3, G3, A3
        ];

        let num_channels = track_output.len();
        let output = track_output;

        for sample_idx in 0..num_buffer_samples {
            // Step sequencer (master-synced)
            if transport_running {
                sequencer_phase += 1.0;
                if sequencer_phase >= samples_per_step {
                    sequencer_phase -= samples_per_step;
                    current_step = (current_step + 1).rem_euclid(16);
                    track
                        .animate_sequencer_step
                        .store(current_step, Ordering::Relaxed);

                    // Update envelope stages for all voices based on grid
                    for row in 0..10 {
                        let note_active = track.animate_sequencer_grid[row * 16 + current_step as usize]
                            .load(Ordering::Relaxed);
                        if note_active {
                            if amp_stages[row] == 0 || amp_stages[row] == 4 {
                                amp_stages[row] = 1; // Attack
                                for slot in 0..4 {
                                    if track.animate_slot_types[slot].load(Ordering::Relaxed) == 1 {
                                        let smp_idx =
                                            track.animate_slot_samples[slot].load(Ordering::Relaxed)
                                                as usize;
                                        if let Some(smp) = animate_library.get_sample_cached(smp_idx) {
                                            let len = smp.get(0).map(|ch| ch.len()).unwrap_or(0);
                                            if len > 0 {
                                                let start = (sample_start[slot]
                                                    * (len.saturating_sub(1) as f32))
                                                    .round()
                                                    .clamp(0.0, (len.saturating_sub(1)) as f32);
                                                track.animate_slot_sample_pos[row][slot]
                                                    .store(start.to_bits(), Ordering::Relaxed);
                                            }
                                        }
                                    }
                                }
                            }
                        } else if amp_stages[row] != 0 && amp_stages[row] != 4 {
                            amp_stages[row] = 4; // Release
                        }
                    }
                }
            }

            let mut wt_lfo_values = [0.0f32; 4];
            for slot in 0..4 {
                wt_lfo_values[slot] =
                    lfo_waveform_value(wt_lfo_shape[slot], wt_lfo_phase[slot], wt_lfo_snh[slot]);
                wt_lfo_phase[slot] += wt_lfo_rate_hz[slot] / sr;
                if wt_lfo_phase[slot] >= 1.0 {
                    wt_lfo_phase[slot] -= 1.0;
                    if wt_lfo_shape[slot] == 4 {
                        wt_lfo_snh[slot] =
                            next_mosaic_rand_unit(&mut lfo_rng_state) * 2.0 - 1.0;
                    }
                }
            }

            // Process Envelopes for all voices
            for i in 0..10 {
                match amp_stages[i] {
                    1 => { // Attack
                        amp_levels[i] = amp_levels[i] * coef_attack + 1.1 * (1.0 - coef_attack);
                        if amp_levels[i] >= 1.0 {
                            amp_levels[i] = 1.0;
                            amp_stages[i] = 2;
                        }
                    }
                    2 => { // Decay
                        amp_levels[i] = amp_levels[i] * coef_decay + sustain * (1.0 - coef_decay);
                        if (amp_levels[i] - sustain).abs() < 0.001 {
                            amp_levels[i] = sustain;
                            amp_stages[i] = 3;
                        }
                    }
                    3 => { // Sustain
                        amp_levels[i] = sustain;
                    }
                    4 => { // Release
                        amp_levels[i] = amp_levels[i] * coef_release;
                        if amp_levels[i] < 0.0001 {
                            amp_levels[i] = 0.0;
                            amp_stages[i] = 0;
                        }
                    }
                    _ => {
                        amp_levels[i] = 0.0;
                    }
                }
            }

            match keybed_amp_stage {
                1 => {
                    keybed_amp_level =
                        keybed_amp_level * coef_attack + 1.1 * (1.0 - coef_attack);
                    if keybed_amp_level >= 1.0 {
                        keybed_amp_level = 1.0;
                        keybed_amp_stage = 2;
                    }
                }
                2 => {
                    keybed_amp_level =
                        keybed_amp_level * coef_decay + keybed_sustain * (1.0 - coef_decay);
                    if (keybed_amp_level - keybed_sustain).abs() < 0.001 {
                        keybed_amp_level = keybed_sustain;
                        keybed_amp_stage = if keybed_hold { 3 } else { 4 };
                    }
                }
                3 => {
                    if keybed_hold {
                        keybed_amp_level = keybed_sustain;
                    } else {
                        keybed_amp_stage = 4;
                    }
                }
                4 => {
                    keybed_amp_level = keybed_amp_level * coef_release;
                    if keybed_amp_level < 0.0001 {
                        keybed_amp_level = 0.0;
                        keybed_amp_stage = 0;
                    }
                }
                _ => {
                    keybed_amp_level = 0.0;
                }
            }

            // Smooth vector position
            let lfo_x_value = lfo_waveform_value(lfo_x_waveform, lfo_x_phase, lfo_x_snh);
            let lfo_y_value = lfo_waveform_value(lfo_y_waveform, lfo_y_phase, lfo_y_snh);
            let target_x_mod = (target_x + lfo_x_value * lfo_x_amount).clamp(0.0, 1.0);
            let target_y_mod = (target_y + lfo_y_value * lfo_y_amount).clamp(0.0, 1.0);
            x_smooth = x_smooth * 0.999 + target_x_mod * 0.001;
            y_smooth = y_smooth * 0.999 + target_y_mod * 0.001;

            lfo_x_phase += lfo_x_rate_hz / sr;
            if lfo_x_phase >= 1.0 {
                lfo_x_phase -= 1.0;
                if lfo_x_waveform == 4 {
                    lfo_x_snh = next_mosaic_rand_unit(&mut lfo_rng_state) * 2.0 - 1.0;
                }
            }
            lfo_y_phase += lfo_y_rate_hz / sr;
            if lfo_y_phase >= 1.0 {
                lfo_y_phase -= 1.0;
                if lfo_y_waveform == 4 {
                    lfo_y_snh = next_mosaic_rand_unit(&mut lfo_rng_state) * 2.0 - 1.0;
                }
            }

            // Calculate weights for 4 slots
            let w_a = (1.0 - x_smooth) * (1.0 - y_smooth);
            let w_b = x_smooth * (1.0 - y_smooth);
            let w_c = (1.0 - x_smooth) * y_smooth;
            let w_d = x_smooth * y_smooth;
            let weights = [w_a, w_b, w_c, w_d];

            let mut mixed_sample_l = 0.0f32;
            let mut mixed_sample_r = 0.0f32;

            // Sum active voices
            for row in 0..10 {
                if amp_levels[row] <= 0.0 {
                    continue;
                }
                
                let base_freq = frequencies[row as usize];
                
                for slot in 0..4 {
                    let slot_type = track.animate_slot_types[slot].load(Ordering::Relaxed);
                    let coarse = f32::from_bits(track.animate_slot_coarse[slot].load(Ordering::Relaxed));
                    let fine = f32::from_bits(track.animate_slot_fine[slot].load(Ordering::Relaxed));
                    let pitch_ratio = 2.0f32.powf((coarse + fine / 100.0) / 12.0);
                    let freq = base_freq * pitch_ratio;

                    let mut slot_sample = 0.0f32;
                    if slot_type == 0 { // Wavetable
                        let wt_idx =
                            track.animate_slot_wavetables[slot].load(Ordering::Relaxed) as usize;
                        if let Some(wt) = animate_library.get_wavetable_cached(wt_idx) {
                            if !wt.is_empty() {
                                let phase = f32::from_bits(track.animate_slot_phases[row][slot].load(Ordering::Relaxed));
                                // Use first cycle (2048 samples)
                                let cycle_len = wt.len().min(2048);
                                if cycle_len > 0 {
                                    let num_cycles = wt.len() / cycle_len;
                                    let cycle_offset = if num_cycles > 1 && wt_lfo_amount[slot] > 0.0 {
                                        let lfo_pos = (wt_lfo_values[slot] * 0.5 + 0.5).clamp(0.0, 1.0);
                                        let max_idx = (num_cycles - 1) as f32;
                                        (lfo_pos * wt_lfo_amount[slot] * max_idx).round() as usize
                                    } else {
                                        0
                                    };
                                    let base = cycle_offset * cycle_len;
                                    let pos = phase * (cycle_len - 1) as f32;
                                    let idx = pos as usize;
                                    let frac = pos - idx as f32;
                                    let s1 = wt[base + idx];
                                    let s2 = wt[base + ((idx + 1) % cycle_len)];
                                    slot_sample = s1 + (s2 - s1) * frac;
                                }

                                let new_phase = (phase + freq / sr) % 1.0;
                                track.animate_slot_phases[row][slot].store(new_phase.to_bits(), Ordering::Relaxed);
                            }
                        }
                    } else { // Sample
                        let smp_idx =
                            track.animate_slot_samples[slot].load(Ordering::Relaxed) as usize;
                        if let Some(smp) = animate_library.get_sample_cached(smp_idx) {
                            if !smp.is_empty() && !smp[0].is_empty() {
                                let len = smp[0].len();
                                if len > 0 {
                                    let mut pos =
                                        f32::from_bits(track.animate_slot_sample_pos[row][slot].load(Ordering::Relaxed));
                                    let start_idx = (sample_start[slot] * (len.saturating_sub(1) as f32))
                                        .round()
                                        .clamp(0.0, (len.saturating_sub(1)) as f32) as usize;
                                    let mut loop_start_idx =
                                        (loop_start[slot] * (len.saturating_sub(1) as f32))
                                            .round()
                                            .clamp(0.0, (len.saturating_sub(1)) as f32) as usize;
                                    let mut loop_end_idx =
                                        (loop_end[slot] * (len.saturating_sub(1) as f32))
                                            .round()
                                            .clamp(0.0, (len.saturating_sub(1)) as f32) as usize;
                                    if loop_end_idx <= loop_start_idx {
                                        loop_start_idx = start_idx.min(len.saturating_sub(1));
                                        loop_end_idx = len.saturating_sub(1);
                                    }
                                    let mut idx = pos as usize;
                                    if idx >= loop_end_idx {
                                        pos = loop_start_idx as f32;
                                        idx = loop_start_idx;
                                    }
                                    if idx < len {
                                        slot_sample = smp[0][idx];
                                        let mut new_pos = pos + (freq / 440.0); // Rough sample playback
                                        if new_pos >= loop_end_idx as f32 {
                                            new_pos = loop_start_idx as f32;
                                        }
                                        track.animate_slot_sample_pos[row][slot]
                                            .store(new_pos.to_bits(), Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                    }

                    let filter_type =
                        track.animate_slot_filter_type[slot].load(Ordering::Relaxed) as u32;
                    let filter_cutoff = f32::from_bits(
                        track.animate_slot_filter_cutoff[slot].load(Ordering::Relaxed),
                    )
                    .clamp(0.001, 1.0);
                    let filter_resonance = f32::from_bits(
                        track.animate_slot_filter_resonance[slot].load(Ordering::Relaxed),
                    )
                    .clamp(0.0, 0.95);
                    let cutoff_hz = 20.0 + filter_cutoff.powf(2.0) * (20_000.0 - 20.0);
                    let filter_f =
                        (2.0 * (std::f32::consts::PI * cutoff_hz / sr).sin()).clamp(0.0, 0.99);
                    let filter_q = 1.0 - filter_resonance;
                    let filter_v1 = f32::from_bits(
                        track.animate_slot_filter_v1[row][slot].load(Ordering::Relaxed),
                    );
                    let filter_v2 = f32::from_bits(
                        track.animate_slot_filter_v2[row][slot].load(Ordering::Relaxed),
                    );
                    let filter_v1_stage2 = f32::from_bits(
                        track
                            .animate_slot_filter_v1_stage2[row][slot]
                            .load(Ordering::Relaxed),
                    );
                    let filter_v2_stage2 = f32::from_bits(
                        track
                            .animate_slot_filter_v2_stage2[row][slot]
                            .load(Ordering::Relaxed),
                    );
                    let filter_low = filter_v2 + filter_f * filter_v1;
                    let filter_high = slot_sample - filter_low - filter_q * filter_v1;
                    let filter_band = filter_f * filter_high + filter_v1;
                    let filter_low_stage2 = filter_v2_stage2 + filter_f * filter_v1_stage2;
                    let filter_high_stage2 = filter_low - filter_low_stage2 - filter_q * filter_v1_stage2;
                    let filter_band_stage2 = filter_f * filter_high_stage2 + filter_v1_stage2;
                    let mut filtered_sample = match filter_type {
                        0 => filter_low_stage2,
                        1 => filter_low,
                        2 => filter_high,
                        3 => filter_band,
                        _ => filter_low,
                    };
                    if !filtered_sample.is_finite() {
                        filtered_sample = slot_sample;
                        track.animate_slot_filter_v1[row][slot]
                            .store(0.0f32.to_bits(), Ordering::Relaxed);
                        track.animate_slot_filter_v2[row][slot]
                            .store(0.0f32.to_bits(), Ordering::Relaxed);
                        track.animate_slot_filter_v1_stage2[row][slot]
                            .store(0.0f32.to_bits(), Ordering::Relaxed);
                        track.animate_slot_filter_v2_stage2[row][slot]
                            .store(0.0f32.to_bits(), Ordering::Relaxed);
                    } else {
                        track.animate_slot_filter_v1[row][slot]
                            .store(filter_band.to_bits(), Ordering::Relaxed);
                        track.animate_slot_filter_v2[row][slot]
                            .store(filter_low.to_bits(), Ordering::Relaxed);
                        track.animate_slot_filter_v1_stage2[row][slot]
                            .store(filter_band_stage2.to_bits(), Ordering::Relaxed);
                        track.animate_slot_filter_v2_stage2[row][slot]
                            .store(filter_low_stage2.to_bits(), Ordering::Relaxed);
                    }
                    slot_sample = filtered_sample;

                    let level = f32::from_bits(track.animate_slot_level[slot].load(Ordering::Relaxed)) * weights[slot] * amp_levels[row];
                    let pan = f32::from_bits(track.animate_slot_pan[slot].load(Ordering::Relaxed)).clamp(-1.0, 1.0);
                    
                    let left_gain = (1.0 - pan).min(1.0);
                    let right_gain = (1.0 + pan).min(1.0);
                    
                    mixed_sample_l += slot_sample * level * left_gain;
                    mixed_sample_r += slot_sample * level * right_gain;
                }
            }

            if keybed_amp_level > 0.0 {
                for slot in 0..4 {
                    let slot_type = track.animate_slot_types[slot].load(Ordering::Relaxed);
                    let coarse =
                        f32::from_bits(track.animate_slot_coarse[slot].load(Ordering::Relaxed));
                    let fine =
                        f32::from_bits(track.animate_slot_fine[slot].load(Ordering::Relaxed));
                    let pitch_ratio = 2.0f32.powf((coarse + fine / 100.0) / 12.0);
                    let freq = keybed_freq * pitch_ratio;

                    let mut slot_sample = 0.0f32;
                    if slot_type == 0 {
                        let wt_idx =
                            track.animate_slot_wavetables[slot].load(Ordering::Relaxed) as usize;
                        if let Some(wt) = animate_library.get_wavetable_cached(wt_idx) {
                            if !wt.is_empty() {
                                let phase = keybed_phases[slot];
                                let cycle_len = wt.len().min(2048);
                                if cycle_len > 0 {
                                    let num_cycles = wt.len() / cycle_len;
                                    let cycle_offset = if num_cycles > 1
                                        && wt_lfo_amount[slot] > 0.0
                                    {
                                        let lfo_pos =
                                            (wt_lfo_values[slot] * 0.5 + 0.5).clamp(0.0, 1.0);
                                        let max_idx = (num_cycles - 1) as f32;
                                        (lfo_pos * wt_lfo_amount[slot] * max_idx).round() as usize
                                    } else {
                                        0
                                    };
                                    let base = cycle_offset * cycle_len;
                                    let pos = phase * (cycle_len - 1) as f32;
                                    let idx = pos as usize;
                                    let frac = pos - idx as f32;
                                    let s1 = wt[base + idx];
                                    let s2 = wt[base + ((idx + 1) % cycle_len)];
                                    slot_sample = s1 + (s2 - s1) * frac;
                                }
                                keybed_phases[slot] = (phase + freq / sr) % 1.0;
                            }
                        }
                    } else {
                        let smp_idx =
                            track.animate_slot_samples[slot].load(Ordering::Relaxed) as usize;
                        if let Some(smp) = animate_library.get_sample_cached(smp_idx) {
                            if !smp.is_empty() && !smp[0].is_empty() {
                                let len = smp[0].len();
                                if len > 0 {
                                    let mut pos = keybed_sample_pos[slot];
                                    let start_idx = (sample_start[slot]
                                        * (len.saturating_sub(1) as f32))
                                        .round()
                                        .clamp(0.0, (len.saturating_sub(1)) as f32)
                                        as usize;
                                    let mut loop_start_idx = (loop_start[slot]
                                        * (len.saturating_sub(1) as f32))
                                        .round()
                                        .clamp(0.0, (len.saturating_sub(1)) as f32)
                                        as usize;
                                    let mut loop_end_idx = (loop_end[slot]
                                        * (len.saturating_sub(1) as f32))
                                        .round()
                                        .clamp(0.0, (len.saturating_sub(1)) as f32)
                                        as usize;
                                    if loop_end_idx <= loop_start_idx {
                                        loop_start_idx = start_idx.min(len.saturating_sub(1));
                                        loop_end_idx = len.saturating_sub(1);
                                    }
                                    let mut idx = pos as usize;
                                    if idx >= loop_end_idx {
                                        pos = loop_start_idx as f32;
                                        idx = loop_start_idx;
                                    }
                                    if idx < len {
                                        slot_sample = smp[0][idx];
                                        let mut new_pos = pos + (freq / 440.0);
                                        if new_pos >= loop_end_idx as f32 {
                                            new_pos = loop_start_idx as f32;
                                        }
                                        keybed_sample_pos[slot] = new_pos;
                                    }
                                }
                            }
                        }
                    }

                    let filter_type =
                        track.animate_slot_filter_type[slot].load(Ordering::Relaxed) as u32;
                    let filter_cutoff = f32::from_bits(
                        track.animate_slot_filter_cutoff[slot].load(Ordering::Relaxed),
                    )
                    .clamp(0.001, 1.0);
                    let filter_resonance = f32::from_bits(
                        track.animate_slot_filter_resonance[slot].load(Ordering::Relaxed),
                    )
                    .clamp(0.0, 0.95);
                    let cutoff_hz = 20.0 + filter_cutoff.powf(2.0) * (20_000.0 - 20.0);
                    let filter_f =
                        (2.0 * (std::f32::consts::PI * cutoff_hz / sr).sin()).clamp(0.0, 0.99);
                    let filter_q = 1.0 - filter_resonance;
                    let mut filter_v1 = keybed_filter_v1[slot];
                    let mut filter_v2 = keybed_filter_v2[slot];
                    let mut filter_v1_stage2 = keybed_filter_v1_stage2[slot];
                    let mut filter_v2_stage2 = keybed_filter_v2_stage2[slot];
                    let filter_low = filter_v2 + filter_f * filter_v1;
                    let filter_high = slot_sample - filter_low - filter_q * filter_v1;
                    let filter_band = filter_f * filter_high + filter_v1;
                    let filter_low_stage2 = filter_v2_stage2 + filter_f * filter_v1_stage2;
                    let filter_high_stage2 =
                        filter_low - filter_low_stage2 - filter_q * filter_v1_stage2;
                    let filter_band_stage2 = filter_f * filter_high_stage2 + filter_v1_stage2;
                    let mut filtered_sample = match filter_type {
                        0 => filter_low_stage2,
                        1 => filter_low,
                        2 => filter_high,
                        3 => filter_band,
                        _ => filter_low,
                    };
                    if !filtered_sample.is_finite() {
                        filtered_sample = slot_sample;
                        keybed_filter_v1[slot] = 0.0;
                        keybed_filter_v2[slot] = 0.0;
                        keybed_filter_v1_stage2[slot] = 0.0;
                        keybed_filter_v2_stage2[slot] = 0.0;
                    } else {
                        filter_v1 = filter_band;
                        filter_v2 = filter_low;
                        filter_v1_stage2 = filter_band_stage2;
                        filter_v2_stage2 = filter_low_stage2;
                        keybed_filter_v1[slot] = filter_v1;
                        keybed_filter_v2[slot] = filter_v2;
                        keybed_filter_v1_stage2[slot] = filter_v1_stage2;
                        keybed_filter_v2_stage2[slot] = filter_v2_stage2;
                    }
                    slot_sample = filtered_sample;

                    let level = f32::from_bits(track.animate_slot_level[slot].load(Ordering::Relaxed))
                        * weights[slot]
                        * keybed_amp_level;
                    let pan = f32::from_bits(track.animate_slot_pan[slot].load(Ordering::Relaxed))
                        .clamp(-1.0, 1.0);
                    let left_gain = (1.0 - pan).min(1.0);
                    let right_gain = (1.0 + pan).min(1.0);
                    mixed_sample_l += slot_sample * level * left_gain;
                    mixed_sample_r += slot_sample * level * right_gain;
                }
            }

            for ch in 0..num_channels.min(2) {
                let input = if ch == 0 { mixed_sample_l } else { mixed_sample_r };
                output[ch][sample_idx] += input;
            }
        }

        if transport_running {
            track
                .animate_sequencer_phase
                .store(sequencer_phase.round().max(0.0) as u32, Ordering::Relaxed);
        }
        track.animate_vector_x_smooth.store(x_smooth.to_bits(), Ordering::Relaxed);
        track.animate_vector_y_smooth.store(y_smooth.to_bits(), Ordering::Relaxed);
        track
            .animate_lfo_x_phase
            .store(lfo_x_phase.to_bits(), Ordering::Relaxed);
        track
            .animate_lfo_x_snh
            .store(lfo_x_snh.to_bits(), Ordering::Relaxed);
        track
            .animate_lfo_y_phase
            .store(lfo_y_phase.to_bits(), Ordering::Relaxed);
        track
            .animate_lfo_y_snh
            .store(lfo_y_snh.to_bits(), Ordering::Relaxed);
        track
            .animate_lfo_rng_state
            .store(lfo_rng_state, Ordering::Relaxed);
        for slot in 0..4 {
            track.animate_slot_wt_lfo_phase[slot]
                .store(wt_lfo_phase[slot].to_bits(), Ordering::Relaxed);
            track.animate_slot_wt_lfo_snh[slot]
                .store(wt_lfo_snh[slot].to_bits(), Ordering::Relaxed);
        }
        for i in 0..10 {
            track.animate_amp_stage[i].store(amp_stages[i], Ordering::Relaxed);
            track.animate_amp_level[i].store(amp_levels[i].to_bits(), Ordering::Relaxed);
        }
        track
            .animate_keybed_amp_stage
            .store(keybed_amp_stage, Ordering::Relaxed);
        track
            .animate_keybed_amp_level
            .store(keybed_amp_level.to_bits(), Ordering::Relaxed);
        for slot in 0..4 {
            track
                .animate_keybed_slot_phases[slot]
                .store(keybed_phases[slot].to_bits(), Ordering::Relaxed);
            track
                .animate_keybed_slot_sample_pos[slot]
                .store(keybed_sample_pos[slot].to_bits(), Ordering::Relaxed);
            track
                .animate_keybed_filter_v1[slot]
                .store(keybed_filter_v1[slot].to_bits(), Ordering::Relaxed);
            track
                .animate_keybed_filter_v2[slot]
                .store(keybed_filter_v2[slot].to_bits(), Ordering::Relaxed);
            track
                .animate_keybed_filter_v1_stage2[slot]
                .store(keybed_filter_v1_stage2[slot].to_bits(), Ordering::Relaxed);
            track
                .animate_keybed_filter_v2_stage2[slot]
                .store(keybed_filter_v2_stage2[slot].to_bits(), Ordering::Relaxed);
        }
    }

    fn process_syndrm(
        track: &Track,
        track_output: &mut [Vec<f32>],
        dsp_state: &mut SynDRMDspState,
        num_buffer_samples: usize,
        _global_tempo: &AtomicU32,
        _master_step: i32,
        master_phase: f32,
        master_step_count: i64,
        samples_per_step: f32,
        sample_rate: f32,
        transport_running: bool,
    ) {
        let sr = sample_rate.max(1.0);
        dsp_state.set_sample_rate(sr);
        let mut max_active_step = None;
        for i in 0..SYNDRM_STEPS {
            if track.kick_sequencer_grid[i].load(Ordering::Relaxed)
                || track.snare_sequencer_grid[i].load(Ordering::Relaxed)
            {
                max_active_step = Some(i);
            }
        }
        let mut loop_steps = SYNDRM_PAGE_SIZE;
        if let Some(max_step) = max_active_step {
            loop_steps = ((max_step / SYNDRM_PAGE_SIZE) + 1) * SYNDRM_PAGE_SIZE;
        }
        let loop_steps_i32 = loop_steps.max(1) as i32;
        let mut sequencer_phase = if transport_running {
            master_phase
        } else {
            f32::from_bits(track.kick_sequencer_phase.load(Ordering::Relaxed))
        };
        let mut current_step = if transport_running {
            (master_step_count as i32).rem_euclid(loop_steps_i32)
        } else {
            let step = track.kick_sequencer_step.load(Ordering::Relaxed);
            if step < 0 {
                step
            } else {
                step.rem_euclid(loop_steps_i32)
            }
        };
        if transport_running {
            track.kick_sequencer_step.store(current_step, Ordering::Relaxed);
            track.snare_sequencer_step.store(current_step, Ordering::Relaxed);
        }

        let mut kick_pitch_base =
            f32::from_bits(track.kick_pitch.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut kick_decay_base =
            f32::from_bits(track.kick_decay.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut kick_attack_base =
            f32::from_bits(track.kick_attack.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let kick_pitch_env_amount =
            f32::from_bits(track.kick_pitch_env_amount.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut kick_drive_base =
            f32::from_bits(track.kick_drive.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let track_muted = track.is_muted.load(Ordering::Relaxed);
        let mut kick_level_base =
            f32::from_bits(track.kick_level.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut kick_filter_type_base = track.kick_filter_type.load(Ordering::Relaxed);
        let mut kick_filter_cutoff_base =
            f32::from_bits(track.kick_filter_cutoff.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut kick_filter_resonance_base =
            f32::from_bits(track.kick_filter_resonance.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let kick_filter_pre_drive = track.kick_filter_pre_drive.load(Ordering::Relaxed);
        let mut snare_tone_base =
            f32::from_bits(track.snare_tone.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut snare_decay_base =
            f32::from_bits(track.snare_decay.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut snare_snappy_base =
            f32::from_bits(track.snare_snappy.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut snare_attack_base =
            f32::from_bits(track.snare_attack.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut snare_drive_base =
            f32::from_bits(track.snare_drive.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut snare_level_base =
            f32::from_bits(track.snare_level.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut snare_filter_type_base = track.snare_filter_type.load(Ordering::Relaxed);
        let mut snare_filter_cutoff_base =
            f32::from_bits(track.snare_filter_cutoff.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut snare_filter_resonance_base =
            f32::from_bits(track.snare_filter_resonance.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let snare_filter_pre_drive = track.snare_filter_pre_drive.load(Ordering::Relaxed);

        let mut env = f32::from_bits(track.kick_env.load(Ordering::Relaxed));
        let mut pitch_env = f32::from_bits(track.kick_pitch_env.load(Ordering::Relaxed));
        let mut attack_remaining = track.kick_attack_remaining.load(Ordering::Relaxed);
        let mut snare_env = f32::from_bits(track.snare_env.load(Ordering::Relaxed));
        let mut snare_noise_env =
            f32::from_bits(track.snare_noise_env.load(Ordering::Relaxed));
        let mut snare_attack_remaining =
            track.snare_attack_remaining.load(Ordering::Relaxed);

        let cutoff_min = 20.0;
        let cutoff_max = 12_000.0;
        let cutoff_span: f32 = cutoff_max / cutoff_min;
        let step_hold = track.syndrm_step_hold.load(Ordering::Relaxed);

        let mut kick_pitch = kick_pitch_base;
        let mut kick_decay = kick_decay_base;
        let mut kick_attack = kick_attack_base;
        let mut kick_drive = kick_drive_base;
        let mut kick_level = kick_level_base;
        let mut kick_filter_type = kick_filter_type_base;
        let mut kick_filter_cutoff = kick_filter_cutoff_base;
        let mut kick_filter_resonance = kick_filter_resonance_base;
        let mut snare_tone = snare_tone_base;
        let mut snare_decay = snare_decay_base;
        let mut snare_snappy = snare_snappy_base;
        let mut snare_attack = snare_attack_base;
        let mut snare_drive = snare_drive_base;
        let mut snare_level = snare_level_base;
        let mut snare_filter_type = snare_filter_type_base;
        let mut snare_filter_cutoff = snare_filter_cutoff_base;
        let mut snare_filter_resonance = snare_filter_resonance_base;

        let mut decay_time = 0.05 + kick_decay * 1.5;
        let mut pitch_decay_time = (0.01 + kick_decay * 0.25)
            * (1.0 - 0.7 * kick_pitch_env_amount).max(0.1);
        let mut env_coeff = (-1.0 / (decay_time * sr)).exp();
        let mut attack_time = kick_attack * 0.01;
        let mut attack_samples = (attack_time * sr).round().max(0.0) as u32;
        let mut attack_step = if attack_samples > 0 {
            1.0 / attack_samples as f32
        } else {
            1.0
        };
        let mut pitch_coeff = (-1.0 / (pitch_decay_time * sr)).exp();
        let mut base_freq = 40.0 + kick_pitch * 120.0;
        let mut sweep = base_freq * (kick_pitch_env_amount * 4.0);
        let mut drive = 1.0 + kick_drive * 8.0;
        let mut kick_cutoff_hz = cutoff_min * cutoff_span.powf(kick_filter_cutoff);
        let mut kick_q = 0.1 + kick_filter_resonance * 0.9;
        let mut snare_decay_time = 0.03 + snare_decay * 0.4;
        let mut snare_noise_decay_time = 0.02 + snare_decay * 0.2;
        let mut snare_env_coeff = (-1.0 / (snare_decay_time * sr)).exp();
        let mut snare_noise_coeff = (-1.0 / (snare_noise_decay_time * sr)).exp();
        let mut snare_attack_time = snare_attack * 0.01;
        let mut snare_attack_samples = (snare_attack_time * sr).round().max(0.0) as u32;
        let mut snare_attack_step = if snare_attack_samples > 0 {
            1.0 / snare_attack_samples as f32
        } else {
            1.0
        };
        let mut snare_freq = 180.0 + snare_tone * 420.0;
        let mut snare_cutoff_hz = cutoff_min * cutoff_span.powf(snare_filter_cutoff);
        let mut snare_q = 0.1 + snare_filter_resonance * 0.9;
        let mut snare_drive_gain = 1.0 + snare_drive * 8.0;

        fn apply_kick_step_params(
            track: &Track,
            step_idx: usize,
            step_hold: bool,
            track_muted: bool,
            cutoff_min: f32,
            cutoff_span: f32,
            sr: f32,
            kick_pitch: &mut f32,
            kick_decay: &mut f32,
            kick_attack: &mut f32,
            kick_pitch_env_amount: f32,
            kick_drive: &mut f32,
            kick_level: &mut f32,
            kick_filter_type: &mut u32,
            kick_filter_cutoff: &mut f32,
            kick_filter_resonance: &mut f32,
            kick_pitch_base: &mut f32,
            kick_decay_base: &mut f32,
            kick_attack_base: &mut f32,
            kick_drive_base: &mut f32,
            kick_level_base: &mut f32,
            kick_filter_type_base: &mut u32,
            kick_filter_cutoff_base: &mut f32,
            kick_filter_resonance_base: &mut f32,
            decay_time: &mut f32,
            pitch_decay_time: &mut f32,
            env_coeff: &mut f32,
            attack_time: &mut f32,
            attack_samples: &mut u32,
            attack_step: &mut f32,
            pitch_coeff: &mut f32,
            base_freq: &mut f32,
            sweep: &mut f32,
            drive: &mut f32,
            kick_cutoff_hz: &mut f32,
            kick_q: &mut f32,
        ) {
            let override_enabled =
                track.kick_step_override_enabled[step_idx].load(Ordering::Relaxed);
            if override_enabled {
                let pitch =
                    f32::from_bits(track.kick_step_pitch[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let decay =
                    f32::from_bits(track.kick_step_decay[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let attack =
                    f32::from_bits(track.kick_step_attack[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let drive_val =
                    f32::from_bits(track.kick_step_drive[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let level =
                    f32::from_bits(track.kick_step_level[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let filter_type = track.kick_step_filter_type[step_idx].load(Ordering::Relaxed);
                let filter_cutoff = f32::from_bits(
                    track.kick_step_filter_cutoff[step_idx].load(Ordering::Relaxed),
                )
                .clamp(0.0, 1.0);
                let filter_resonance = f32::from_bits(
                    track.kick_step_filter_resonance[step_idx].load(Ordering::Relaxed),
                )
                .clamp(0.0, 1.0);

                *kick_pitch = pitch;
                *kick_decay = decay;
                *kick_attack = attack;
                *kick_drive = drive_val;
                *kick_level = if track_muted { 0.0 } else { level };
                *kick_filter_type = filter_type;
                *kick_filter_cutoff = filter_cutoff;
                *kick_filter_resonance = filter_resonance;

                *decay_time = 0.05 + *kick_decay * 1.5;
                *pitch_decay_time = (0.01 + *kick_decay * 0.25)
                    * (1.0 - 0.7 * kick_pitch_env_amount).max(0.1);
                *env_coeff = (-1.0 / (*decay_time * sr)).exp();
                *attack_time = *kick_attack * 0.01;
                *attack_samples = (*attack_time * sr).round().max(0.0) as u32;
                *attack_step = if *attack_samples > 0 {
                    1.0 / *attack_samples as f32
                } else {
                    1.0
                };
                *pitch_coeff = (-1.0 / (*pitch_decay_time * sr)).exp();
                *base_freq = 40.0 + *kick_pitch * 120.0;
                *sweep = *base_freq * (kick_pitch_env_amount * 4.0);
                *drive = 1.0 + *kick_drive * 8.0;
                *kick_cutoff_hz = cutoff_min * cutoff_span.powf(*kick_filter_cutoff);
                *kick_q = 0.1 + *kick_filter_resonance * 0.9;

                if step_hold {
                    *kick_pitch_base = pitch;
                    *kick_decay_base = decay;
                    *kick_attack_base = attack;
                    *kick_drive_base = drive_val;
                    *kick_level_base = level;
                    *kick_filter_type_base = filter_type;
                    *kick_filter_cutoff_base = filter_cutoff;
                    *kick_filter_resonance_base = filter_resonance;
                    track.kick_pitch.store(pitch.to_bits(), Ordering::Relaxed);
                    track.kick_decay.store(decay.to_bits(), Ordering::Relaxed);
                    track.kick_attack.store(attack.to_bits(), Ordering::Relaxed);
                    track.kick_drive.store(drive_val.to_bits(), Ordering::Relaxed);
                    track.kick_level.store(level.to_bits(), Ordering::Relaxed);
                    track.kick_filter_type.store(filter_type, Ordering::Relaxed);
                    track.kick_filter_cutoff.store(filter_cutoff.to_bits(), Ordering::Relaxed);
                    track
                        .kick_filter_resonance
                        .store(filter_resonance.to_bits(), Ordering::Relaxed);
                }
            } else {
                *kick_pitch = *kick_pitch_base;
                *kick_decay = *kick_decay_base;
                *kick_attack = *kick_attack_base;
                *kick_drive = *kick_drive_base;
                *kick_level = if track_muted { 0.0 } else { *kick_level_base };
                *kick_filter_type = *kick_filter_type_base;
                *kick_filter_cutoff = *kick_filter_cutoff_base;
                *kick_filter_resonance = *kick_filter_resonance_base;

                *decay_time = 0.05 + *kick_decay * 1.5;
                *pitch_decay_time = (0.01 + *kick_decay * 0.25)
                    * (1.0 - 0.7 * kick_pitch_env_amount).max(0.1);
                *env_coeff = (-1.0 / (*decay_time * sr)).exp();
                *attack_time = *kick_attack * 0.01;
                *attack_samples = (*attack_time * sr).round().max(0.0) as u32;
                *attack_step = if *attack_samples > 0 {
                    1.0 / *attack_samples as f32
                } else {
                    1.0
                };
                *pitch_coeff = (-1.0 / (*pitch_decay_time * sr)).exp();
                *base_freq = 40.0 + *kick_pitch * 120.0;
                *sweep = *base_freq * (kick_pitch_env_amount * 4.0);
                *drive = 1.0 + *kick_drive * 8.0;
                *kick_cutoff_hz = cutoff_min * cutoff_span.powf(*kick_filter_cutoff);
                *kick_q = 0.1 + *kick_filter_resonance * 0.9;
            }
        }

        fn apply_snare_step_params(
            track: &Track,
            step_idx: usize,
            step_hold: bool,
            track_muted: bool,
            cutoff_min: f32,
            cutoff_span: f32,
            sr: f32,
            snare_tone: &mut f32,
            snare_decay: &mut f32,
            snare_snappy: &mut f32,
            snare_attack: &mut f32,
            snare_drive: &mut f32,
            snare_level: &mut f32,
            snare_filter_type: &mut u32,
            snare_filter_cutoff: &mut f32,
            snare_filter_resonance: &mut f32,
            snare_tone_base: &mut f32,
            snare_decay_base: &mut f32,
            snare_snappy_base: &mut f32,
            snare_attack_base: &mut f32,
            snare_drive_base: &mut f32,
            snare_level_base: &mut f32,
            snare_filter_type_base: &mut u32,
            snare_filter_cutoff_base: &mut f32,
            snare_filter_resonance_base: &mut f32,
            snare_decay_time: &mut f32,
            snare_noise_decay_time: &mut f32,
            snare_env_coeff: &mut f32,
            snare_noise_coeff: &mut f32,
            snare_attack_time: &mut f32,
            snare_attack_samples: &mut u32,
            snare_attack_step: &mut f32,
            snare_freq: &mut f32,
            snare_cutoff_hz: &mut f32,
            snare_q: &mut f32,
            snare_drive_gain: &mut f32,
        ) {
            let override_enabled =
                track.snare_step_override_enabled[step_idx].load(Ordering::Relaxed);
            if override_enabled {
                let tone =
                    f32::from_bits(track.snare_step_tone[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let decay =
                    f32::from_bits(track.snare_step_decay[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let snappy =
                    f32::from_bits(track.snare_step_snappy[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let attack =
                    f32::from_bits(track.snare_step_attack[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let drive_val =
                    f32::from_bits(track.snare_step_drive[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let level =
                    f32::from_bits(track.snare_step_level[step_idx].load(Ordering::Relaxed))
                        .clamp(0.0, 1.0);
                let filter_type = track.snare_step_filter_type[step_idx].load(Ordering::Relaxed);
                let filter_cutoff = f32::from_bits(
                    track.snare_step_filter_cutoff[step_idx].load(Ordering::Relaxed),
                )
                .clamp(0.0, 1.0);
                let filter_resonance = f32::from_bits(
                    track.snare_step_filter_resonance[step_idx].load(Ordering::Relaxed),
                )
                .clamp(0.0, 1.0);

                *snare_tone = tone;
                *snare_decay = decay;
                *snare_snappy = snappy;
                *snare_attack = attack;
                *snare_drive = drive_val;
                *snare_level = if track_muted { 0.0 } else { level };
                *snare_filter_type = filter_type;
                *snare_filter_cutoff = filter_cutoff;
                *snare_filter_resonance = filter_resonance;

                *snare_decay_time = 0.03 + *snare_decay * 0.4;
                *snare_noise_decay_time = 0.02 + *snare_decay * 0.2;
                *snare_env_coeff = (-1.0 / (*snare_decay_time * sr)).exp();
                *snare_noise_coeff = (-1.0 / (*snare_noise_decay_time * sr)).exp();
                *snare_attack_time = *snare_attack * 0.01;
                *snare_attack_samples = (*snare_attack_time * sr).round().max(0.0) as u32;
                *snare_attack_step = if *snare_attack_samples > 0 {
                    1.0 / *snare_attack_samples as f32
                } else {
                    1.0
                };
                *snare_freq = 180.0 + *snare_tone * 420.0;
                *snare_cutoff_hz = cutoff_min * cutoff_span.powf(*snare_filter_cutoff);
                *snare_q = 0.1 + *snare_filter_resonance * 0.9;
                *snare_drive_gain = 1.0 + *snare_drive * 8.0;

                if step_hold {
                    *snare_tone_base = tone;
                    *snare_decay_base = decay;
                    *snare_snappy_base = snappy;
                    *snare_attack_base = attack;
                    *snare_drive_base = drive_val;
                    *snare_level_base = level;
                    *snare_filter_type_base = filter_type;
                    *snare_filter_cutoff_base = filter_cutoff;
                    *snare_filter_resonance_base = filter_resonance;
                    track.snare_tone.store(tone.to_bits(), Ordering::Relaxed);
                    track.snare_decay.store(decay.to_bits(), Ordering::Relaxed);
                    track.snare_snappy.store(snappy.to_bits(), Ordering::Relaxed);
                    track.snare_attack.store(attack.to_bits(), Ordering::Relaxed);
                    track.snare_drive.store(drive_val.to_bits(), Ordering::Relaxed);
                    track.snare_level.store(level.to_bits(), Ordering::Relaxed);
                    track.snare_filter_type.store(filter_type, Ordering::Relaxed);
                    track
                        .snare_filter_cutoff
                        .store(filter_cutoff.to_bits(), Ordering::Relaxed);
                    track
                        .snare_filter_resonance
                        .store(filter_resonance.to_bits(), Ordering::Relaxed);
                }
            } else {
                *snare_tone = *snare_tone_base;
                *snare_decay = *snare_decay_base;
                *snare_snappy = *snare_snappy_base;
                *snare_attack = *snare_attack_base;
                *snare_drive = *snare_drive_base;
                *snare_level = if track_muted { 0.0 } else { *snare_level_base };
                *snare_filter_type = *snare_filter_type_base;
                *snare_filter_cutoff = *snare_filter_cutoff_base;
                *snare_filter_resonance = *snare_filter_resonance_base;

                *snare_decay_time = 0.03 + *snare_decay * 0.4;
                *snare_noise_decay_time = 0.02 + *snare_decay * 0.2;
                *snare_env_coeff = (-1.0 / (*snare_decay_time * sr)).exp();
                *snare_noise_coeff = (-1.0 / (*snare_noise_decay_time * sr)).exp();
                *snare_attack_time = *snare_attack * 0.01;
                *snare_attack_samples = (*snare_attack_time * sr).round().max(0.0) as u32;
                *snare_attack_step = if *snare_attack_samples > 0 {
                    1.0 / *snare_attack_samples as f32
                } else {
                    1.0
                };
                *snare_freq = 180.0 + *snare_tone * 420.0;
                *snare_cutoff_hz = cutoff_min * cutoff_span.powf(*snare_filter_cutoff);
                *snare_q = 0.1 + *snare_filter_resonance * 0.9;
                *snare_drive_gain = 1.0 + *snare_drive * 8.0;
            }
        }

        if current_step >= 0 {
            let step_idx = current_step as usize;
            if step_idx < SYNDRM_STEPS {
                apply_kick_step_params(
                    track,
                    step_idx,
                    step_hold,
                    track_muted,
                    cutoff_min,
                    cutoff_span,
                    sr,
                    &mut kick_pitch,
                    &mut kick_decay,
                    &mut kick_attack,
                    kick_pitch_env_amount,
                    &mut kick_drive,
                    &mut kick_level,
                    &mut kick_filter_type,
                    &mut kick_filter_cutoff,
                    &mut kick_filter_resonance,
                    &mut kick_pitch_base,
                    &mut kick_decay_base,
                    &mut kick_attack_base,
                    &mut kick_drive_base,
                    &mut kick_level_base,
                    &mut kick_filter_type_base,
                    &mut kick_filter_cutoff_base,
                    &mut kick_filter_resonance_base,
                    &mut decay_time,
                    &mut pitch_decay_time,
                    &mut env_coeff,
                    &mut attack_time,
                    &mut attack_samples,
                    &mut attack_step,
                    &mut pitch_coeff,
                    &mut base_freq,
                    &mut sweep,
                    &mut drive,
                    &mut kick_cutoff_hz,
                    &mut kick_q,
                );
                apply_snare_step_params(
                    track,
                    step_idx,
                    step_hold,
                    track_muted,
                    cutoff_min,
                    cutoff_span,
                    sr,
                    &mut snare_tone,
                    &mut snare_decay,
                    &mut snare_snappy,
                    &mut snare_attack,
                    &mut snare_drive,
                    &mut snare_level,
                    &mut snare_filter_type,
                    &mut snare_filter_cutoff,
                    &mut snare_filter_resonance,
                    &mut snare_tone_base,
                    &mut snare_decay_base,
                    &mut snare_snappy_base,
                    &mut snare_attack_base,
                    &mut snare_drive_base,
                    &mut snare_level_base,
                    &mut snare_filter_type_base,
                    &mut snare_filter_cutoff_base,
                    &mut snare_filter_resonance_base,
                    &mut snare_decay_time,
                    &mut snare_noise_decay_time,
                    &mut snare_env_coeff,
                    &mut snare_noise_coeff,
                    &mut snare_attack_time,
                    &mut snare_attack_samples,
                    &mut snare_attack_step,
                    &mut snare_freq,
                    &mut snare_cutoff_hz,
                    &mut snare_q,
                    &mut snare_drive_gain,
                );
            }
        }

        let output = track_output;

        for sample_idx in 0..num_buffer_samples {
            if transport_running {
                sequencer_phase += 1.0;
                if sequencer_phase >= samples_per_step {
                    sequencer_phase -= samples_per_step;
                    current_step = (current_step + 1).rem_euclid(loop_steps_i32);
                    track.kick_sequencer_step.store(current_step, Ordering::Relaxed);
                    track.snare_sequencer_step.store(current_step, Ordering::Relaxed);
                    let step_idx = current_step as usize;
                    if step_idx < SYNDRM_STEPS {
                        apply_kick_step_params(
                            track,
                            step_idx,
                            step_hold,
                            track_muted,
                            cutoff_min,
                            cutoff_span,
                            sr,
                            &mut kick_pitch,
                            &mut kick_decay,
                            &mut kick_attack,
                            kick_pitch_env_amount,
                            &mut kick_drive,
                            &mut kick_level,
                            &mut kick_filter_type,
                            &mut kick_filter_cutoff,
                            &mut kick_filter_resonance,
                            &mut kick_pitch_base,
                            &mut kick_decay_base,
                            &mut kick_attack_base,
                            &mut kick_drive_base,
                            &mut kick_level_base,
                            &mut kick_filter_type_base,
                            &mut kick_filter_cutoff_base,
                            &mut kick_filter_resonance_base,
                            &mut decay_time,
                            &mut pitch_decay_time,
                            &mut env_coeff,
                            &mut attack_time,
                            &mut attack_samples,
                            &mut attack_step,
                            &mut pitch_coeff,
                            &mut base_freq,
                            &mut sweep,
                            &mut drive,
                            &mut kick_cutoff_hz,
                            &mut kick_q,
                        );
                        apply_snare_step_params(
                            track,
                            step_idx,
                            step_hold,
                            track_muted,
                            cutoff_min,
                            cutoff_span,
                            sr,
                            &mut snare_tone,
                            &mut snare_decay,
                            &mut snare_snappy,
                            &mut snare_attack,
                            &mut snare_drive,
                            &mut snare_level,
                            &mut snare_filter_type,
                            &mut snare_filter_cutoff,
                            &mut snare_filter_resonance,
                            &mut snare_tone_base,
                            &mut snare_decay_base,
                            &mut snare_snappy_base,
                            &mut snare_attack_base,
                            &mut snare_drive_base,
                            &mut snare_level_base,
                            &mut snare_filter_type_base,
                            &mut snare_filter_cutoff_base,
                            &mut snare_filter_resonance_base,
                            &mut snare_decay_time,
                            &mut snare_noise_decay_time,
                            &mut snare_env_coeff,
                            &mut snare_noise_coeff,
                            &mut snare_attack_time,
                            &mut snare_attack_samples,
                            &mut snare_attack_step,
                            &mut snare_freq,
                            &mut snare_cutoff_hz,
                            &mut snare_q,
                            &mut snare_drive_gain,
                        );

                        if track.kick_sequencer_grid[step_idx].load(Ordering::Relaxed) {
                            pitch_env = 1.0;
                            if attack_samples > 0 {
                                attack_remaining = attack_samples;
                            } else {
                                env = 1.0;
                            }
                        }
                        if track.snare_sequencer_grid[step_idx].load(Ordering::Relaxed) {
                            if snare_attack_samples > 0 {
                                snare_attack_remaining = snare_attack_samples;
                                snare_env = 0.0;
                                snare_noise_env = 0.0;
                            } else {
                                snare_env = 1.0;
                                snare_noise_env = 1.0;
                            }
                        }
                    }
                }
            }

            if attack_remaining > 0 {
                env = (env + (1.0 - env) * attack_step).min(1.0);
                attack_remaining = attack_remaining.saturating_sub(1);
            } else {
                env *= env_coeff;
            }
            pitch_env *= pitch_coeff;
            if snare_attack_remaining > 0 {
                snare_env = (snare_env + (1.0 - snare_env) * snare_attack_step).min(1.0);
                snare_noise_env =
                    (snare_noise_env + (1.0 - snare_noise_env) * snare_attack_step).min(1.0);
                snare_attack_remaining = snare_attack_remaining.saturating_sub(1);
            } else {
                snare_env *= snare_env_coeff;
                snare_noise_env *= snare_noise_coeff;
            }

            let freq = base_freq + pitch_env * sweep;
            let mut osc_out = [0.0f32];
            dsp_state.kick_osc.tick(&[freq], &mut osc_out);
            let mut sample = osc_out[0] * env;
            if kick_filter_pre_drive {
                sample = Self::apply_syndrm_filter(
                    kick_filter_type,
                    sample,
                    kick_cutoff_hz,
                    kick_q,
                    &mut *dsp_state.kick_filter_moog,
                    &mut *dsp_state.kick_filter_lp,
                    &mut *dsp_state.kick_filter_hp,
                    &mut *dsp_state.kick_filter_bp,
                );
            }
            if kick_drive > 0.0 {
                let mut drive_out = [0.0f32];
                dsp_state.kick_drive.tick(&[sample * drive], &mut drive_out);
                sample = drive_out[0];
            }
            if !kick_filter_pre_drive {
                sample = Self::apply_syndrm_filter(
                    kick_filter_type,
                    sample,
                    kick_cutoff_hz,
                    kick_q,
                    &mut *dsp_state.kick_filter_moog,
                    &mut *dsp_state.kick_filter_lp,
                    &mut *dsp_state.kick_filter_hp,
                    &mut *dsp_state.kick_filter_bp,
                );
            }
            sample *= kick_level;

            if snare_env > 0.0 || snare_noise_env > 0.0 {
                let mut tone_out = [0.0f32];
                dsp_state.snare_osc.tick(&[snare_freq], &mut tone_out);
                let mut noise_out = [0.0f32];
                dsp_state.snare_noise.tick(&[], &mut noise_out);
                let tone_sample = tone_out[0] * snare_env;
                let noise_sample = noise_out[0] * snare_noise_env;
                let mut snare_sample =
                    tone_sample * (1.0 - snare_snappy) + noise_sample * snare_snappy;
                if snare_filter_pre_drive {
                    snare_sample = Self::apply_syndrm_filter(
                        snare_filter_type,
                        snare_sample,
                        snare_cutoff_hz,
                        snare_q,
                        &mut *dsp_state.snare_filter_moog,
                        &mut *dsp_state.snare_filter_lp,
                        &mut *dsp_state.snare_filter_hp,
                        &mut *dsp_state.snare_filter_bp,
                    );
                }
                let mut drive_out = [0.0f32];
                dsp_state
                    .snare_drive
                    .tick(&[snare_sample * snare_drive_gain], &mut drive_out);
                snare_sample = drive_out[0];
                if !snare_filter_pre_drive {
                    snare_sample = Self::apply_syndrm_filter(
                        snare_filter_type,
                        snare_sample,
                        snare_cutoff_hz,
                        snare_q,
                        &mut *dsp_state.snare_filter_moog,
                        &mut *dsp_state.snare_filter_lp,
                        &mut *dsp_state.snare_filter_hp,
                        &mut *dsp_state.snare_filter_bp,
                    );
                }
                sample += snare_sample * snare_level;
            }

            for channel in output.iter_mut() {
                channel[sample_idx] += sample;
            }
        }

        track
            .kick_sequencer_phase
            .store(sequencer_phase.round().max(0.0) as u32, Ordering::Relaxed);
        track
            .snare_sequencer_phase
            .store(sequencer_phase.round().max(0.0) as u32, Ordering::Relaxed);
        track.kick_env.store(env.to_bits(), Ordering::Relaxed);
        track
            .kick_pitch_env
            .store(pitch_env.to_bits(), Ordering::Relaxed);
        track
            .kick_attack_remaining
            .store(attack_remaining, Ordering::Relaxed);
        track.snare_env.store(snare_env.to_bits(), Ordering::Relaxed);
        track
            .snare_noise_env
            .store(snare_noise_env.to_bits(), Ordering::Relaxed);
        track
            .snare_attack_remaining
            .store(snare_attack_remaining, Ordering::Relaxed);
    }

    fn apply_syndrm_filter(
        filter_type: u32,
        sample: f32,
        cutoff_hz: f32,
        q: f32,
        moog: &mut dyn AudioUnit,
        lp: &mut dyn AudioUnit,
        hp: &mut dyn AudioUnit,
        bp: &mut dyn AudioUnit,
    ) -> f32 {
        let mut out = [0.0f32];
        match filter_type {
            0 => {
                moog.tick(&[sample, cutoff_hz, q], &mut out);
                out[0]
            }
            1 => {
                lp.tick(&[sample, cutoff_hz, q], &mut out);
                out[0]
            }
            2 => {
                hp.tick(&[sample, cutoff_hz, q], &mut out);
                out[0]
            }
            3 => {
                bp.tick(&[sample, cutoff_hz, q], &mut out);
                out[0]
            }
            _ => sample,
        }
    }

    fn process_voidseed(
        track: &Track,
        track_output: &mut [Vec<f32>],
        num_buffer_samples: usize,
        _global_tempo: &AtomicU32,
        _master_step: i32,
        _master_phase: f32,
        _samples_per_step: f32,
        sample_rate: f32,
    ) {
        let sr = sample_rate.max(1.0);
        const VOID_SEED_DB_BOOST: f32 = 1.4125375; // +3 dB
        let target_base_freq = f32::from_bits(track.void_base_freq.load(Ordering::Relaxed));
        let target_chaos_depth = f32::from_bits(track.void_chaos_depth.load(Ordering::Relaxed));
        let target_entropy = f32::from_bits(track.void_entropy.load(Ordering::Relaxed));
        let target_feedback = f32::from_bits(track.void_feedback.load(Ordering::Relaxed));
        let target_diffusion = f32::from_bits(track.void_diffusion.load(Ordering::Relaxed));
        let target_mod_rate = f32::from_bits(track.void_mod_rate.load(Ordering::Relaxed));
        let target_void_level = f32::from_bits(track.void_level.load(Ordering::Relaxed));

        let base_freq = smooth_param(
            f32::from_bits(track.void_base_freq_smooth.load(Ordering::Relaxed)),
            target_base_freq,
            num_buffer_samples,
            sr,
        );
        let chaos_depth = smooth_param(
            f32::from_bits(track.void_chaos_depth_smooth.load(Ordering::Relaxed)),
            target_chaos_depth,
            num_buffer_samples,
            sr,
        );
        let entropy = smooth_param(
            f32::from_bits(track.void_entropy_smooth.load(Ordering::Relaxed)),
            target_entropy,
            num_buffer_samples,
            sr,
        );
        let feedback = smooth_param(
            f32::from_bits(track.void_feedback_smooth.load(Ordering::Relaxed)),
            target_feedback,
            num_buffer_samples,
            sr,
        );
        let diffusion = smooth_param(
            f32::from_bits(track.void_diffusion_smooth.load(Ordering::Relaxed)),
            target_diffusion,
            num_buffer_samples,
            sr,
        );
        let mod_rate = smooth_param(
            f32::from_bits(track.void_mod_rate_smooth.load(Ordering::Relaxed)),
            target_mod_rate,
            num_buffer_samples,
            sr,
        );
        let void_level =
            smooth_param(
                f32::from_bits(track.void_level_smooth.load(Ordering::Relaxed)),
                target_void_level,
                num_buffer_samples,
                sr,
            ) * VOID_SEED_DB_BOOST;

        let mut osc_phases = [0.0f32; 12];
        let mut lfo_phases = [0.0f32; 12];
        let mut lfo_freqs = [0.0f32; 12];
        for i in 0..12 {
            osc_phases[i] = f32::from_bits(track.void_osc_phases[i].load(Ordering::Relaxed));
            lfo_phases[i] = f32::from_bits(track.void_lfo_phases[i].load(Ordering::Relaxed));
            lfo_freqs[i] = f32::from_bits(track.void_lfo_freqs[i].load(Ordering::Relaxed));
        }
        let mut chaos_phase = f32::from_bits(track.void_lfo_chaos_phase.load(Ordering::Relaxed));
        let mut filter_v1 = [
            f32::from_bits(track.void_filter_v1[0].load(Ordering::Relaxed)),
            f32::from_bits(track.void_filter_v1[1].load(Ordering::Relaxed)),
        ];
        let mut filter_v2 = [
            f32::from_bits(track.void_filter_v2[0].load(Ordering::Relaxed)),
            f32::from_bits(track.void_filter_v2[1].load(Ordering::Relaxed)),
        ];
        let mut internal_gain = f32::from_bits(track.void_internal_gain.load(Ordering::Relaxed));

        // Targeting 0.8 gain when enabled, 0.0 when disabled (ramping)
        let target_gain = if track.void_enabled.load(Ordering::Relaxed) {
            0.8 * VOID_SEED_DB_BOOST
        } else {
            0.0
        };
        let gain_step = (target_gain - internal_gain) / (4.0 * sr); // 4 second ramp

        let output = track_output;
        let num_channels = output.len();

        if let Some(mut delay_buf) = track.void_delay_buffer.try_lock() {
            let mut write_pos = track.void_delay_write_pos.load(Ordering::Relaxed) as usize;
            let delay_len = delay_buf[0].len();
            // 8n delay at 120bpm = 0.25s. 
            let delay_samples = (0.25 * sr) as usize;

            for sample_idx in 0..num_buffer_samples {
                internal_gain = (internal_gain + gain_step).clamp(0.0, target_gain);

                // Chaos LFO (affects filter frequency)
                chaos_phase += 0.02 / sr;
                if chaos_phase >= 1.0 { chaos_phase -= 1.0; }
                let chaos_lfo = (chaos_phase * 2.0 * PI).sin() * 0.5 + 0.5;
                let filter_mod = 0.2 + chaos_lfo * chaos_depth * 5.0;

                let mut swarm_sample = 0.0f32;
                let types = [0, 1, 2, 3]; // sine, sawtooth, square, triangle

                for i in 0..12 {
                    // Detune LFO
                    lfo_phases[i] += lfo_freqs[i] / sr;
                    if lfo_phases[i] >= 1.0 { lfo_phases[i] -= 1.0; }
                    let detune_cents = (lfo_phases[i] * 2.0 * PI).sin() * 20.0;
                    let detune_ratio = 2.0f32.powf(detune_cents / 1200.0);

                    // Frequency with entropy
                    let entropy_offset = ((i as f32 * 1.618).fract() - 0.5) * entropy;
                    let freq = base_freq * (i as f32 * 0.5 + 1.0) * (1.0 + entropy_offset) * detune_ratio;

                    osc_phases[i] += freq / sr;
                    if osc_phases[i] >= 1.0 { osc_phases[i] -= 1.0; }

                    let phase = osc_phases[i];
                    let val = match types[i % 4] {
                        0 => (phase * 2.0 * PI).sin(),
                        1 => phase * 2.0 - 1.0,
                        2 => if phase < 0.5 { 1.0 } else { -1.0 },
                        3 => if phase < 0.5 { phase * 4.0 - 1.0 } else { 3.0 - phase * 4.0 },
                        _ => 0.0,
                    };
                    swarm_sample += val * 0.04;
                }

                // Filter
                let cutoff_hz = (150.0 + mod_rate * 500.0 * filter_mod).clamp(20.0, 20000.0);
                let filter_f = (2.0 * (PI * cutoff_hz / sr).sin()).clamp(0.0, 0.99);
                let filter_q = 0.5;

                for ch in 0..num_channels.min(2) {
                    let low = filter_v2[ch] + filter_f * filter_v1[ch];
                    let high = swarm_sample - low - filter_q * filter_v1[ch];
                    let band = filter_f * high + filter_v1[ch];
                    
                    let filtered = low;
                    
                    filter_v1[ch] = band;
                    filter_v2[ch] = low;

                    // Delay (Diffusion & Feedback)
                    let read_pos = (write_pos + delay_len - delay_samples) % delay_len;
                    let delayed_sample = delay_buf[ch][read_pos];
                    
                    // Diffusion is "wet" in DroneSYN
                    let output_sample = filtered * (1.0 - diffusion) + delayed_sample * diffusion;
                    
                    // Write back to delay buffer with feedback
                    delay_buf[ch][write_pos] = filtered + delayed_sample * feedback;
                    
                    output[ch][sample_idx] += output_sample * internal_gain * void_level;
                }
                write_pos = (write_pos + 1) % delay_len;
            }
            track.void_delay_write_pos.store(write_pos as u32, Ordering::Relaxed);
        }

        // Store back
        for i in 0..12 {
            track.void_osc_phases[i].store(osc_phases[i].to_bits(), Ordering::Relaxed);
            track.void_lfo_phases[i].store(lfo_phases[i].to_bits(), Ordering::Relaxed);
        }
        track.void_lfo_chaos_phase.store(chaos_phase.to_bits(), Ordering::Relaxed);
        track
            .void_base_freq_smooth
            .store(base_freq.to_bits(), Ordering::Relaxed);
        track
            .void_chaos_depth_smooth
            .store(chaos_depth.to_bits(), Ordering::Relaxed);
        track
            .void_entropy_smooth
            .store(entropy.to_bits(), Ordering::Relaxed);
        track
            .void_feedback_smooth
            .store(feedback.to_bits(), Ordering::Relaxed);
        track
            .void_diffusion_smooth
            .store(diffusion.to_bits(), Ordering::Relaxed);
        track
            .void_mod_rate_smooth
            .store(mod_rate.to_bits(), Ordering::Relaxed);
        track
            .void_level_smooth
            .store((void_level / VOID_SEED_DB_BOOST).to_bits(), Ordering::Relaxed);

        track.void_filter_v1[0].store(filter_v1[0].to_bits(), Ordering::Relaxed);
        track.void_filter_v1[1].store(filter_v1[1].to_bits(), Ordering::Relaxed);
        track.void_filter_v2[0].store(filter_v2[0].to_bits(), Ordering::Relaxed);
        track.void_filter_v2[1].store(filter_v2[1].to_bits(), Ordering::Relaxed);
        track.void_internal_gain.store(internal_gain.to_bits(), Ordering::Relaxed);
    }

    fn process_track_mosaic(
        track: &Track,
        track_output: &mut [Vec<f32>],
        num_buffer_samples: usize,
        global_tempo: f32,
    ) {
        if track.granular_type.load(Ordering::Relaxed) != 1 {
            return;
        }
        if !track.mosaic_enabled.load(Ordering::Relaxed) {
            return;
        }
        let mosaic_buffer = match track.mosaic_buffer.try_lock() {
            Some(buffer) => buffer,
            None => return,
        };
        if mosaic_buffer.is_empty() || num_buffer_samples == 0 {
            return;
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
        let target_spatial =
            f32::from_bits(track.mosaic_spatial.load(Ordering::Relaxed)).clamp(0.0, 1.0);
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
        let mosaic_spatial = smooth_param(
            f32::from_bits(track.mosaic_spatial_smooth.load(Ordering::Relaxed)),
            target_spatial,
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
            .mosaic_spatial_smooth
            .store(mosaic_spatial.to_bits(), Ordering::Relaxed);
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
        let tempo = if global_tempo.is_finite() {
            global_tempo.clamp(20.0, 240.0)
        } else {
            120.0
        };
        let base_rate = if mosaic_rate <= 0.5 {
            let divisions = [
                4.0_f32,
                2.0,
                1.0,
                0.5,
                0.25,
                0.125,
                0.0625,
                0.03125,
            ];
            let t = (mosaic_rate / 0.5).clamp(0.0, 1.0);
            let idx = (t * (divisions.len().saturating_sub(1)) as f32).round() as usize;
            let beats = divisions[idx.min(divisions.len() - 1)];
            ((tempo / 60.0) / beats).clamp(MOSAIC_RATE_MIN, MOSAIC_RATE_MAX)
        } else {
            let free = ((mosaic_rate - 0.5) / 0.5).clamp(0.0, 1.0);
            MOSAIC_RATE_MIN + (MOSAIC_RATE_MAX - MOSAIC_RATE_MIN) * free
        };
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
        let mut grain_pan =
            f32::from_bits(track.mosaic_grain_pan.load(Ordering::Relaxed));
        let mut rng_state = track.mosaic_rng_state.load(Ordering::Relaxed);

        let num_channels = track_output.len();
        let output = track_output;

        for channel_idx in 0..num_channels {
            for sample_idx in 0..num_buffer_samples {
                output[channel_idx][sample_idx] *= 1.0 - mosaic_wet;
            }
        }

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
                let pan_rand = next_mosaic_rand_unit(&mut rng_state) * 2.0 - 1.0;
                grain_pan = (pan_rand * mosaic_spatial).clamp(-1.0, 1.0);

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
            let (left_gain, right_gain, other_gain) = if num_channels >= 2 {
                let pan = grain_pan.clamp(-1.0, 1.0);
                let angle = (pan + 1.0) * 0.25 * PI;
                let left = angle.cos();
                let right = angle.sin();
                (left, right, 0.5 * (left + right))
            } else {
                (1.0, 1.0, 1.0)
            };
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
                let pan_gain = if channel_idx == 0 {
                    left_gain
                } else if channel_idx == 1 {
                    right_gain
                } else {
                    other_gain
                };
                output[channel_idx][sample_idx] +=
                    sample_value * env * MOSAIC_OUTPUT_GAIN * wet * pan_gain;
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
            .mosaic_grain_pan
            .store(grain_pan.to_bits(), Ordering::Relaxed);
        track
            .mosaic_rng_state
            .store(rng_state, Ordering::Relaxed);
    }

    fn process_track_ring(
        track: &Track,
        track_output: &mut [Vec<f32>],
        num_buffer_samples: usize,
        global_tempo: f32,
    ) {
        if !track.ring_enabled.load(Ordering::Relaxed) {
            return;
        }
        if num_buffer_samples == 0 {
            return;
        }
        let sr = track.sample_rate.load(Ordering::Relaxed).max(1) as f32;
        let target_cutoff =
            f32::from_bits(track.ring_cutoff.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_resonance =
            f32::from_bits(track.ring_resonance.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_decay =
            f32::from_bits(track.ring_decay.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_pitch =
            f32::from_bits(track.ring_pitch.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_tone =
            f32::from_bits(track.ring_tone.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_tilt =
            f32::from_bits(track.ring_tilt.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_slope =
            f32::from_bits(track.ring_slope.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_wet =
            f32::from_bits(track.ring_wet.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_detune =
            f32::from_bits(track.ring_detune.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_waves =
            f32::from_bits(track.ring_waves.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_waves_rate =
            f32::from_bits(track.ring_waves_rate.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_waves_mode =
            track.ring_waves_rate_mode.load(Ordering::Relaxed);
        let target_noise =
            f32::from_bits(track.ring_noise.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_noise_rate =
            f32::from_bits(track.ring_noise_rate.load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let target_noise_mode =
            track.ring_noise_rate_mode.load(Ordering::Relaxed);
        let ring_scale = track.ring_scale.load(Ordering::Relaxed);

        let ring_cutoff = smooth_param(
            f32::from_bits(track.ring_cutoff_smooth.load(Ordering::Relaxed)),
            target_cutoff,
            num_buffer_samples,
            sr,
        );
        let ring_resonance = smooth_param(
            f32::from_bits(track.ring_resonance_smooth.load(Ordering::Relaxed)),
            target_resonance,
            num_buffer_samples,
            sr,
        );
        let ring_decay = smooth_param(
            f32::from_bits(track.ring_decay_smooth.load(Ordering::Relaxed)),
            target_decay,
            num_buffer_samples,
            sr,
        );
        let ring_pitch = smooth_param(
            f32::from_bits(track.ring_pitch_smooth.load(Ordering::Relaxed)),
            target_pitch,
            num_buffer_samples,
            sr,
        );
        let ring_tone = smooth_param(
            f32::from_bits(track.ring_tone_smooth.load(Ordering::Relaxed)),
            target_tone,
            num_buffer_samples,
            sr,
        );
        let ring_tilt = smooth_param(
            f32::from_bits(track.ring_tilt_smooth.load(Ordering::Relaxed)),
            target_tilt,
            num_buffer_samples,
            sr,
        );
        let ring_slope = smooth_param(
            f32::from_bits(track.ring_slope_smooth.load(Ordering::Relaxed)),
            target_slope,
            num_buffer_samples,
            sr,
        );
        let ring_wet = smooth_param(
            f32::from_bits(track.ring_wet_smooth.load(Ordering::Relaxed)),
            target_wet,
            num_buffer_samples,
            sr,
        )
        .clamp(0.0, 1.0);
        let ring_detune = smooth_param(
            f32::from_bits(track.ring_detune_smooth.load(Ordering::Relaxed)),
            target_detune,
            num_buffer_samples,
            sr,
        );
        let ring_waves = smooth_param(
            f32::from_bits(track.ring_waves_smooth.load(Ordering::Relaxed)),
            target_waves,
            num_buffer_samples,
            sr,
        );
        let ring_waves_rate = smooth_param(
            f32::from_bits(track.ring_waves_rate_smooth.load(Ordering::Relaxed)),
            target_waves_rate,
            num_buffer_samples,
            sr,
        );
        let ring_noise = smooth_param(
            f32::from_bits(track.ring_noise_smooth.load(Ordering::Relaxed)),
            target_noise,
            num_buffer_samples,
            sr,
        );
        let ring_noise_rate = smooth_param(
            f32::from_bits(track.ring_noise_rate_smooth.load(Ordering::Relaxed)),
            target_noise_rate,
            num_buffer_samples,
            sr,
        );

        track
            .ring_cutoff_smooth
            .store(ring_cutoff.to_bits(), Ordering::Relaxed);
        track
            .ring_resonance_smooth
            .store(ring_resonance.to_bits(), Ordering::Relaxed);
        track
            .ring_decay_smooth
            .store(ring_decay.to_bits(), Ordering::Relaxed);
        track
            .ring_pitch_smooth
            .store(ring_pitch.to_bits(), Ordering::Relaxed);
        track
            .ring_tone_smooth
            .store(ring_tone.to_bits(), Ordering::Relaxed);
        track
            .ring_tilt_smooth
            .store(ring_tilt.to_bits(), Ordering::Relaxed);
        track
            .ring_slope_smooth
            .store(ring_slope.to_bits(), Ordering::Relaxed);
        track
            .ring_wet_smooth
            .store(ring_wet.to_bits(), Ordering::Relaxed);
        track
            .ring_detune_smooth
            .store(ring_detune.to_bits(), Ordering::Relaxed);
        track
            .ring_waves_smooth
            .store(ring_waves.to_bits(), Ordering::Relaxed);
        track
            .ring_waves_rate_smooth
            .store(ring_waves_rate.to_bits(), Ordering::Relaxed);
        track
            .ring_noise_smooth
            .store(ring_noise.to_bits(), Ordering::Relaxed);
        track
            .ring_noise_rate_smooth
            .store(ring_noise_rate.to_bits(), Ordering::Relaxed);

        if ring_wet > 0.0 {
            let decay_mode = track.ring_decay_mode.load(Ordering::Relaxed);
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
            let cutoff_hz = ring_quantize_freq(cutoff_hz, ring_scale);
            let q = 0.5 + ring_resonance * 12.0;
            let r = 1.0 / (2.0 * q.max(0.001));
            let slope = ring_slope.clamp(0.0, 1.0);

            let num_channels = track_output.len();
            let output = track_output;
            let detune_phase =
                f32::from_bits(track.ring_detune_phase.load(Ordering::Relaxed));
            let waves_mode = target_waves_mode;
            let noise_mode = target_noise_mode;
            let waves_rate_hz = ring_rate_hz(ring_waves_rate, waves_mode, global_tempo);
            let noise_rate_hz = ring_rate_hz(ring_noise_rate, noise_mode, global_tempo);
            let waves_step = (waves_rate_hz / sr).min(1.0);
            let noise_step = (noise_rate_hz / sr).min(1.0);
            let mut waves_phase =
                f32::from_bits(track.ring_waves_phase.load(Ordering::Relaxed));
            let mut noise_phase =
                f32::from_bits(track.ring_noise_phase.load(Ordering::Relaxed));
            let mut noise_value =
                f32::from_bits(track.ring_noise_value.load(Ordering::Relaxed));
            let mut noise_rng = track.ring_noise_rng.load(Ordering::Relaxed);
            let mut phase = detune_phase;
            let detune_rate = RING_DETUNE_RATE_HZ / sr;
            let detune_depth = ring_detune.clamp(0.0, 1.0) * RING_DETUNE_CENTS;
            for channel_idx in 0..num_channels {
                let mut low =
                    f32::from_bits(track.ring_low[channel_idx].load(Ordering::Relaxed));
                let mut band =
                    f32::from_bits(track.ring_band[channel_idx].load(Ordering::Relaxed));
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
                    let waves_lfo = (waves_phase * 2.0 * PI).sin();
                    let waves_mod =
                        (1.0 - ring_waves) + ring_waves * (0.5 + 0.5 * waves_lfo);
                    let noise_mod =
                        (1.0 - ring_noise) + ring_noise * (0.5 + 0.5 * noise_value);
                    output[channel_idx][sample_idx] =
                        input * (1.0 - ring_wet)
                            + tone_mix * ring_wet * waves_mod * noise_mod;
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
                        waves_phase += waves_step;
                        if waves_phase >= 1.0 {
                            waves_phase -= 1.0;
                        }
                        noise_phase += noise_step;
                        if noise_phase >= 1.0 {
                            noise_phase -= 1.0;
                            noise_rng = noise_rng
                                .wrapping_mul(1664525)
                                .wrapping_add(1013904223);
                            let noise = (noise_rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
                            if noise.is_finite() {
                                noise_value = noise;
                            } else {
                                noise_value = 0.0;
                            }
                        }
                    }
                }
                track.ring_low[channel_idx]
                    .store(low.to_bits(), Ordering::Relaxed);
                track.ring_band[channel_idx]
                    .store(band.to_bits(), Ordering::Relaxed);
            }
            track
                .ring_detune_phase
                .store(phase.to_bits(), Ordering::Relaxed);
            track
                .ring_waves_phase
                .store(waves_phase.to_bits(), Ordering::Relaxed);
            track
                .ring_noise_phase
                .store(noise_phase.to_bits(), Ordering::Relaxed);
            track
                .ring_noise_value
                .store(noise_value.to_bits(), Ordering::Relaxed);
            track
                .ring_noise_rng
                .store(noise_rng, Ordering::Relaxed);
        }
    }
}

impl Plugin for TLBX1 {
    const NAME: &'static str = "TLBX-1";
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
    type BackgroundTask = TLBX1Task;

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate.store(buffer_config.sample_rate as u32, Ordering::Relaxed);
        self.track_buffer = vec![vec![0.0; buffer_config.max_buffer_size as usize]; 2];
        true
    }

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
            pending_project_params: self.pending_project_params.clone(),
            animate_library: self.animate_library.clone(),
        }))
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let tracks = self.tracks.clone();
        let global_tempo = self.global_tempo.clone();
        let params = self.params.clone();
        let pending_project_params = self.pending_project_params.clone();
        Box::new(move |task| match task {
            TLBX1Task::LoadSample(track_idx, path) => {
                if track_idx >= NUM_TRACKS {
                    return;
                }
                
                match load_media_file(&path) {
                    Ok((new_samples, sample_rate, video)) => {
                        let mut samples = tracks[track_idx].samples.lock();
                        let mut summary = tracks[track_idx].waveform_summary.lock();
                        let mut sample_path = tracks[track_idx].sample_path.lock();
                        let mut video_cache = tracks[track_idx].video_cache.lock();

                        *samples = new_samples;
                        *sample_path = Some(path.clone());
                        tracks[track_idx]
                            .sample_rate
                            .store(sample_rate, Ordering::Relaxed);

                        if let Some(video) = video {
                            tracks[track_idx]
                                .video_enabled
                                .store(true, Ordering::Relaxed);
                            tracks[track_idx]
                                .video_width
                                .store(video.width, Ordering::Relaxed);
                            tracks[track_idx]
                                .video_height
                                .store(video.height, Ordering::Relaxed);
                            tracks[track_idx]
                                .video_fps
                                .store(video.fps.to_bits(), Ordering::Relaxed);
                            *video_cache = Some(video);
                            tracks[track_idx]
                                .video_cache_id
                                .fetch_add(1, Ordering::Relaxed);
                        } else {
                            tracks[track_idx]
                                .video_enabled
                                .store(false, Ordering::Relaxed);
                            tracks[track_idx]
                                .video_width
                                .store(0, Ordering::Relaxed);
                            tracks[track_idx]
                                .video_height
                                .store(0, Ordering::Relaxed);
                            tracks[track_idx]
                                .video_fps
                                .store(0.0f32.to_bits(), Ordering::Relaxed);
                            *video_cache = None;
                            tracks[track_idx]
                                .video_cache_id
                                .fetch_add(1, Ordering::Relaxed);
                        }

                        if !samples.is_empty() {
                            calculate_waveform_summary(&samples[0], &mut summary);
                        } else {
                            summary.fill(0.0);
                        }

                        nih_log!("Loaded media: {:?}", path);
                    }
                    Err(e) => {
                        tracks[track_idx]
                            .video_enabled
                            .store(false, Ordering::Relaxed);
                        tracks[track_idx]
                            .video_cache_id
                            .fetch_add(1, Ordering::Relaxed);
                        *tracks[track_idx].video_cache.lock() = None;
                        nih_log!("Failed to load media: {:?}", e);
                    }
                }
            }
            TLBX1Task::SaveProject {
                path,
                title,
                description,
            } => {
                let tempo = f32::from_bits(global_tempo.load(Ordering::Relaxed));
                if let Err(err) = save_project(&tracks, tempo, &params, &title, &description, &path) {
                    nih_log!("Failed to save project: {:?}", err);
                } else {
                    nih_log!("Saved project: {:?}", path);
                }
            }
            TLBX1Task::LoadProject(path) => {
                if let Err(err) = load_project(
                    &tracks,
                    &global_tempo,
                    &params,
                    &pending_project_params,
                    &path,
                ) {
                    nih_log!("Failed to load project: {:?}", err);
                } else {
                    nih_log!("Loaded project: {:?}", path);
                }
            }
            TLBX1Task::ExportProjectZip {
                path,
                title,
                description,
            } => {
                let tempo = f32::from_bits(global_tempo.load(Ordering::Relaxed));
                if let Err(err) =
                    export_project_as_zip(&tracks, tempo, &params, &title, &description, &path)
                {
                    nih_log!("Failed to export project: {:?}", err);
                } else {
                    nih_log!("Exported project zip: {:?}", path);
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
        if !global_tempo.is_finite() {
            global_tempo = 120.0;
            self.global_tempo
                .store(global_tempo.to_bits(), Ordering::Relaxed);
        }
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
        let mut master_sr = context.transport().sample_rate;
        if !master_sr.is_finite() || master_sr <= 0.0 {
            master_sr = self.sample_rate.load(Ordering::Relaxed).max(1) as f32;
        }
        self.sample_rate.store(master_sr as u32, Ordering::Relaxed);
        let mut samples_per_step = (master_sr * 60.0) / (global_tempo * 4.0);
        if !samples_per_step.is_finite() || samples_per_step <= 0.0 {
            samples_per_step = (master_sr * 60.0) / (120.0 * 4.0);
        }
        let mut master_step = self.master_step_index;
        let mut master_phase = self.master_step_phase;
        let mut master_step_count = self.master_step_count;
        if samples_per_step > 0.0 {
            while master_phase >= samples_per_step {
                master_phase -= samples_per_step;
                master_step = (master_step + 1).rem_euclid(16);
                master_step_count += 1;
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
        let transport_running = any_playing;
        for (track, syndrm_dsp) in self
            .tracks
            .iter()
            .zip(self.syndrm_dsp.iter_mut())
        {
            if track.is_recording.load(Ordering::Relaxed) {
                continue;
            }

            let engine_type = track.engine_type.load(Ordering::Relaxed);
            let track_muted = track.is_muted.load(Ordering::Relaxed);
            let should_process =
                transport_running || matches!(engine_type, 2 | 3 | 4);
            if !should_process {
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
                continue;
            }
            if !transport_running && track_muted {
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
                continue;
            }

            keep_alive = true;

            // Clear track buffer
            for channel in self.track_buffer.iter_mut() {
                channel.fill(0.0);
            }

            let mut track_peak_left = 0.0f32;
            let mut track_peak_right = 0.0f32;

            if engine_type == 2 {
                Self::process_animate(
                    track,
                    &mut self.track_buffer,
                    buffer.samples(),
                    &self.global_tempo,
                    &self.animate_library,
                    master_step,
                    master_phase,
                    samples_per_step,
                    transport_running,
                );
            } else if engine_type == 3 {
                Self::process_syndrm(
                    track,
                    &mut self.track_buffer,
                    syndrm_dsp,
                    buffer.samples(),
                    &self.global_tempo,
                    master_step,
                    master_phase,
                    master_step_count,
                    samples_per_step,
                    master_sr,
                    transport_running,
                );
            } else if engine_type == 4 {
                Self::process_voidseed(
                    track,
                    &mut self.track_buffer,
                    buffer.samples(),
                    &self.global_tempo,
                    master_step,
                    master_phase,
                    samples_per_step,
                    master_sr,
                );
            } else if transport_running {
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
                    let track_level =
                        f32::from_bits(track.level.load(Ordering::Relaxed));
                    let tape_speed =
                        f32::from_bits(track.tape_speed.load(Ordering::Relaxed)).clamp(-4.0, 4.0);
                    let tape_tempo = global_tempo.max(1.0);
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
                    let reverse_active = tape_reverse || loop_mode == 3;
                    let rotate_offset = (rotate_norm * num_samples as f32) as usize;
                    let loop_start = if reverse_active {
                        let base_end =
                            ((1.0 - loop_start_norm) * num_samples as f32) as usize;
                        let loop_end = (base_end + rotate_offset).min(num_samples);
                        let mut loop_len = (loop_length_norm * num_samples as f32) as usize;
                        if loop_len == 0 {
                            loop_len = loop_end.max(1);
                        }
                        loop_end.saturating_sub(loop_len)
                    } else {
                        let base_start = (loop_start_norm * num_samples as f32) as usize;
                        (base_start + rotate_offset) % num_samples.max(1)
                    };
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
                    if tape_rate_mode == 1 && tape_speed < 0.0 {
                        direction *= -1;
                    }
                    let rate_factor = match tape_rate_mode {
                        1 => 1.0,
                        2 => 1.5,
                        3 => 2.0 / 3.0,
                        _ => 0.0,
                    };
                    let (tempo_speed, straight_bars) = match tape_rate_mode {
                        0 => (tape_speed, None),
                        1 => {
                            let divisions = [
                                1.0 / 64.0,
                                1.0 / 32.0,
                                1.0 / 16.0,
                                1.0 / 8.0,
                                1.0 / 4.0,
                                1.0 / 2.0,
                                1.0,
                                2.0,
                                4.0,
                                8.0,
                                16.0,
                            ];
                            let normalized = (tape_speed.abs() / 4.0).clamp(0.0, 1.0);
                            let idx = ((divisions.len() - 1) as f32 * normalized).round() as usize;
                            let bars = divisions[idx];
                            let seconds_per_bar = (60.0 / tape_tempo) * 4.0;
                            let target_seconds = (bars * seconds_per_bar).max(0.001);
                            let speed = loop_len as f32
                                / (target_seconds
                                    * track.sample_rate.load(Ordering::Relaxed).max(1) as f32);
                            (speed, Some(bars))
                        }
                        _ => ((tape_tempo / 120.0) * rate_factor, None),
                    };
                    let sync_requested =
                        track.tape_sync_requested.swap(false, Ordering::Relaxed);
                    let use_straight_lock = tape_rate_mode == 1
                        && straight_bars.is_some()
                        && samples_per_step > 0.0;
                    let mut straight_phase = master_phase;
                    let mut straight_step_count = master_step_count;
                    if sync_requested && use_straight_lock && loop_len > 0 {
                        let bars = straight_bars.unwrap_or(1.0f32).max(0.000_01f32);
                        let step_progress =
                            straight_step_count as f32 + (straight_phase / samples_per_step);
                        let total_bars = step_progress / 16.0;
                        let loop_units = total_bars / bars;
                        let phase = loop_units.fract();
                        let locked_pos = if direction >= 0 {
                            loop_start as f32 + phase * loop_len as f32
                        } else {
                            loop_end.saturating_sub(1) as f32 - phase * loop_len as f32
                        };
                        play_pos = locked_pos;
                    }
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

                    let mut prev_play_pos;
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

                        if use_straight_lock {
                            let bars = straight_bars.unwrap_or(1.0f32).max(0.000_01f32);
                            let step_progress =
                                straight_step_count as f32 + (straight_phase / samples_per_step);
                            let total_bars = step_progress / 16.0;
                            let loop_units = total_bars / bars;
                            let phase = loop_units.fract();
                            let locked_pos = if direction >= 0 {
                                loop_start as f32 + phase * loop_len as f32
                            } else {
                                loop_end.saturating_sub(1) as f32 - phase * loop_len as f32
                            };
                            let xfade_start = loop_end.saturating_sub(xfade_samples) as f32;
                            let xfade_len = xfade_samples as f32;
                            for channel_idx in 0..output.len() {
                                let src_channel = if num_channels == 1 {
                                    0
                                } else if channel_idx < num_channels {
                                    channel_idx
                                } else {
                                    continue;
                                };
                                let mut sample_value = sample_at_linear(
                                    &samples,
                                    src_channel,
                                    locked_pos,
                                    loop_start,
                                    loop_end,
                                    loop_active,
                                    num_samples,
                                );
                                if loop_active && xfade_samples > 0 {
                                    if direction > 0 && locked_pos >= xfade_start {
                                        let fade_in =
                                            ((locked_pos - xfade_start) / xfade_len).clamp(0.0, 1.0);
                                        let head_pos =
                                            loop_start as f32 + (locked_pos - xfade_start);
                                        let head_sample = sample_at_linear(
                                            &samples,
                                            src_channel,
                                            head_pos,
                                            loop_start,
                                            loop_end,
                                            loop_active,
                                            num_samples,
                                        );
                                        sample_value = sample_value * (1.0 - fade_in)
                                            + head_sample * fade_in;
                                    } else if direction < 0
                                        && locked_pos <= loop_start as f32 + xfade_len
                                    {
                                        let fade_in = ((loop_start as f32 + xfade_len - locked_pos)
                                            / xfade_len)
                                            .clamp(0.0, 1.0);
                                        let head_pos =
                                            loop_end as f32 - (loop_start as f32 + xfade_len - locked_pos);
                                        let head_sample = sample_at_linear(
                                            &samples,
                                            src_channel,
                                            head_pos,
                                            loop_start,
                                            loop_end,
                                            loop_active,
                                            num_samples,
                                        );
                                        sample_value = sample_value * (1.0 - fade_in)
                                            + head_sample * fade_in;
                                    }
                                }
                                    let level = smooth_level + level_step * sample_idx as f32;
                                    let out_value = sample_value * level;
                                    self.track_buffer[channel_idx][sample_idx] += out_value;
                                    if let Some(mosaic) = mosaic_buffer.as_mut() {
                                        if mosaic_len > 0 && channel_idx < mosaic.len() {
                                            let existing = mosaic[channel_idx][mosaic_write_pos];
                                            mosaic[channel_idx][mosaic_write_pos] =
                                                out_value * (1.0 - smooth_mosaic_sos)
                                                    + existing * smooth_mosaic_sos;
                                        }
                                    }
                                }
                            if mosaic_buffer.is_some() && mosaic_len > 0 {
                                mosaic_write_pos = (mosaic_write_pos + 1) % mosaic_len;
                            }
                            play_pos = locked_pos;
                            straight_phase += 1.0;
                            if straight_phase >= samples_per_step {
                                straight_phase -= samples_per_step;
                                straight_step_count += 1;
                            }
                        } else if keylock_enabled {
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
                                    self.track_buffer[channel_idx][sample_idx] += out_value;
                                    if let Some(mosaic) = mosaic_buffer.as_mut() {
                                        if mosaic_len > 0 && channel_idx < mosaic.len() {
                                            let existing = mosaic[channel_idx][mosaic_write_pos];
                                            mosaic[channel_idx][mosaic_write_pos] =
                                                out_value * (1.0 - smooth_mosaic_sos)
                                                    + existing * smooth_mosaic_sos;
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
                            prev_play_pos = play_pos;
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
                                    if direction > 0 && play_pos >= loop_end as f32 && prev_play_pos < loop_end as f32 {
                                        direction = -1;
                                        play_pos = loop_end.saturating_sub(1) as f32;
                                    } else if direction < 0 && play_pos <= loop_start as f32 && prev_play_pos > loop_start as f32 {
                                        direction = 1;
                                        play_pos = loop_start as f32;
                                    }
                                } else if direction > 0 && play_pos >= loop_end as f32 && prev_play_pos < loop_end as f32 {
                                    play_pos = loop_start as f32;
                                } else if direction < 0 && play_pos < loop_start as f32 && prev_play_pos >= loop_start as f32 {
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
                            self.track_buffer[channel_idx][sample_idx] += out_value;
                            if let Some(mosaic) = mosaic_buffer.as_mut() {
                                if mosaic_len > 0 && channel_idx < mosaic.len() {
                                    let existing = mosaic[channel_idx][mosaic_write_pos];
                                    mosaic[channel_idx][mosaic_write_pos] =
                                        out_value * (1.0 - smooth_mosaic_sos)
                                            + existing * smooth_mosaic_sos;
                                }
                            }
                        }

                        if mosaic_buffer.is_some() && mosaic_len > 0 {
                            mosaic_write_pos = (mosaic_write_pos + 1) % mosaic_len;
                        }

                            let speed = smooth_speed + speed_step * sample_idx as f32;
                            prev_play_pos = play_pos;
                            play_pos += direction as f32 * speed;
                            if loop_active && loop_end > loop_start {
                                if loop_mode == 1 {
                                    if direction > 0 && play_pos >= loop_end as f32 && prev_play_pos < loop_end as f32 {
                                        direction = -1;
                                        play_pos = loop_end.saturating_sub(1) as f32;
                                    } else if direction < 0 && play_pos <= loop_start as f32 && prev_play_pos > loop_start as f32 {
                                        direction = 1;
                                        play_pos = loop_start as f32;
                                    }
                                } else if direction > 0 && play_pos >= loop_end as f32 && prev_play_pos < loop_end as f32 {
                                    play_pos = loop_start as f32;
                                } else if direction < 0 && play_pos < loop_start as f32 && prev_play_pos >= loop_start as f32 {
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
                }
            }

            // Apply track effects
            Self::process_track_mosaic(
                track,
                &mut self.track_buffer,
                buffer.samples(),
                global_tempo,
            );
            Self::process_track_ring(track, &mut self.track_buffer, buffer.samples(), global_tempo);

            let mix_gain = if track_muted && engine_type != 1 { 0.0 } else { 1.0 };
            // Sum track buffer to master output and calculate final peaks
            let num_buffer_samples = buffer.samples();
            let output = buffer.as_slice();
            for sample_idx in 0..num_buffer_samples {
                for channel_idx in 0..output.len() {
                    let val = self.track_buffer[channel_idx][sample_idx] * mix_gain;
                    output[channel_idx][sample_idx] += val;

                    if channel_idx == 0 {
                        track_peak_left = track_peak_left.max(val.abs());
                    } else if channel_idx == 1 {
                        track_peak_right = track_peak_right.max(val.abs());
                    }
                }
            }

            // Update meters with final peaks
            if output.len() == 1 && buffer.channels() > 1 {
                track_peak_right = track_peak_left;
            }
            let prev_left = f32::from_bits(track.meter_left.load(Ordering::Relaxed));
            let prev_right = f32::from_bits(track.meter_right.load(Ordering::Relaxed));
            let next_left = smooth_meter(prev_left, track_peak_left);
            let next_right = smooth_meter(prev_right, track_peak_right);
            track.meter_left.store(next_left.to_bits(), Ordering::Relaxed);
            track.meter_right.store(next_right.to_bits(), Ordering::Relaxed);
        }

        if any_monitoring {
            keep_alive = true;
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

        // Master FX Chain
        let sr = self.sample_rate.load(Ordering::Relaxed) as f32;
        let num_channels = buffer.channels();
        let num_samples = buffer.samples();

        for sample_idx in 0..num_samples {
            let master_filter = self.params.master_filter.smoothed.next();
            let master_comp = self.params.master_comp.smoothed.next();

            // Calculate DJ Filter coefficients
            let mut filter_type = 0; // 0=None, 1=HP, 2=LP
            let mut f = 0.0f32;
            let q = 0.707f32;

            if master_filter < 0.49 {
                filter_type = 1; // HP
                let cutoff_hz = 20.0 + (1.0 - master_filter / 0.5) * 2000.0;
                f = (PI * cutoff_hz / sr).tan();
            } else if master_filter > 0.51 {
                filter_type = 2; // LP
                let cutoff_hz = 20000.0 - ((master_filter - 0.5) / 0.5) * 19000.0;
                f = (PI * cutoff_hz / sr).tan();
            }

            let res_coeff = 1.0 / q;

            // Compressor parameters
            let threshold_db = -24.0 * master_comp;
            let threshold = util::db_to_gain(threshold_db);
            let ratio = 1.0 + master_comp * 10.0;
            let attack_ms = 5.0;
            let release_ms = 100.0;
            let attack_coeff = (-1.0 / (attack_ms * sr / 1000.0)).exp();
            let release_coeff = (-1.0 / (release_ms * sr / 1000.0)).exp();

            let mut max_abs = 0.0f32;
            
            // Process Filter + Find Max for Compressor
            for channel_idx in 0..num_channels {
                let mut x = buffer.as_slice()[channel_idx][sample_idx];
                
                if filter_type > 0 {
                    let low = self.master_fx.filter_low[channel_idx];
                    let band = self.master_fx.filter_band[channel_idx];
                    let high = x - low - res_coeff * band;
                    let new_band = f * high + band;
                    let new_low = f * new_band + low;
                    
                    self.master_fx.filter_low[channel_idx] = new_low;
                    self.master_fx.filter_band[channel_idx] = new_band;
                    
                    if filter_type == 1 {
                        x = high;
                    } else {
                        x = new_low;
                    }
                }
                
                buffer.as_slice()[channel_idx][sample_idx] = x;
                max_abs = max_abs.max(x.abs());
            }

            // Compressor
            let mut env = self.master_fx.comp_env;
            if max_abs > env {
                env = attack_coeff * env + (1.0 - attack_coeff) * max_abs;
            } else {
                env = release_coeff * env + (1.0 - release_coeff) * max_abs;
            }
            self.master_fx.comp_env = env;

            let mut reduction = 1.0f32;
            if env > threshold {
                let env_db = util::gain_to_db(env);
                let over_db = env_db - threshold_db;
                let reduced_db = over_db / ratio;
                reduction = util::db_to_gain(reduced_db - over_db);
            }

            // Apply global gain + compression
            let gain = self.params.gain.smoothed.next();
            for channel_idx in 0..num_channels {
                buffer.as_slice()[channel_idx][sample_idx] *= gain * reduction;
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
                            let mag = mag.clamp(0.0, 1.0);
                            // Compress dynamic range to make low-level movement more visible.
                            let mag = (1.0_f32 + 20.0 * mag).ln() / (1.0_f32 + 20.0).ln();
                            spectrum[bin] = mag.clamp(0.0, 1.0);
                        }
                        for bin in bins..SPECTRUM_BINS {
                            spectrum[bin] = 0.0;
                        }
                    }
                }
            }
        }

        if any_playing || any_pending {
            let mut phase = master_phase + buffer.samples() as f32;
            let mut step = master_step;
            if samples_per_step > 0.0 {
                while phase >= samples_per_step {
                    phase -= samples_per_step;
                    step = (step + 1).rem_euclid(16);
                    master_step_count += 1;
                }
            }
            self.master_step_phase = phase;
            self.master_step_index = step;
            self.master_step_count = master_step_count;
        } else {
            self.master_step_phase = 0.0;
            self.master_step_index = 0;
            self.master_step_count = 0;
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
    if pos < 0.0 {
        if loop_active && loop_end > loop_start {
            pos = (loop_end.saturating_sub(1)) as f32;
        } else {
            pos = 0.0;
        }
    } else if pos as usize >= num_samples {
        if loop_active && loop_end > loop_start {
            pos = loop_start as f32;
        } else {
            pos = (num_samples.saturating_sub(1)) as f32;
        }
    }
    pos
}

fn lfo_division_beats(index: u32) -> f32 {
    match index {
        0 => 0.25,        // 1/16
        1 => 0.5,         // 1/8
        2 => 1.0,         // 1/4
        3 => 1.333_333_4, // 1/3
        4 => 2.0,         // 1/2
        5 => 4.0,         // 1 bar
        6 => 8.0,         // 2 bars
        7 => 16.0,        // 4 bars
        _ => 4.0,
    }
}

fn lfo_waveform_value(waveform: u32, phase: f32, sample_hold: f32) -> f32 {
    match waveform {
        0 => (2.0 * PI * phase).sin(),
        1 => 1.0 - 4.0 * (phase - 0.5).abs(),
        2 => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        3 => 2.0 * phase - 1.0,
        4 => sample_hold,
        _ => 0.0,
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

fn syndrm_rand_bool(state: &mut u32) -> bool {
    next_mosaic_rand_unit(state) >= 0.5
}

fn syndrm_rand_unit(state: &mut u32) -> f32 {
    next_mosaic_rand_unit(state).clamp(0.0, 1.0)
}

fn syndrm_rand_filter_type(state: &mut u32) -> u32 {
    let max = SYNDRM_FILTER_TYPES.max(1);
    let idx = (syndrm_rand_unit(state) * max as f32).floor() as u32;
    idx.min(max - 1)
}

fn syndrm_randomize_steps(
    track: &Track,
    rng_state: &mut u32,
    lanes: u8,
    start: usize,
    len: usize,
) {
    let end = (start + len).min(SYNDRM_STEPS);
    for step in start..end {
        if (lanes & 0b01) != 0 {
            track.kick_sequencer_grid[step].store(syndrm_rand_bool(rng_state), Ordering::Relaxed);
        }
        if (lanes & 0b10) != 0 {
            track.snare_sequencer_grid[step].store(syndrm_rand_bool(rng_state), Ordering::Relaxed);
        }
    }
}

fn syndrm_randomize_params(
    track: &Track,
    rng_state: &mut u32,
    lanes: u8,
    start: usize,
    len: usize,
) {
    let end = (start + len).min(SYNDRM_STEPS);
    for step in start..end {
        if (lanes & 0b01) != 0 {
            track.kick_step_override_enabled[step].store(true, Ordering::Relaxed);
            track.kick_step_pitch[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.kick_step_decay[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.kick_step_attack[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.kick_step_drive[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.kick_step_level[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.kick_step_filter_type[step]
                .store(syndrm_rand_filter_type(rng_state), Ordering::Relaxed);
            track.kick_step_filter_cutoff[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.kick_step_filter_resonance[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
        }
        if (lanes & 0b10) != 0 {
            track.snare_step_override_enabled[step].store(true, Ordering::Relaxed);
            track.snare_step_tone[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.snare_step_decay[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.snare_step_snappy[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.snare_step_attack[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.snare_step_drive[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.snare_step_level[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.snare_step_filter_type[step]
                .store(syndrm_rand_filter_type(rng_state), Ordering::Relaxed);
            track.snare_step_filter_cutoff[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
            track.snare_step_filter_resonance[step]
                .store(syndrm_rand_unit(rng_state).to_bits(), Ordering::Relaxed);
        }
    }
}

fn syndrm_randomize_apply(
    track: &Track,
    lanes: u8,
    start: usize,
    len: usize,
    randomize_steps: bool,
    randomize_params: bool,
) {
    let mut rng_state = track.syndrm_rng_state.load(Ordering::Relaxed);
    if randomize_steps {
        syndrm_randomize_steps(track, &mut rng_state, lanes, start, len);
    }
    if randomize_params {
        syndrm_randomize_params(track, &mut rng_state, lanes, start, len);
    }
    track.syndrm_rng_state.store(rng_state, Ordering::Relaxed);
}

fn syndrm_clear_steps(track: &Track, lanes: u8, start: usize, len: usize) {
    let end = (start + len).min(SYNDRM_STEPS);
    for step in start..end {
        if (lanes & 0b01) != 0 {
            track.kick_sequencer_grid[step].store(false, Ordering::Relaxed);
        }
        if (lanes & 0b10) != 0 {
            track.snare_sequencer_grid[step].store(false, Ordering::Relaxed);
        }
    }
}

fn syndrm_clear_params(track: &Track, lanes: u8, start: usize, len: usize) {
    let end = (start + len).min(SYNDRM_STEPS);
    let kick_pitch = f32::from_bits(track.kick_pitch.load(Ordering::Relaxed));
    let kick_decay = f32::from_bits(track.kick_decay.load(Ordering::Relaxed));
    let kick_attack = f32::from_bits(track.kick_attack.load(Ordering::Relaxed));
    let kick_drive = f32::from_bits(track.kick_drive.load(Ordering::Relaxed));
    let kick_level = f32::from_bits(track.kick_level.load(Ordering::Relaxed));
    let kick_filter_type = track.kick_filter_type.load(Ordering::Relaxed);
    let kick_filter_cutoff = f32::from_bits(track.kick_filter_cutoff.load(Ordering::Relaxed));
    let kick_filter_resonance =
        f32::from_bits(track.kick_filter_resonance.load(Ordering::Relaxed));

    let snare_tone = f32::from_bits(track.snare_tone.load(Ordering::Relaxed));
    let snare_decay = f32::from_bits(track.snare_decay.load(Ordering::Relaxed));
    let snare_snappy = f32::from_bits(track.snare_snappy.load(Ordering::Relaxed));
    let snare_attack = f32::from_bits(track.snare_attack.load(Ordering::Relaxed));
    let snare_drive = f32::from_bits(track.snare_drive.load(Ordering::Relaxed));
    let snare_level = f32::from_bits(track.snare_level.load(Ordering::Relaxed));
    let snare_filter_type = track.snare_filter_type.load(Ordering::Relaxed);
    let snare_filter_cutoff = f32::from_bits(track.snare_filter_cutoff.load(Ordering::Relaxed));
    let snare_filter_resonance =
        f32::from_bits(track.snare_filter_resonance.load(Ordering::Relaxed));

    for step in start..end {
        if (lanes & 0b01) != 0 {
            track.kick_step_override_enabled[step].store(false, Ordering::Relaxed);
            track.kick_step_pitch[step].store(kick_pitch.to_bits(), Ordering::Relaxed);
            track.kick_step_decay[step].store(kick_decay.to_bits(), Ordering::Relaxed);
            track.kick_step_attack[step].store(kick_attack.to_bits(), Ordering::Relaxed);
            track.kick_step_drive[step].store(kick_drive.to_bits(), Ordering::Relaxed);
            track.kick_step_level[step].store(kick_level.to_bits(), Ordering::Relaxed);
            track.kick_step_filter_type[step].store(kick_filter_type, Ordering::Relaxed);
            track.kick_step_filter_cutoff[step].store(kick_filter_cutoff.to_bits(), Ordering::Relaxed);
            track.kick_step_filter_resonance[step]
                .store(kick_filter_resonance.to_bits(), Ordering::Relaxed);
        }
        if (lanes & 0b10) != 0 {
            track.snare_step_override_enabled[step].store(false, Ordering::Relaxed);
            track.snare_step_tone[step].store(snare_tone.to_bits(), Ordering::Relaxed);
            track.snare_step_decay[step].store(snare_decay.to_bits(), Ordering::Relaxed);
            track.snare_step_snappy[step].store(snare_snappy.to_bits(), Ordering::Relaxed);
            track.snare_step_attack[step].store(snare_attack.to_bits(), Ordering::Relaxed);
            track.snare_step_drive[step].store(snare_drive.to_bits(), Ordering::Relaxed);
            track.snare_step_level[step].store(snare_level.to_bits(), Ordering::Relaxed);
            track.snare_step_filter_type[step].store(snare_filter_type, Ordering::Relaxed);
            track.snare_step_filter_cutoff[step].store(snare_filter_cutoff.to_bits(), Ordering::Relaxed);
            track.snare_step_filter_resonance[step]
                .store(snare_filter_resonance.to_bits(), Ordering::Relaxed);
        }
    }
}

fn syndrm_clear_apply(
    track: &Track,
    lanes: u8,
    start: usize,
    len: usize,
    clear_steps: bool,
    clear_params: bool,
) {
    if clear_steps {
        syndrm_clear_steps(track, lanes, start, len);
    }
    if clear_params {
        syndrm_clear_params(track, lanes, start, len);
    }
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

fn ring_rate_hz(rate_norm: f32, mode: u32, tempo: f32) -> f32 {
    let rate_norm = rate_norm.clamp(0.0, 1.0);
    if mode == 0 {
        return RING_LFO_RATE_MIN_HZ
            + rate_norm * (RING_LFO_RATE_MAX_HZ - RING_LFO_RATE_MIN_HZ);
    }

    let base = tempo.clamp(20.0, 300.0) / 60.0;
    let factor = match mode {
        1 => 1.0,
        2 => 1.5,
        3 => 2.0 / 3.0,
        _ => 1.0,
    };
    let multiplier = 0.25 + rate_norm * 3.75;
    (base * factor * multiplier).max(0.01)
}

fn ring_quantize_freq(freq: f32, scale_mode: u32) -> f32 {
    let freq = freq.max(1.0);
    let scale = match scale_mode {
        1 => [0, 2, 4, 5, 7, 9, 11],
        2 => [0, 2, 3, 5, 7, 8, 10],
        _ => return freq,
    };
    let note = 69.0 + 12.0 * (freq / 440.0).log2();
    let base_octave = (note / 12.0).floor();
    let semitone = ((note - base_octave * 12.0).round() as i32).rem_euclid(12);
    let mut closest = scale[0];
    let mut best_dist = 12;
    for &deg in scale.iter() {
        let dist = (deg - semitone).abs();
        if dist < best_dist {
            best_dist = dist;
            closest = deg;
        }
    }
    let quant_note = base_octave * 12.0 + closest as f32;
    440.0 * 2.0f32.powf((quant_note - 69.0) / 12.0)
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

fn image_from_rgba(width: u32, height: u32, data: &[u8]) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let pixels = buffer.make_mut_slice();
    for (pixel, chunk) in pixels.iter_mut().zip(data.chunks_exact(4)) {
        *pixel = Rgba8Pixel {
            r: chunk[0],
            g: chunk[1],
            b: chunk[2],
            a: chunk[3],
        };
    }
    Image::from_rgba8(buffer)
}

#[derive(Debug)]
enum MediaLoadError {
    NoVideoStream,
    Ffmpeg(String),
}

impl std::fmt::Display for MediaLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaLoadError::NoVideoStream => write!(f, "no video stream found"),
            MediaLoadError::Ffmpeg(msg) => write!(f, "ffmpeg error: {msg}"),
        }
    }
}

impl std::error::Error for MediaLoadError {}

fn load_media_file(
    path: &std::path::Path,
) -> Result<(Vec<Vec<f32>>, u32, Option<VideoCache>), Box<dyn std::error::Error>> {
    match load_audio_video_ffmpeg(path) {
        Ok((samples, sample_rate, video)) => Ok((samples, sample_rate, Some(video))),
        Err(MediaLoadError::NoVideoStream) => {
            load_audio_file(path).map(|(samples, sample_rate)| (samples, sample_rate, None))
        }
        Err(err) => Err(Box::new(err)),
    }
}

fn load_audio_video_ffmpeg(
    path: &std::path::Path,
) -> Result<(Vec<Vec<f32>>, u32, VideoCache), MediaLoadError> {
    use ffmpeg_next as ffmpeg;
    use ffmpeg::format::Pixel;
    use ffmpeg::media::Type;
    use ffmpeg::software::scaling::Flags as ScalingFlags;
    use ffmpeg::software::{resampling, scaling};
    use ffmpeg::util::format::sample::{Sample, Type as SampleType};

    static FFMPEG_INIT: Once = Once::new();
    FFMPEG_INIT.call_once(|| {
        let _ = ffmpeg::init();
    });

    let mut input = ffmpeg::format::input(&path).map_err(|e| {
        MediaLoadError::Ffmpeg(format!("failed to open media: {e:?}"))
    })?;

    let video_stream = input.streams().best(Type::Video);
    let audio_stream = input.streams().best(Type::Audio);

    let video_stream = video_stream.ok_or(MediaLoadError::NoVideoStream)?;
    let audio_stream = audio_stream.ok_or_else(|| {
        MediaLoadError::Ffmpeg("no audio stream found".to_string())
    })?;

    let video_stream_index = video_stream.index();
    let audio_stream_index = audio_stream.index();

    let mut video_decoder = ffmpeg::codec::Context::from_parameters(video_stream.parameters())
        .map_err(|e| MediaLoadError::Ffmpeg(format!("video params: {e:?}")))?
        .decoder()
        .video()
        .map_err(|e| MediaLoadError::Ffmpeg(format!("video decoder: {e:?}")))?;
    let mut audio_decoder = ffmpeg::codec::Context::from_parameters(audio_stream.parameters())
        .map_err(|e| MediaLoadError::Ffmpeg(format!("audio params: {e:?}")))?
        .decoder()
        .audio()
        .map_err(|e| MediaLoadError::Ffmpeg(format!("audio decoder: {e:?}")))?;

    let width = video_decoder.width();
    let height = video_decoder.height();

    let mut scaler = scaling::Context::get(
        video_decoder.format(),
        width,
        height,
        Pixel::RGBA,
        width,
        height,
        ScalingFlags::BILINEAR,
    )
    .map_err(|e| MediaLoadError::Ffmpeg(format!("scaler: {e:?}")))?;

    let out_sample_format = Sample::F32(SampleType::Planar);
    let mut resampler = resampling::Context::get(
        audio_decoder.format(),
        audio_decoder.channel_layout(),
        audio_decoder.rate(),
        out_sample_format,
        audio_decoder.channel_layout(),
        audio_decoder.rate(),
    )
    .map_err(|e| MediaLoadError::Ffmpeg(format!("resampler: {e:?}")))?;

    let channels = audio_decoder.channel_layout().channels() as usize;
    let mut samples: Vec<Vec<f32>> = vec![Vec::new(); channels.max(1)];
    let mut frames: Vec<VideoFrame> = Vec::new();

    let time_base = video_stream.time_base();
    let fps = {
        let rate = video_stream.avg_frame_rate();
        if rate.denominator() != 0 {
            rate.numerator() as f32 / rate.denominator() as f32
        } else {
            30.0
        }
    };

    let mut video_frame = ffmpeg::frame::Video::empty();
    let mut rgba_frame = ffmpeg::frame::Video::empty();
    let mut audio_frame = ffmpeg::frame::Audio::empty();

    for (stream, packet) in input.packets() {
        if stream.index() == video_stream_index {
            if video_decoder.send_packet(&packet).is_ok() {
                while video_decoder.receive_frame(&mut video_frame).is_ok() {
                    scaler
                        .run(&video_frame, &mut rgba_frame)
                        .map_err(|e| MediaLoadError::Ffmpeg(format!("scale: {e:?}")))?;
                    let stride = rgba_frame.stride(0);
                    let data = rgba_frame.data(0);
                    let mut buffer = vec![0u8; (width * height * 4) as usize];
                    for y in 0..height as usize {
                        let row_start = y * stride;
                        let row_end = row_start + (width * 4) as usize;
                        let dst_start = y * (width * 4) as usize;
                        let dst_end = dst_start + (width * 4) as usize;
                        buffer[dst_start..dst_end].copy_from_slice(&data[row_start..row_end]);
                    }
                    let pts = video_frame.pts().unwrap_or(0);
                    let timestamp = pts as f32 * time_base.numerator() as f32
                        / time_base.denominator() as f32;
                    frames.push(VideoFrame {
                        timestamp,
                        data: Arc::new(buffer),
                    });
                }
            }
        } else if stream.index() == audio_stream_index {
            if audio_decoder.send_packet(&packet).is_ok() {
                while audio_decoder.receive_frame(&mut audio_frame).is_ok() {
                    let mut out = ffmpeg::frame::Audio::empty();
                    resampler
                        .run(&audio_frame, &mut out)
                        .map_err(|e| MediaLoadError::Ffmpeg(format!("resample: {e:?}")))?;
                    let out_samples = out.samples() as usize;
                    let out_channels = out.channels() as usize;
                    if samples.len() < out_channels {
                        samples.resize_with(out_channels, Vec::new);
                    }
                    for ch in 0..out_channels {
                        let data = out.data(ch);
                        let data = unsafe {
                            std::slice::from_raw_parts(
                                data.as_ptr() as *const f32,
                                out_samples,
                            )
                        };
                        samples[ch].extend_from_slice(data);
                    }
                }
            }
        }
    }

    let _ = video_decoder.send_eof();
    while video_decoder.receive_frame(&mut video_frame).is_ok() {
        scaler
            .run(&video_frame, &mut rgba_frame)
            .map_err(|e| MediaLoadError::Ffmpeg(format!("scale: {e:?}")))?;
        let stride = rgba_frame.stride(0);
        let data = rgba_frame.data(0);
        let mut buffer = vec![0u8; (width * height * 4) as usize];
        for y in 0..height as usize {
            let row_start = y * stride;
            let row_end = row_start + (width * 4) as usize;
            let dst_start = y * (width * 4) as usize;
            let dst_end = dst_start + (width * 4) as usize;
            buffer[dst_start..dst_end].copy_from_slice(&data[row_start..row_end]);
        }
        let pts = video_frame.pts().unwrap_or(0);
        let timestamp = pts as f32 * time_base.numerator() as f32
            / time_base.denominator() as f32;
        frames.push(VideoFrame {
            timestamp,
            data: Arc::new(buffer),
        });
    }

    let _ = audio_decoder.send_eof();
    while audio_decoder.receive_frame(&mut audio_frame).is_ok() {
        let mut out = ffmpeg::frame::Audio::empty();
        resampler
            .run(&audio_frame, &mut out)
            .map_err(|e| MediaLoadError::Ffmpeg(format!("resample: {e:?}")))?;
        let out_samples = out.samples() as usize;
        let out_channels = out.channels() as usize;
        if samples.len() < out_channels {
            samples.resize_with(out_channels, Vec::new);
        }
        for ch in 0..out_channels {
            let data = out.data(ch);
            let data = unsafe {
                std::slice::from_raw_parts(
                    data.as_ptr() as *const f32,
                    out_samples,
                )
            };
            samples[ch].extend_from_slice(data);
        }
    }

    let sample_rate = audio_decoder.rate() as u32;
    let video = VideoCache {
        frames,
        width,
        height,
        fps: if fps.is_finite() && fps > 0.0 { fps } else { 30.0 },
    };

    Ok((samples, sample_rate, video))
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


fn capture_track_params(track: &Track, params: &mut HashMap<String, f32>) {
    let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
    let b = |a: &AtomicBool| if a.load(Ordering::Relaxed) { 1.0 } else { 0.0 };
    let u = |a: &AtomicU32| a.load(Ordering::Relaxed) as f32;

    params.insert("level".to_string(), f(&track.level));
    params.insert("muted".to_string(), b(&track.is_muted));
    params.insert("tape_speed".to_string(), f(&track.tape_speed));
    params.insert("tape_rate_mode".to_string(), u(&track.tape_rate_mode));
    params.insert("tape_rotate".to_string(), f(&track.tape_rotate));
    params.insert("tape_glide".to_string(), f(&track.tape_glide));
    params.insert("tape_sos".to_string(), f(&track.tape_sos));
    params.insert("tape_reverse".to_string(), b(&track.tape_reverse));
    params.insert("tape_freeze".to_string(), b(&track.tape_freeze));
    params.insert("tape_keylock".to_string(), b(&track.tape_keylock));
    params.insert("tape_monitor".to_string(), b(&track.tape_monitor));
    params.insert("tape_overdub".to_string(), b(&track.tape_overdub));
    params.insert("loop_start".to_string(), f(&track.loop_start));
    params.insert("trigger_start".to_string(), f(&track.trigger_start));
    params.insert("loop_length".to_string(), f(&track.loop_length));
    params.insert("loop_xfade".to_string(), f(&track.loop_xfade));
    params.insert("loop_enabled".to_string(), b(&track.loop_enabled));
    params.insert("loop_mode".to_string(), u(&track.loop_mode));
    params.insert("granular_type".to_string(), u(&track.granular_type));
    params.insert("mosaic_pitch".to_string(), f(&track.mosaic_pitch));
    params.insert("mosaic_rate".to_string(), f(&track.mosaic_rate));
    params.insert("mosaic_size".to_string(), f(&track.mosaic_size));
    params.insert("mosaic_contour".to_string(), f(&track.mosaic_contour));
    params.insert("mosaic_warp".to_string(), f(&track.mosaic_warp));
    params.insert("mosaic_spray".to_string(), f(&track.mosaic_spray));
    params.insert("mosaic_pattern".to_string(), f(&track.mosaic_pattern));
    params.insert("mosaic_wet".to_string(), f(&track.mosaic_wet));
    params.insert("mosaic_spatial".to_string(), f(&track.mosaic_spatial));
    params.insert("mosaic_detune".to_string(), f(&track.mosaic_detune));
    params.insert("mosaic_rand_rate".to_string(), f(&track.mosaic_rand_rate));
    params.insert("mosaic_rand_size".to_string(), f(&track.mosaic_rand_size));
    params.insert("mosaic_sos".to_string(), f(&track.mosaic_sos));
    params.insert("mosaic_enabled".to_string(), b(&track.mosaic_enabled));
    params.insert("ring_cutoff".to_string(), f(&track.ring_cutoff));
    params.insert("ring_resonance".to_string(), f(&track.ring_resonance));
    params.insert("ring_decay".to_string(), f(&track.ring_decay));
    params.insert("ring_decay_mode".to_string(), u(&track.ring_decay_mode));
    params.insert("ring_pitch".to_string(), f(&track.ring_pitch));
    params.insert("ring_tone".to_string(), f(&track.ring_tone));
    params.insert("ring_tilt".to_string(), f(&track.ring_tilt));
    params.insert("ring_slope".to_string(), f(&track.ring_slope));
    params.insert("ring_wet".to_string(), f(&track.ring_wet));
    params.insert("ring_detune".to_string(), f(&track.ring_detune));
    params.insert("ring_waves".to_string(), f(&track.ring_waves));
    params.insert("ring_waves_rate".to_string(), f(&track.ring_waves_rate));
    params.insert("ring_waves_rate_mode".to_string(), u(&track.ring_waves_rate_mode));
    params.insert("ring_noise".to_string(), f(&track.ring_noise));
    params.insert("ring_noise_rate".to_string(), f(&track.ring_noise_rate));
    params.insert("ring_noise_rate_mode".to_string(), u(&track.ring_noise_rate_mode));
    params.insert("ring_scale".to_string(), u(&track.ring_scale));
    params.insert("ring_enabled".to_string(), b(&track.ring_enabled));

    for i in 0..4 {
        params.insert(format!("animate_slot_type_{}", i), u(&track.animate_slot_types[i]));
        params.insert(format!("animate_slot_wavetable_{}", i), u(&track.animate_slot_wavetables[i]));
        params.insert(format!("animate_slot_sample_{}", i), u(&track.animate_slot_samples[i]));
        params.insert(format!("animate_slot_coarse_{}", i), f(&track.animate_slot_coarse[i]));
        params.insert(format!("animate_slot_fine_{}", i), f(&track.animate_slot_fine[i]));
        params.insert(format!("animate_slot_level_{}", i), f(&track.animate_slot_level[i]));
        params.insert(format!("animate_slot_pan_{}", i), f(&track.animate_slot_pan[i]));
        params.insert(format!("animate_slot_wt_lfo_amount_{}", i), f(&track.animate_slot_wt_lfo_amount[i]));
        params.insert(format!("animate_slot_wt_lfo_shape_{}", i), u(&track.animate_slot_wt_lfo_shape[i]));
        params.insert(format!("animate_slot_wt_lfo_rate_{}", i), f(&track.animate_slot_wt_lfo_rate[i]));
        params.insert(format!("animate_slot_wt_lfo_sync_{}", i), b(&track.animate_slot_wt_lfo_sync[i]));
        params.insert(format!("animate_slot_wt_lfo_division_{}", i), u(&track.animate_slot_wt_lfo_division[i]));
        params.insert(format!("animate_slot_sample_start_{}", i), f(&track.animate_slot_sample_start[i]));
        params.insert(format!("animate_slot_loop_start_{}", i), f(&track.animate_slot_loop_start[i]));
        params.insert(format!("animate_slot_loop_end_{}", i), f(&track.animate_slot_loop_end[i]));
        params.insert(format!("animate_slot_filter_type_{}", i), u(&track.animate_slot_filter_type[i]));
        params.insert(format!("animate_slot_filter_cutoff_{}", i), f(&track.animate_slot_filter_cutoff[i]));
        params.insert(format!("animate_slot_filter_resonance_{}", i), f(&track.animate_slot_filter_resonance[i]));
    }

    params.insert("animate_vector_x".to_string(), f(&track.animate_vector_x));
    params.insert("animate_vector_y".to_string(), f(&track.animate_vector_y));
    params.insert("animate_lfo_x_waveform".to_string(), u(&track.animate_lfo_x_waveform));
    params.insert("animate_lfo_x_sync".to_string(), b(&track.animate_lfo_x_sync));
    params.insert("animate_lfo_x_division".to_string(), u(&track.animate_lfo_x_division));
    params.insert("animate_lfo_x_rate".to_string(), f(&track.animate_lfo_x_rate));
    params.insert("animate_lfo_x_amount".to_string(), f(&track.animate_lfo_x_amount));
    params.insert("animate_lfo_y_waveform".to_string(), u(&track.animate_lfo_y_waveform));
    params.insert("animate_lfo_y_sync".to_string(), b(&track.animate_lfo_y_sync));
    params.insert("animate_lfo_y_division".to_string(), u(&track.animate_lfo_y_division));
    params.insert("animate_lfo_y_rate".to_string(), f(&track.animate_lfo_y_rate));
    params.insert("animate_lfo_y_amount".to_string(), f(&track.animate_lfo_y_amount));

    params.insert("kick_pitch".to_string(), f(&track.kick_pitch));
    params.insert("kick_decay".to_string(), f(&track.kick_decay));
    params.insert("kick_attack".to_string(), f(&track.kick_attack));
    params.insert(
        "kick_pitch_env_amount".to_string(),
        f(&track.kick_pitch_env_amount),
    );
    params.insert("kick_drive".to_string(), f(&track.kick_drive));
    params.insert("kick_level".to_string(), f(&track.kick_level));
    params.insert("kick_filter_type".to_string(), u(&track.kick_filter_type));
    params.insert("kick_filter_cutoff".to_string(), f(&track.kick_filter_cutoff));
    params.insert("kick_filter_resonance".to_string(), f(&track.kick_filter_resonance));
    params.insert("kick_filter_pre_drive".to_string(), b(&track.kick_filter_pre_drive));
    params.insert("snare_tone".to_string(), f(&track.snare_tone));
    params.insert("snare_decay".to_string(), f(&track.snare_decay));
    params.insert("snare_snappy".to_string(), f(&track.snare_snappy));
    params.insert("snare_attack".to_string(), f(&track.snare_attack));
    params.insert("snare_drive".to_string(), f(&track.snare_drive));
    params.insert("snare_level".to_string(), f(&track.snare_level));
    params.insert("snare_filter_type".to_string(), u(&track.snare_filter_type));
    params.insert("snare_filter_cutoff".to_string(), f(&track.snare_filter_cutoff));
    params.insert("snare_filter_resonance".to_string(), f(&track.snare_filter_resonance));
    params.insert("snare_filter_pre_drive".to_string(), b(&track.snare_filter_pre_drive));
    params.insert("snare_tone".to_string(), f(&track.snare_tone));
    params.insert("snare_decay".to_string(), f(&track.snare_decay));
    params.insert("snare_snappy".to_string(), f(&track.snare_snappy));
    params.insert("snare_attack".to_string(), f(&track.snare_attack));
    params.insert("snare_level".to_string(), f(&track.snare_level));
    params.insert("syndrm_page".to_string(), u(&track.syndrm_page));
    params.insert("syndrm_edit_lane".to_string(), u(&track.syndrm_edit_lane));
    params.insert("syndrm_edit_step".to_string(), u(&track.syndrm_edit_step));
    params.insert("syndrm_step_hold".to_string(), b(&track.syndrm_step_hold));
    for i in 0..SYNDRM_STEPS {
        params.insert(
            format!("syndrm_kick_step_override_{}", i),
            b(&track.kick_step_override_enabled[i]),
        );
        params.insert(format!("syndrm_kick_step_pitch_{}", i), f(&track.kick_step_pitch[i]));
        params.insert(format!("syndrm_kick_step_decay_{}", i), f(&track.kick_step_decay[i]));
        params.insert(format!("syndrm_kick_step_attack_{}", i), f(&track.kick_step_attack[i]));
        params.insert(format!("syndrm_kick_step_drive_{}", i), f(&track.kick_step_drive[i]));
        params.insert(format!("syndrm_kick_step_level_{}", i), f(&track.kick_step_level[i]));
        params.insert(
            format!("syndrm_kick_step_filter_type_{}", i),
            u(&track.kick_step_filter_type[i]),
        );
        params.insert(
            format!("syndrm_kick_step_filter_cutoff_{}", i),
            f(&track.kick_step_filter_cutoff[i]),
        );
        params.insert(
            format!("syndrm_kick_step_filter_resonance_{}", i),
            f(&track.kick_step_filter_resonance[i]),
        );
        params.insert(
            format!("syndrm_snare_step_override_{}", i),
            b(&track.snare_step_override_enabled[i]),
        );
        params.insert(format!("syndrm_snare_step_tone_{}", i), f(&track.snare_step_tone[i]));
        params.insert(format!("syndrm_snare_step_decay_{}", i), f(&track.snare_step_decay[i]));
        params.insert(
            format!("syndrm_snare_step_snappy_{}", i),
            f(&track.snare_step_snappy[i]),
        );
        params.insert(
            format!("syndrm_snare_step_attack_{}", i),
            f(&track.snare_step_attack[i]),
        );
        params.insert(format!("syndrm_snare_step_drive_{}", i), f(&track.snare_step_drive[i]));
        params.insert(format!("syndrm_snare_step_level_{}", i), f(&track.snare_step_level[i]));
        params.insert(
            format!("syndrm_snare_step_filter_type_{}", i),
            u(&track.snare_step_filter_type[i]),
        );
        params.insert(
            format!("syndrm_snare_step_filter_cutoff_{}", i),
            f(&track.snare_step_filter_cutoff[i]),
        );
        params.insert(
            format!("syndrm_snare_step_filter_resonance_{}", i),
            f(&track.snare_step_filter_resonance[i]),
        );
    }

    params.insert("void_base_freq".to_string(), f(&track.void_base_freq));
    params.insert("void_chaos_depth".to_string(), f(&track.void_chaos_depth));
    params.insert("void_entropy".to_string(), f(&track.void_entropy));
    params.insert("void_feedback".to_string(), f(&track.void_feedback));
    params.insert("void_diffusion".to_string(), f(&track.void_diffusion));
    params.insert("void_mod_rate".to_string(), f(&track.void_mod_rate));
    params.insert("void_level".to_string(), f(&track.void_level));
    params.insert("void_enabled".to_string(), b(&track.void_enabled));
}

fn apply_track_params(track: &Track, params: &HashMap<String, f32>) {
    let sf = |a: &AtomicU32, name: &str| {
        if let Some(&v) = params.get(name) {
            a.store(v.to_bits(), Ordering::Relaxed);
        }
    };
    let sb = |a: &AtomicBool, name: &str| {
        if let Some(&v) = params.get(name) {
            a.store(v > 0.5, Ordering::Relaxed);
        }
    };
    let su = |a: &AtomicU32, name: &str| {
        if let Some(&v) = params.get(name) {
            a.store(v as u32, Ordering::Relaxed);
        }
    };

    sf(&track.level, "level");
    sb(&track.is_muted, "muted");
    sf(&track.tape_speed, "tape_speed");
    sf(&track.tape_speed_smooth, "tape_speed");
    su(&track.tape_rate_mode, "tape_rate_mode");
    sf(&track.tape_rotate, "tape_rotate");
    sf(&track.tape_glide, "tape_glide");
    sf(&track.tape_sos, "tape_sos");
    sb(&track.tape_reverse, "tape_reverse");
    sb(&track.tape_freeze, "tape_freeze");
    sb(&track.tape_keylock, "tape_keylock");
    sb(&track.tape_monitor, "tape_monitor");
    sb(&track.tape_overdub, "tape_overdub");
    sf(&track.loop_start, "loop_start");
    sf(&track.trigger_start, "trigger_start");
    sf(&track.loop_length, "loop_length");
    sf(&track.loop_xfade, "loop_xfade");
    sb(&track.loop_enabled, "loop_enabled");
    su(&track.loop_mode, "loop_mode");
    su(&track.granular_type, "granular_type");
    sf(&track.mosaic_pitch, "mosaic_pitch");
    sf(&track.mosaic_rate, "mosaic_rate");
    sf(&track.mosaic_size, "mosaic_size");
    sf(&track.mosaic_contour, "mosaic_contour");
    sf(&track.mosaic_warp, "mosaic_warp");
    sf(&track.mosaic_spray, "mosaic_spray");
    sf(&track.mosaic_pattern, "mosaic_pattern");
    sf(&track.mosaic_wet, "mosaic_wet");
    sf(&track.mosaic_spatial, "mosaic_spatial");
    sf(&track.mosaic_detune, "mosaic_detune");
    sf(&track.mosaic_rand_rate, "mosaic_rand_rate");
    sf(&track.mosaic_rand_size, "mosaic_rand_size");
    sf(&track.mosaic_sos, "mosaic_sos");
    sb(&track.mosaic_enabled, "mosaic_enabled");
    sf(&track.ring_cutoff, "ring_cutoff");
    sf(&track.ring_resonance, "ring_resonance");
    sf(&track.ring_decay, "ring_decay");
    su(&track.ring_decay_mode, "ring_decay_mode");
    sf(&track.ring_pitch, "ring_pitch");
    sf(&track.ring_tone, "ring_tone");
    sf(&track.ring_tilt, "ring_tilt");
    sf(&track.ring_slope, "ring_slope");
    sf(&track.ring_wet, "ring_wet");
    sf(&track.ring_detune, "ring_detune");
    sf(&track.ring_waves, "ring_waves");
    sf(&track.ring_waves_rate, "ring_waves_rate");
    su(&track.ring_waves_rate_mode, "ring_waves_rate_mode");
    sf(&track.ring_noise, "ring_noise");
    sf(&track.ring_noise_rate, "ring_noise_rate");
    su(&track.ring_noise_rate_mode, "ring_noise_rate_mode");
    su(&track.ring_scale, "ring_scale");
    sb(&track.ring_enabled, "ring_enabled");

    for i in 0..4 {
        su(&track.animate_slot_types[i], &format!("animate_slot_type_{}", i));
        su(&track.animate_slot_wavetables[i], &format!("animate_slot_wavetable_{}", i));
        su(&track.animate_slot_samples[i], &format!("animate_slot_sample_{}", i));
        sf(&track.animate_slot_coarse[i], &format!("animate_slot_coarse_{}", i));
        sf(&track.animate_slot_fine[i], &format!("animate_slot_fine_{}", i));
        sf(&track.animate_slot_level[i], &format!("animate_slot_level_{}", i));
        sf(&track.animate_slot_pan[i], &format!("animate_slot_pan_{}", i));
        sf(&track.animate_slot_wt_lfo_amount[i], &format!("animate_slot_wt_lfo_amount_{}", i));
        su(&track.animate_slot_wt_lfo_shape[i], &format!("animate_slot_wt_lfo_shape_{}", i));
        sf(&track.animate_slot_wt_lfo_rate[i], &format!("animate_slot_wt_lfo_rate_{}", i));
        sb(&track.animate_slot_wt_lfo_sync[i], &format!("animate_slot_wt_lfo_sync_{}", i));
        su(&track.animate_slot_wt_lfo_division[i], &format!("animate_slot_wt_lfo_division_{}", i));
        sf(&track.animate_slot_sample_start[i], &format!("animate_slot_sample_start_{}", i));
        sf(&track.animate_slot_loop_start[i], &format!("animate_slot_loop_start_{}", i));
        sf(&track.animate_slot_loop_end[i], &format!("animate_slot_loop_end_{}", i));
        su(&track.animate_slot_filter_type[i], &format!("animate_slot_filter_type_{}", i));
        sf(&track.animate_slot_filter_cutoff[i], &format!("animate_slot_filter_cutoff_{}", i));
        sf(&track.animate_slot_filter_resonance[i], &format!("animate_slot_filter_resonance_{}", i));
    }

    sf(&track.animate_vector_x, "animate_vector_x");
    sf(&track.animate_vector_y, "animate_vector_y");
    su(&track.animate_lfo_x_waveform, "animate_lfo_x_waveform");
    sb(&track.animate_lfo_x_sync, "animate_lfo_x_sync");
    su(&track.animate_lfo_x_division, "animate_lfo_x_division");
    sf(&track.animate_lfo_x_rate, "animate_lfo_x_rate");
    sf(&track.animate_lfo_x_amount, "animate_lfo_x_amount");
    su(&track.animate_lfo_y_waveform, "animate_lfo_y_waveform");
    sb(&track.animate_lfo_y_sync, "animate_lfo_y_sync");
    su(&track.animate_lfo_y_division, "animate_lfo_y_division");
    sf(&track.animate_lfo_y_rate, "animate_lfo_y_rate");
    sf(&track.animate_lfo_y_amount, "animate_lfo_y_amount");

    sf(&track.kick_pitch, "kick_pitch");
    sf(&track.kick_decay, "kick_decay");
    sf(&track.kick_attack, "kick_attack");
    sf(&track.kick_pitch_env_amount, "kick_pitch_env_amount");
    sf(&track.kick_drive, "kick_drive");
    sf(&track.kick_level, "kick_level");
    su(&track.kick_filter_type, "kick_filter_type");
    sf(&track.kick_filter_cutoff, "kick_filter_cutoff");
    sf(&track.kick_filter_resonance, "kick_filter_resonance");
    sb(&track.kick_filter_pre_drive, "kick_filter_pre_drive");
    sf(&track.snare_tone, "snare_tone");
    sf(&track.snare_decay, "snare_decay");
    sf(&track.snare_snappy, "snare_snappy");
    sf(&track.snare_attack, "snare_attack");
    sf(&track.snare_drive, "snare_drive");
    sf(&track.snare_level, "snare_level");
    su(&track.snare_filter_type, "snare_filter_type");
    sf(&track.snare_filter_cutoff, "snare_filter_cutoff");
    sf(&track.snare_filter_resonance, "snare_filter_resonance");
    sb(&track.snare_filter_pre_drive, "snare_filter_pre_drive");
    su(&track.syndrm_page, "syndrm_page");
    su(&track.syndrm_edit_lane, "syndrm_edit_lane");
    su(&track.syndrm_edit_step, "syndrm_edit_step");
    sb(&track.syndrm_step_hold, "syndrm_step_hold");
    for i in 0..SYNDRM_STEPS {
        sb(&track.kick_step_override_enabled[i], &format!("syndrm_kick_step_override_{}", i));
        sf(&track.kick_step_pitch[i], &format!("syndrm_kick_step_pitch_{}", i));
        sf(&track.kick_step_decay[i], &format!("syndrm_kick_step_decay_{}", i));
        sf(&track.kick_step_attack[i], &format!("syndrm_kick_step_attack_{}", i));
        sf(&track.kick_step_drive[i], &format!("syndrm_kick_step_drive_{}", i));
        sf(&track.kick_step_level[i], &format!("syndrm_kick_step_level_{}", i));
        su(&track.kick_step_filter_type[i], &format!("syndrm_kick_step_filter_type_{}", i));
        sf(
            &track.kick_step_filter_cutoff[i],
            &format!("syndrm_kick_step_filter_cutoff_{}", i),
        );
        sf(
            &track.kick_step_filter_resonance[i],
            &format!("syndrm_kick_step_filter_resonance_{}", i),
        );
        sb(&track.snare_step_override_enabled[i], &format!("syndrm_snare_step_override_{}", i));
        sf(&track.snare_step_tone[i], &format!("syndrm_snare_step_tone_{}", i));
        sf(&track.snare_step_decay[i], &format!("syndrm_snare_step_decay_{}", i));
        sf(
            &track.snare_step_snappy[i],
            &format!("syndrm_snare_step_snappy_{}", i),
        );
        sf(&track.snare_step_attack[i], &format!("syndrm_snare_step_attack_{}", i));
        sf(&track.snare_step_drive[i], &format!("syndrm_snare_step_drive_{}", i));
        sf(&track.snare_step_level[i], &format!("syndrm_snare_step_level_{}", i));
        su(
            &track.snare_step_filter_type[i],
            &format!("syndrm_snare_step_filter_type_{}", i),
        );
        sf(
            &track.snare_step_filter_cutoff[i],
            &format!("syndrm_snare_step_filter_cutoff_{}", i),
        );
        sf(
            &track.snare_step_filter_resonance[i],
            &format!("syndrm_snare_step_filter_resonance_{}", i),
        );
    }

    sf(&track.void_base_freq, "void_base_freq");
    sf(&track.void_chaos_depth, "void_chaos_depth");
    sf(&track.void_entropy, "void_entropy");
    sf(&track.void_feedback, "void_feedback");
    sf(&track.void_diffusion, "void_diffusion");
    sf(&track.void_mod_rate, "void_mod_rate");
    sf(&track.void_level, "void_level");
    sb(&track.void_enabled, "void_enabled");
}

fn save_project(
    tracks: &Arc<[Track; NUM_TRACKS]>,
    global_tempo: f32,
    params: &Arc<TLBX1Params>,
    title: &str,
    description: &str,
    project_dir: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(project_dir)?;
    let samples_dir = project_dir.join("samples");
    fs::create_dir_all(&samples_dir)?;

    let mut track_file_names = Vec::new();

    for (i, track) in tracks.iter().enumerate() {
        let track_idx = i + 1;
        let mut track_data = TrackData {
            engine_type: track.engine_type.load(Ordering::Relaxed),
            params: HashMap::new(),
            sequence: Vec::new(),
            sample_path: None,
        };

        capture_track_params(track, &mut track_data.params);

        if track_data.engine_type == 2 {
            let grid = track.animate_sequencer_grid.clone();
            for j in 0..160 {
                track_data.sequence.push(grid[j].load(Ordering::Relaxed));
            }
        } else if track_data.engine_type == 3 {
            let grid = track.kick_sequencer_grid.clone();
            for j in 0..SYNDRM_STEPS {
                track_data.sequence.push(grid[j].load(Ordering::Relaxed));
            }
            let snare_grid = track.snare_sequencer_grid.clone();
            for j in 0..SYNDRM_STEPS {
                track_data.sequence.push(snare_grid[j].load(Ordering::Relaxed));
            }
        }

        if let Some(path) = track.sample_path.lock().as_ref() {
            if let Some(file_name) = path.file_name() {
                let dest_path = samples_dir.join(file_name);
                if path.exists() {
                    fs::copy(path, &dest_path)?;
                    track_data.sample_path = Some(format!("samples/{}", file_name.to_string_lossy()));
                }
            }
        }

        let track_file_name = format!("{}.trk", track_idx);
        let track_path = project_dir.join(&track_file_name);
        let track_json = serde_json::to_string_pretty(&track_data)?;
        fs::write(track_path, track_json)?;
        track_file_names.push(track_file_name);
    }

    let project_data = ProjectData {
        title: title.to_string(),
        description: description.to_string(),
        bpm: global_tempo,
        master_gain: params.gain.value(),
        master_filter: params.master_filter.value(),
        master_comp: params.master_comp.value(),
        tracks: track_file_names,
    };

    let project_file_name = format!("{}.tlbx", title);
    let project_path = project_dir.join(project_file_name);
    let project_json = serde_json::to_string_pretty(&project_data)?;
    fs::write(project_path, project_json)?;

    Ok(())
}

fn load_project(
    tracks: &Arc<[Track; NUM_TRACKS]>,
    global_tempo: &Arc<AtomicU32>,
    _params: &Arc<TLBX1Params>,
    pending_project_params: &Arc<Mutex<Option<PendingProjectParams>>>,
    path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_dir = path.parent().ok_or("Invalid project path")?;
    let json = fs::read_to_string(path)?;
    let project: ProjectData = serde_json::from_str(&json)?;

    global_tempo.store(project.bpm.to_bits(), Ordering::Relaxed);
    *pending_project_params.lock() = Some(PendingProjectParams {
        gain: project.master_gain,
        master_filter: project.master_filter,
        master_comp: project.master_comp,
    });

    for (track_idx, track_file_name) in project.tracks.iter().enumerate() {
        if track_idx >= NUM_TRACKS {
            break;
        }
        let track = &tracks[track_idx];
        let track_path = project_dir.join(track_file_name);
        if !track_path.exists() {
            continue;
        }
        let track_json = fs::read_to_string(track_path)?;
        let track_data: TrackData = serde_json::from_str(&track_json)?;

        track.engine_type.store(track_data.engine_type, Ordering::Relaxed);
        apply_track_params(track, &track_data.params);

        if track_data.engine_type == 2 && track_data.sequence.len() == 160 {
            let grid = track.animate_sequencer_grid.clone();
            for j in 0..160 {
                grid[j].store(track_data.sequence[j], Ordering::Relaxed);
            }
        } else if track_data.engine_type == 3 && track_data.sequence.len() == SYNDRM_STEPS * 2 {
            let grid = track.kick_sequencer_grid.clone();
            for j in 0..SYNDRM_STEPS {
                grid[j].store(track_data.sequence[j], Ordering::Relaxed);
            }
            let snare_grid = track.snare_sequencer_grid.clone();
            for j in 0..SYNDRM_STEPS {
                snare_grid[j].store(track_data.sequence[j + SYNDRM_STEPS], Ordering::Relaxed);
            }
        } else if track_data.engine_type == 3 && track_data.sequence.len() == 32 {
            let grid = track.kick_sequencer_grid.clone();
            for j in 0..16 {
                grid[j].store(track_data.sequence[j], Ordering::Relaxed);
            }
            let snare_grid = track.snare_sequencer_grid.clone();
            for j in 0..16 {
                snare_grid[j].store(track_data.sequence[j + 16], Ordering::Relaxed);
            }
        } else if track_data.engine_type == 3 && track_data.sequence.len() == 16 {
            let grid = track.kick_sequencer_grid.clone();
            for j in 0..16 {
                grid[j].store(track_data.sequence[j], Ordering::Relaxed);
            }
        }

        track.is_playing.store(false, Ordering::Relaxed);
        track.is_recording.store(false, Ordering::Relaxed);
        track.play_pos.store(0.0f32.to_bits(), Ordering::Relaxed);

        let mut samples = track.samples.lock();
        let mut summary = track.waveform_summary.lock();
        let mut sample_path = track.sample_path.lock();
        
        if let Some(rel_path) = &track_data.sample_path {
            let abs_path = project_dir.join(rel_path);
            match load_audio_file(&abs_path) {
                Ok((new_samples, sample_rate)) => {
                    *samples = new_samples;
                    *sample_path = Some(abs_path);
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

fn export_project_as_zip(
    tracks: &Arc<[Track; NUM_TRACKS]>,
    global_tempo: f32,
    params: &Arc<TLBX1Params>,
    title: &str,
    description: &str,
    zip_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::create(zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let mut track_file_names = Vec::new();
    for i in 1..=NUM_TRACKS {
        track_file_names.push(format!("{}.trk", i));
    }

    let project_data = ProjectData {
        title: title.to_string(),
        description: description.to_string(),
        bpm: global_tempo,
        master_gain: params.gain.value(),
        master_filter: params.master_filter.value(),
        master_comp: params.master_comp.value(),
        tracks: track_file_names,
    };

    let project_json = serde_json::to_string_pretty(&project_data)?;
    zip.start_file(format!("{}.tlbx", title), options)?;
    zip.write_all(project_json.as_bytes())?;

    for (i, track) in tracks.iter().enumerate() {
        let mut track_data = TrackData {
            engine_type: track.engine_type.load(Ordering::Relaxed),
            params: HashMap::new(),
            sequence: Vec::new(),
            sample_path: None,
        };

        capture_track_params(track, &mut track_data.params);

        if track_data.engine_type == 2 {
            let grid = track.animate_sequencer_grid.clone();
            for j in 0..160 {
                track_data.sequence.push(grid[j].load(Ordering::Relaxed));
            }
        } else if track_data.engine_type == 3 {
            let grid = track.kick_sequencer_grid.clone();
            for j in 0..SYNDRM_STEPS {
                track_data.sequence.push(grid[j].load(Ordering::Relaxed));
            }
            let snare_grid = track.snare_sequencer_grid.clone();
            for j in 0..SYNDRM_STEPS {
                track_data.sequence.push(snare_grid[j].load(Ordering::Relaxed));
            }
        }

        if let Some(path) = track.sample_path.lock().as_ref() {
            if let Some(file_name) = path.file_name() {
                let rel_sample_path = format!("samples/{}", file_name.to_string_lossy());
                track_data.sample_path = Some(rel_sample_path.clone());

                if path.exists() {
                    zip.start_file(rel_sample_path, options)?;
                    let sample_bytes = fs::read(path)?;
                    zip.write_all(&sample_bytes)?;
                }
            }
        }

        let track_json = serde_json::to_string_pretty(&track_data)?;
        zip.start_file(format!("{}.trk", i + 1), options)?;
        zip.write_all(track_json.as_bytes())?;
    }

    zip.finish()?;
    Ok(())
}

fn refresh_browser_impl(
    ui: &TLBX1UI,
    current_path: &Path,
    current_folder_content_model: &VecModel<BrowserEntry>,
) {
    let mut entries = Vec::new();

    if let Some(parent) = current_path.parent() {
        if current_path != Path::new(".") && current_path.as_os_str() != "" {
            entries.push(BrowserEntry {
                name: "..".into(),
                is_dir: true,
                path: parent.to_string_lossy().to_string().into(),
            });
        }
    }

    if let Ok(dir_entries) = std::fs::read_dir(current_path) {
        let mut folders = Vec::new();
        let mut files = Vec::new();

        for entry in dir_entries.flatten() {
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string().into();
            let is_dir = entry_path.is_dir();

            if is_dir {
                folders.push(BrowserEntry {
                    name,
                    is_dir,
                    path: entry_path.to_string_lossy().to_string().into(),
                });
            } else {
                let ext = entry_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "tlbx" | "wav" | "mp3" | "json" | "trk") {
                    files.push(BrowserEntry {
                        name,
                        is_dir,
                        path: entry_path.to_string_lossy().to_string().into(),
                    });
                }
            }
        }

        folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        entries.extend(folders);
        entries.extend(files);
    }

    current_folder_content_model.set_vec(entries);
    ui.set_current_path(current_path.to_string_lossy().to_string().into());
}

struct SlintEditor {
    params: Arc<TLBX1Params>,
    tracks: Arc<[Track; NUM_TRACKS]>,
    master_meters: Arc<MasterMeters>,
    visualizer: Arc<VisualizerState>,
    global_tempo: Arc<AtomicU32>,
    follow_host_tempo: Arc<AtomicBool>,
    metronome_enabled: Arc<AtomicBool>,
    metronome_count_in_ticks: Arc<AtomicU32>,
    metronome_count_in_playback: Arc<AtomicBool>,
    metronome_count_in_record: Arc<AtomicBool>,
    async_executor: AsyncExecutor<TLBX1>,
    pending_project_params: Arc<Mutex<Option<PendingProjectParams>>>,
    animate_library: Arc<AnimateLibrary>,
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
        let pending_project_params = self.pending_project_params.clone();
        let animate_library = self.animate_library.clone();

        let initial_size = default_window_size();
        let window_handle = baseview::Window::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: "TLBX-1".to_string(),
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
                    pending_project_params,
                    animate_library,
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
    params: Arc<TLBX1Params>,
    tracks: Arc<[Track; NUM_TRACKS]>,
    master_meters: Arc<MasterMeters>,
    visualizer: Arc<VisualizerState>,
    global_tempo: Arc<AtomicU32>,
    _follow_host_tempo: Arc<AtomicBool>,
    metronome_enabled: Arc<AtomicBool>,
    metronome_count_in_ticks: Arc<AtomicU32>,
    metronome_count_in_playback: Arc<AtomicBool>,
    metronome_count_in_record: Arc<AtomicBool>,
    async_executor: AsyncExecutor<TLBX1>,
    pending_project_params: Arc<Mutex<Option<PendingProjectParams>>>,
    slint_window: std::rc::Rc<MinimalSoftwareWindow>,
    ui: Box<TLBX1UI>,
    waveform_model: std::rc::Rc<VecModel<f32>>,
    oscilloscope_model: std::rc::Rc<VecModel<f32>>,
    spectrum_model: std::rc::Rc<VecModel<f32>>,
    vectorscope_x_model: std::rc::Rc<VecModel<f32>>,
    vectorscope_y_model: std::rc::Rc<VecModel<f32>>,
    video_frame_cache: Vec<Option<(u32, usize, Image)>>,
    sample_dialog_rx: std::sync::mpsc::Receiver<SampleDialogAction>,
    project_dialog_rx: std::sync::mpsc::Receiver<ProjectDialogAction>,
    sb_surface: softbuffer::Surface<SoftbufferWindowHandleAdapter, SoftbufferWindowHandleAdapter>,
    _sb_context: softbuffer::Context<SoftbufferWindowHandleAdapter>,
    physical_width: u32,
    physical_height: u32,
    scale_factor: f32,
    pixel_buffer: Vec<PremultipliedRgbaColor>,
    last_cursor: LogicalPosition,
    _library_folders: Arc<Mutex<Vec<PathBuf>>>,
    current_path: Arc<Mutex<PathBuf>>,
    _library_folders_model: std::rc::Rc<VecModel<SharedString>>,
    current_folder_content_model: std::rc::Rc<VecModel<BrowserEntry>>,
    _animate_library: Arc<AnimateLibrary>,
}

impl SlintWindow {
    fn new(
        window: &mut BaseWindow,
        initial_size: baseview::Size,
        gui_context: Arc<dyn GuiContext>,
        params: Arc<TLBX1Params>,
        tracks: Arc<[Track; NUM_TRACKS]>,
        master_meters: Arc<MasterMeters>,
        visualizer: Arc<VisualizerState>,
        global_tempo: Arc<AtomicU32>,
        follow_host_tempo: Arc<AtomicBool>,
        metronome_enabled: Arc<AtomicBool>,
        metronome_count_in_ticks: Arc<AtomicU32>,
        metronome_count_in_playback: Arc<AtomicBool>,
        metronome_count_in_record: Arc<AtomicBool>,
        async_executor: AsyncExecutor<TLBX1>,
        pending_project_params: Arc<Mutex<Option<PendingProjectParams>>>,
        animate_library: Arc<AnimateLibrary>,
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
        let video_frame_cache = vec![None; NUM_TRACKS];
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
                    let dpi = GetDpiForWindow(handle.hwnd as _) as f32;
                    if dpi > 0.0 {
                        scale_factor = dpi / 96.0;
                    }
                    ShowWindow(handle.hwnd as _, SW_MAXIMIZE);
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
        slint_window.set_size(slint::WindowSize::Physical(PhysicalSize::new(physical_width, physical_height)));

        let output_devices = available_output_devices();
        let input_devices = available_input_devices();
        let sample_rates = vec![44100, 48000, 88200, 96000];
        let buffer_sizes = vec![256, 512, 1024, 2048, 4096];

        let _is_software = true;
        #[cfg(any(feature = "renderer-opengl", feature = "renderer-vulkan"))]
        let _is_software = false;

        ui.set_is_software_renderer(_is_software);

        let library_folders = Arc::new(Mutex::new(Vec::new()));
        let current_path = Arc::new(Mutex::new(PathBuf::from(".")));
        let library_folders_model = std::rc::Rc::new(VecModel::default());
        let current_folder_content_model = std::rc::Rc::new(VecModel::default());

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
            &library_folders,
            &current_path,
            &library_folders_model,
            &current_folder_content_model,
            &animate_library,
        );

        ui.set_library_folders(ModelRc::from(library_folders_model.clone()));
        ui.set_current_folder_content(ModelRc::from(current_folder_content_model.clone()));

        ui.show().unwrap();

        // Mark window as active
        slint_window.dispatch_event(WindowEvent::WindowActiveChanged(true));

        Self {
            gui_context,
            params,
            tracks,
            master_meters,
            visualizer,
            global_tempo,
            _follow_host_tempo: follow_host_tempo,
            metronome_enabled,
            metronome_count_in_ticks,
            metronome_count_in_playback,
            metronome_count_in_record,
            async_executor,
            pending_project_params,
            slint_window,
            ui,
            waveform_model,
            oscilloscope_model,
            spectrum_model,
            vectorscope_x_model,
            vectorscope_y_model,
            video_frame_cache,
            sample_dialog_rx,
            project_dialog_rx,
            sb_surface,
            _sb_context: sb_context,
            physical_width,
            physical_height,
            scale_factor,
            pixel_buffer: vec![PremultipliedRgbaColor::default(); (physical_width * physical_height) as usize],
            last_cursor: LogicalPosition::new(0.0, 0.0),
            _library_folders: library_folders,
            current_path,
            _library_folders_model: library_folders_model,
            current_folder_content_model,
            _animate_library: animate_library,
        }
    }

    fn dispatch_slint_event(&self, event: WindowEvent) {
        self.slint_window.dispatch_event(event);
    }

    #[allow(dead_code)]
    fn refresh_browser(&self) {
        refresh_browser_impl(
            &self.ui,
            &self.current_path.lock(),
            &self.current_folder_content_model,
        );
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
        let master_filter = self.params.master_filter.unmodulated_normalized_value();
        let master_comp = self.params.master_comp.unmodulated_normalized_value();
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
        let mosaic_spatial =
            f32::from_bits(self.tracks[track_idx].mosaic_spatial.load(Ordering::Relaxed));
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
        let ring_waves =
            f32::from_bits(self.tracks[track_idx].ring_waves.load(Ordering::Relaxed));
        let ring_waves_rate =
            f32::from_bits(self.tracks[track_idx].ring_waves_rate.load(Ordering::Relaxed));
        let ring_waves_rate_mode =
            self.tracks[track_idx].ring_waves_rate_mode.load(Ordering::Relaxed);
        let ring_noise =
            f32::from_bits(self.tracks[track_idx].ring_noise.load(Ordering::Relaxed));
        let ring_noise_rate =
            f32::from_bits(self.tracks[track_idx].ring_noise_rate.load(Ordering::Relaxed));
        let ring_noise_rate_mode =
            self.tracks[track_idx].ring_noise_rate_mode.load(Ordering::Relaxed);
        let ring_scale =
            self.tracks[track_idx].ring_scale.load(Ordering::Relaxed);
        let loop_start =
            f32::from_bits(self.tracks[track_idx].loop_start.load(Ordering::Relaxed));
        let trigger_start =
            f32::from_bits(self.tracks[track_idx].trigger_start.load(Ordering::Relaxed));
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
        let active_engine_type = self.tracks[track_idx].engine_type.load(Ordering::Relaxed);

        let animate_slot_types = [
            self.tracks[track_idx].animate_slot_types[0].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_types[1].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_types[2].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_types[3].load(Ordering::Relaxed),
        ];
        let animate_slot_wavetables = [
            self.tracks[track_idx].animate_slot_wavetables[0].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_wavetables[1].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_wavetables[2].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_wavetables[3].load(Ordering::Relaxed),
        ];
        let animate_slot_samples = [
            self.tracks[track_idx].animate_slot_samples[0].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_samples[1].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_samples[2].load(Ordering::Relaxed),
            self.tracks[track_idx].animate_slot_samples[3].load(Ordering::Relaxed),
        ];
        let animate_slot_coarse = [
            f32::from_bits(self.tracks[track_idx].animate_slot_coarse[0].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_coarse[1].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_coarse[2].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_coarse[3].load(Ordering::Relaxed)),
        ];
        let animate_slot_fine = [
            f32::from_bits(self.tracks[track_idx].animate_slot_fine[0].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_fine[1].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_fine[2].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_fine[3].load(Ordering::Relaxed)),
        ];
        let animate_slot_level = [
            f32::from_bits(self.tracks[track_idx].animate_slot_level[0].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_level[1].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_level[2].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_level[3].load(Ordering::Relaxed)),
        ];
        let animate_slot_pan = [
            f32::from_bits(self.tracks[track_idx].animate_slot_pan[0].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_pan[1].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_pan[2].load(Ordering::Relaxed)),
            f32::from_bits(self.tracks[track_idx].animate_slot_pan[3].load(Ordering::Relaxed)),
        ];
        let animate_slot_wt_lfo_amount = [
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_amount[0]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_amount[1]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_amount[2]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_amount[3]
                    .load(Ordering::Relaxed),
            ),
        ];
        let animate_slot_wt_lfo_shape = [
            self.tracks[track_idx]
                .animate_slot_wt_lfo_shape[0]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_shape[1]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_shape[2]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_shape[3]
                .load(Ordering::Relaxed),
        ];
        let animate_slot_wt_lfo_rate = [
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_rate[0]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_rate[1]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_rate[2]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_wt_lfo_rate[3]
                    .load(Ordering::Relaxed),
            ),
        ];
        let animate_slot_wt_lfo_sync = [
            self.tracks[track_idx]
                .animate_slot_wt_lfo_sync[0]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_sync[1]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_sync[2]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_sync[3]
                .load(Ordering::Relaxed),
        ];
        let animate_slot_wt_lfo_division = [
            self.tracks[track_idx]
                .animate_slot_wt_lfo_division[0]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_division[1]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_division[2]
                .load(Ordering::Relaxed),
            self.tracks[track_idx]
                .animate_slot_wt_lfo_division[3]
                .load(Ordering::Relaxed),
        ];
        let animate_slot_sample_start = [
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_sample_start[0]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_sample_start[1]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_sample_start[2]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_sample_start[3]
                    .load(Ordering::Relaxed),
            ),
        ];
        let animate_slot_loop_start = [
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_start[0]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_start[1]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_start[2]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_start[3]
                    .load(Ordering::Relaxed),
            ),
        ];
        let animate_slot_loop_end = [
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_end[0]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_end[1]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_end[2]
                    .load(Ordering::Relaxed),
            ),
            f32::from_bits(
                self.tracks[track_idx]
                    .animate_slot_loop_end[3]
                    .load(Ordering::Relaxed),
            ),
        ];
        let animate_slot_a_filter_type =
            self.tracks[track_idx].animate_slot_filter_type[0].load(Ordering::Relaxed);
        let animate_slot_a_filter_cutoff = f32::from_bits(
            self.tracks[track_idx]
                .animate_slot_filter_cutoff[0]
                .load(Ordering::Relaxed),
        );
        let animate_slot_a_filter_resonance = f32::from_bits(
            self.tracks[track_idx]
                .animate_slot_filter_resonance[0]
                .load(Ordering::Relaxed),
        );
        let animate_vector_x =
            f32::from_bits(self.tracks[track_idx].animate_vector_x.load(Ordering::Relaxed));
        let animate_vector_y =
            f32::from_bits(self.tracks[track_idx].animate_vector_y.load(Ordering::Relaxed));
        let animate_lfo_x_waveform =
            self.tracks[track_idx].animate_lfo_x_waveform.load(Ordering::Relaxed);
        let animate_lfo_x_sync =
            self.tracks[track_idx].animate_lfo_x_sync.load(Ordering::Relaxed);
        let animate_lfo_x_division =
            self.tracks[track_idx].animate_lfo_x_division.load(Ordering::Relaxed);
        let animate_lfo_x_rate =
            f32::from_bits(self.tracks[track_idx].animate_lfo_x_rate.load(Ordering::Relaxed));
        let animate_lfo_x_amount =
            f32::from_bits(self.tracks[track_idx].animate_lfo_x_amount.load(Ordering::Relaxed));
        let animate_lfo_y_waveform =
            self.tracks[track_idx].animate_lfo_y_waveform.load(Ordering::Relaxed);
        let animate_lfo_y_sync =
            self.tracks[track_idx].animate_lfo_y_sync.load(Ordering::Relaxed);
        let animate_lfo_y_division =
            self.tracks[track_idx].animate_lfo_y_division.load(Ordering::Relaxed);
        let animate_lfo_y_rate =
            f32::from_bits(self.tracks[track_idx].animate_lfo_y_rate.load(Ordering::Relaxed));
        let animate_lfo_y_amount =
            f32::from_bits(self.tracks[track_idx].animate_lfo_y_amount.load(Ordering::Relaxed));
        let animate_sequencer_current_step =
            self.tracks[track_idx].animate_sequencer_step.load(Ordering::Relaxed);

        let mut animate_sequencer_grid = Vec::with_capacity(160);
        for i in 0..160 {
            animate_sequencer_grid
                .push(self.tracks[track_idx].animate_sequencer_grid[i].load(Ordering::Relaxed));
        }

        let kick_pitch =
            f32::from_bits(self.tracks[track_idx].kick_pitch.load(Ordering::Relaxed));
        let kick_decay =
            f32::from_bits(self.tracks[track_idx].kick_decay.load(Ordering::Relaxed));
        let kick_attack =
            f32::from_bits(self.tracks[track_idx].kick_attack.load(Ordering::Relaxed));
        let kick_pitch_env_amount =
            f32::from_bits(self.tracks[track_idx].kick_pitch_env_amount.load(Ordering::Relaxed));
        let kick_drive =
            f32::from_bits(self.tracks[track_idx].kick_drive.load(Ordering::Relaxed));
        let kick_level =
            f32::from_bits(self.tracks[track_idx].kick_level.load(Ordering::Relaxed));
        let kick_filter_type =
            self.tracks[track_idx].kick_filter_type.load(Ordering::Relaxed);
        let kick_filter_cutoff =
            f32::from_bits(self.tracks[track_idx].kick_filter_cutoff.load(Ordering::Relaxed));
        let kick_filter_resonance =
            f32::from_bits(self.tracks[track_idx].kick_filter_resonance.load(Ordering::Relaxed));
        let kick_filter_pre_drive =
            self.tracks[track_idx].kick_filter_pre_drive.load(Ordering::Relaxed);
        let kick_sequencer_current_step =
            self.tracks[track_idx].kick_sequencer_step.load(Ordering::Relaxed);
        let mut kick_sequencer_grid = Vec::with_capacity(SYNDRM_STEPS);
        for i in 0..SYNDRM_STEPS {
            kick_sequencer_grid
                .push(self.tracks[track_idx].kick_sequencer_grid[i].load(Ordering::Relaxed));
        }
        let snare_tone =
            f32::from_bits(self.tracks[track_idx].snare_tone.load(Ordering::Relaxed));
        let snare_decay =
            f32::from_bits(self.tracks[track_idx].snare_decay.load(Ordering::Relaxed));
        let snare_snappy =
            f32::from_bits(self.tracks[track_idx].snare_snappy.load(Ordering::Relaxed));
        let snare_attack =
            f32::from_bits(self.tracks[track_idx].snare_attack.load(Ordering::Relaxed));
        let snare_drive =
            f32::from_bits(self.tracks[track_idx].snare_drive.load(Ordering::Relaxed));
        let snare_level =
            f32::from_bits(self.tracks[track_idx].snare_level.load(Ordering::Relaxed));
        let snare_filter_type =
            self.tracks[track_idx].snare_filter_type.load(Ordering::Relaxed);
        let snare_filter_cutoff =
            f32::from_bits(self.tracks[track_idx].snare_filter_cutoff.load(Ordering::Relaxed));
        let snare_filter_resonance =
            f32::from_bits(self.tracks[track_idx].snare_filter_resonance.load(Ordering::Relaxed));
        let snare_filter_pre_drive =
            self.tracks[track_idx].snare_filter_pre_drive.load(Ordering::Relaxed);
        let snare_sequencer_current_step =
            self.tracks[track_idx].snare_sequencer_step.load(Ordering::Relaxed);
        let mut snare_sequencer_grid = Vec::with_capacity(SYNDRM_STEPS);
        for i in 0..SYNDRM_STEPS {
            snare_sequencer_grid
                .push(self.tracks[track_idx].snare_sequencer_grid[i].load(Ordering::Relaxed));
        }
        let syndrm_page = self.tracks[track_idx].syndrm_page.load(Ordering::Relaxed) as i32;
        let syndrm_edit_lane = self.tracks[track_idx].syndrm_edit_lane.load(Ordering::Relaxed) as i32;
        let syndrm_edit_step = self.tracks[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as i32;
        let syndrm_step_hold = self.tracks[track_idx].syndrm_step_hold.load(Ordering::Relaxed);
        let edit_step = syndrm_edit_step.clamp(0, (SYNDRM_STEPS - 1) as i32) as usize;
        let syndrm_step_override = if syndrm_edit_lane == 0 {
            self.tracks[track_idx].kick_step_override_enabled[edit_step].load(Ordering::Relaxed)
        } else {
            self.tracks[track_idx].snare_step_override_enabled[edit_step].load(Ordering::Relaxed)
        };
        let syndrm_step_kick_pitch =
            f32::from_bits(self.tracks[track_idx].kick_step_pitch[edit_step].load(Ordering::Relaxed));
        let syndrm_step_kick_decay =
            f32::from_bits(self.tracks[track_idx].kick_step_decay[edit_step].load(Ordering::Relaxed));
        let syndrm_step_kick_attack =
            f32::from_bits(self.tracks[track_idx].kick_step_attack[edit_step].load(Ordering::Relaxed));
        let syndrm_step_kick_drive =
            f32::from_bits(self.tracks[track_idx].kick_step_drive[edit_step].load(Ordering::Relaxed));
        let syndrm_step_kick_level =
            f32::from_bits(self.tracks[track_idx].kick_step_level[edit_step].load(Ordering::Relaxed));
        let syndrm_step_kick_filter_type =
            self.tracks[track_idx].kick_step_filter_type[edit_step].load(Ordering::Relaxed);
        let syndrm_step_kick_filter_cutoff =
            f32::from_bits(self.tracks[track_idx].kick_step_filter_cutoff[edit_step].load(Ordering::Relaxed));
        let syndrm_step_kick_filter_resonance =
            f32::from_bits(self.tracks[track_idx].kick_step_filter_resonance[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_tone =
            f32::from_bits(self.tracks[track_idx].snare_step_tone[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_decay =
            f32::from_bits(self.tracks[track_idx].snare_step_decay[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_snappy =
            f32::from_bits(self.tracks[track_idx].snare_step_snappy[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_attack =
            f32::from_bits(self.tracks[track_idx].snare_step_attack[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_drive =
            f32::from_bits(self.tracks[track_idx].snare_step_drive[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_level =
            f32::from_bits(self.tracks[track_idx].snare_step_level[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_filter_type =
            self.tracks[track_idx].snare_step_filter_type[edit_step].load(Ordering::Relaxed);
        let syndrm_step_snare_filter_cutoff =
            f32::from_bits(self.tracks[track_idx].snare_step_filter_cutoff[edit_step].load(Ordering::Relaxed));
        let syndrm_step_snare_filter_resonance =
            f32::from_bits(self.tracks[track_idx].snare_step_filter_resonance[edit_step].load(Ordering::Relaxed));

        let void_base_freq = f32::from_bits(self.tracks[track_idx].void_base_freq.load(Ordering::Relaxed));
        let void_chaos_depth = f32::from_bits(self.tracks[track_idx].void_chaos_depth.load(Ordering::Relaxed));
        let void_entropy = f32::from_bits(self.tracks[track_idx].void_entropy.load(Ordering::Relaxed));
        let void_feedback = f32::from_bits(self.tracks[track_idx].void_feedback.load(Ordering::Relaxed));
        let void_diffusion = f32::from_bits(self.tracks[track_idx].void_diffusion.load(Ordering::Relaxed));
        let void_mod_rate = f32::from_bits(self.tracks[track_idx].void_mod_rate.load(Ordering::Relaxed));
        let void_level = f32::from_bits(self.tracks[track_idx].void_level.load(Ordering::Relaxed));
        let void_enabled = self.tracks[track_idx].void_enabled.load(Ordering::Relaxed);

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

        let mut tape_video_enabled = self.tracks[track_idx].video_enabled.load(Ordering::Relaxed);
        let mut tape_video_frame = Image::default();
        let mut tape_video_duration = duration_secs;
        if tape_video_enabled {
            if let Some(cache_guard) = self.tracks[track_idx].video_cache.try_lock() {
                if let Some(cache) = cache_guard.as_ref() {
                    if cache.frames.is_empty() || cache.width == 0 || cache.height == 0 {
                        tape_video_enabled = false;
                    } else {
                        if let Some(last_frame) = cache.frames.last() {
                            tape_video_duration = last_frame.timestamp.max(0.0);
                        }
                        let fps = f32::from_bits(
                            self.tracks[track_idx].video_fps.load(Ordering::Relaxed),
                        )
                        .max(1.0);
                        let time_secs = if sample_rate > 0 {
                            (play_pos / sample_rate as f32).max(0.0)
                        } else {
                            0.0
                        };
                        let mut frame_idx = (time_secs * fps).floor() as usize;
                        if frame_idx >= cache.frames.len() {
                            frame_idx = cache.frames.len().saturating_sub(1);
                        }

                        let cache_id =
                            self.tracks[track_idx].video_cache_id.load(Ordering::Relaxed);
                        let mut used_cache = false;
                        if let Some((cached_id, cached_idx, cached_image)) =
                            &self.video_frame_cache[track_idx]
                        {
                            if *cached_id == cache_id && *cached_idx == frame_idx {
                                tape_video_frame = cached_image.clone();
                                used_cache = true;
                            }
                        }

                        if !used_cache {
                            let frame = &cache.frames[frame_idx];
                            let image = image_from_rgba(cache.width, cache.height, &frame.data);
                            tape_video_frame = image.clone();
                            self.video_frame_cache[track_idx] =
                                Some((cache_id, frame_idx, image));
                        }
                    }
                } else {
                    tape_video_enabled = false;
                }
            } else {
                tape_video_enabled = false;
            }
        }

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
        self.ui.set_master_filter(master_filter);
        self.ui.set_master_comp(master_comp);
        self.ui.set_track_level(track_level);
        self.ui.set_track_muted(track_muted);
        self.ui.set_meter_left(meter_left);
        self.ui.set_meter_right(meter_right);
        self.ui.set_track_meter_left(track_meter_left);
        self.ui.set_track_meter_right(track_meter_right);
        self.ui.set_tape_video_enabled(tape_video_enabled);
        self.ui.set_tape_video_duration(tape_video_duration);
        self.ui.set_tape_video_frame(tape_video_frame);
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
        self.ui.set_mosaic_spatial(mosaic_spatial);
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
        self.ui.set_ring_waves(ring_waves);
        self.ui.set_ring_waves_rate(ring_waves_rate);
        self.ui.set_ring_waves_rate_mode(ring_waves_rate_mode as i32);
        self.ui.set_ring_noise(ring_noise);
        self.ui.set_ring_noise_rate(ring_noise_rate);
        self.ui.set_ring_noise_rate_mode(ring_noise_rate_mode as i32);
        self.ui.set_ring_scale(ring_scale as i32);
        self.ui.set_loop_start(loop_start);
        self.ui.set_trigger_start(trigger_start);
        self.ui.set_loop_length(loop_length);
        self.ui.set_loop_xfade(loop_xfade);
        self.ui.set_loop_enabled(loop_enabled);
        self.ui.set_loop_mode(loop_mode as i32);
        self.ui.set_mosaic_enabled(mosaic_enabled);
        self.ui.set_ring_enabled(ring_enabled);
        self.ui.set_ring_decay_mode(ring_decay_mode as i32);
        self.ui.set_engine_loaded(engine_loaded);
        self.ui.set_active_engine_type(active_engine_type as i32);

        self.ui.set_animate_slot_a_type(animate_slot_types[0] as i32);
        self.ui.set_animate_slot_b_type(animate_slot_types[1] as i32);
        self.ui.set_animate_slot_c_type(animate_slot_types[2] as i32);
        self.ui.set_animate_slot_d_type(animate_slot_types[3] as i32);

        self.ui.set_animate_slot_a_wavetable(animate_slot_wavetables[0] as i32);
        self.ui.set_animate_slot_b_wavetable(animate_slot_wavetables[1] as i32);
        self.ui.set_animate_slot_c_wavetable(animate_slot_wavetables[2] as i32);
        self.ui.set_animate_slot_d_wavetable(animate_slot_wavetables[3] as i32);

        self.ui.set_animate_slot_a_sample(animate_slot_samples[0] as i32);
        self.ui.set_animate_slot_b_sample(animate_slot_samples[1] as i32);
        self.ui.set_animate_slot_c_sample(animate_slot_samples[2] as i32);
        self.ui.set_animate_slot_d_sample(animate_slot_samples[3] as i32);

        self.ui.set_animate_slot_a_coarse(animate_slot_coarse[0]);
        self.ui.set_animate_slot_a_fine(animate_slot_fine[0]);
        self.ui.set_animate_slot_a_level(animate_slot_level[0]);
        self.ui.set_animate_slot_a_pan(animate_slot_pan[0]);

        self.ui.set_animate_slot_b_coarse(animate_slot_coarse[1]);
        self.ui.set_animate_slot_b_fine(animate_slot_fine[1]);
        self.ui.set_animate_slot_b_level(animate_slot_level[1]);
        self.ui.set_animate_slot_b_pan(animate_slot_pan[1]);

        self.ui.set_animate_slot_c_coarse(animate_slot_coarse[2]);
        self.ui.set_animate_slot_c_fine(animate_slot_fine[2]);
        self.ui.set_animate_slot_c_level(animate_slot_level[2]);
        self.ui.set_animate_slot_c_pan(animate_slot_pan[2]);

        self.ui.set_animate_slot_d_coarse(animate_slot_coarse[3]);
        self.ui.set_animate_slot_d_fine(animate_slot_fine[3]);
        self.ui.set_animate_slot_d_level(animate_slot_level[3]);
        self.ui.set_animate_slot_d_pan(animate_slot_pan[3]);
        self.ui
            .set_animate_slot_a_wt_lfo_amount(animate_slot_wt_lfo_amount[0]);
        self.ui
            .set_animate_slot_a_wt_lfo_shape(animate_slot_wt_lfo_shape[0] as i32);
        self.ui
            .set_animate_slot_a_wt_lfo_rate(animate_slot_wt_lfo_rate[0]);
        self.ui
            .set_animate_slot_a_wt_lfo_sync(animate_slot_wt_lfo_sync[0]);
        self.ui
            .set_animate_slot_a_wt_lfo_division(animate_slot_wt_lfo_division[0] as i32);
        self.ui
            .set_animate_slot_b_wt_lfo_amount(animate_slot_wt_lfo_amount[1]);
        self.ui
            .set_animate_slot_b_wt_lfo_shape(animate_slot_wt_lfo_shape[1] as i32);
        self.ui
            .set_animate_slot_b_wt_lfo_rate(animate_slot_wt_lfo_rate[1]);
        self.ui
            .set_animate_slot_b_wt_lfo_sync(animate_slot_wt_lfo_sync[1]);
        self.ui
            .set_animate_slot_b_wt_lfo_division(animate_slot_wt_lfo_division[1] as i32);
        self.ui
            .set_animate_slot_c_wt_lfo_amount(animate_slot_wt_lfo_amount[2]);
        self.ui
            .set_animate_slot_c_wt_lfo_shape(animate_slot_wt_lfo_shape[2] as i32);
        self.ui
            .set_animate_slot_c_wt_lfo_rate(animate_slot_wt_lfo_rate[2]);
        self.ui
            .set_animate_slot_c_wt_lfo_sync(animate_slot_wt_lfo_sync[2]);
        self.ui
            .set_animate_slot_c_wt_lfo_division(animate_slot_wt_lfo_division[2] as i32);
        self.ui
            .set_animate_slot_d_wt_lfo_amount(animate_slot_wt_lfo_amount[3]);
        self.ui
            .set_animate_slot_d_wt_lfo_shape(animate_slot_wt_lfo_shape[3] as i32);
        self.ui
            .set_animate_slot_d_wt_lfo_rate(animate_slot_wt_lfo_rate[3]);
        self.ui
            .set_animate_slot_d_wt_lfo_sync(animate_slot_wt_lfo_sync[3]);
        self.ui
            .set_animate_slot_d_wt_lfo_division(animate_slot_wt_lfo_division[3] as i32);
        self.ui
            .set_animate_slot_a_sample_start(animate_slot_sample_start[0]);
        self.ui
            .set_animate_slot_a_loop_start(animate_slot_loop_start[0]);
        self.ui
            .set_animate_slot_a_loop_end(animate_slot_loop_end[0]);
        self.ui
            .set_animate_slot_b_sample_start(animate_slot_sample_start[1]);
        self.ui
            .set_animate_slot_b_loop_start(animate_slot_loop_start[1]);
        self.ui
            .set_animate_slot_b_loop_end(animate_slot_loop_end[1]);
        self.ui
            .set_animate_slot_c_sample_start(animate_slot_sample_start[2]);
        self.ui
            .set_animate_slot_c_loop_start(animate_slot_loop_start[2]);
        self.ui
            .set_animate_slot_c_loop_end(animate_slot_loop_end[2]);
        self.ui
            .set_animate_slot_d_sample_start(animate_slot_sample_start[3]);
        self.ui
            .set_animate_slot_d_loop_start(animate_slot_loop_start[3]);
        self.ui
            .set_animate_slot_d_loop_end(animate_slot_loop_end[3]);
        self.ui
            .set_animate_slot_a_filter_type(animate_slot_a_filter_type as i32);
        self.ui
            .set_animate_slot_a_filter_cutoff(animate_slot_a_filter_cutoff);
        self.ui
            .set_animate_slot_a_filter_resonance(animate_slot_a_filter_resonance);

        self.ui.set_animate_vector_x(animate_vector_x);
        self.ui.set_animate_vector_y(animate_vector_y);
        self.ui.set_animate_lfo_x_waveform(animate_lfo_x_waveform as i32);
        self.ui.set_animate_lfo_x_sync(animate_lfo_x_sync);
        self.ui
            .set_animate_lfo_x_division(animate_lfo_x_division as i32);
        self.ui.set_animate_lfo_x_rate(animate_lfo_x_rate);
        self.ui.set_animate_lfo_x_amount(animate_lfo_x_amount);
        self.ui.set_animate_lfo_y_waveform(animate_lfo_y_waveform as i32);
        self.ui.set_animate_lfo_y_sync(animate_lfo_y_sync);
        self.ui
            .set_animate_lfo_y_division(animate_lfo_y_division as i32);
        self.ui.set_animate_lfo_y_rate(animate_lfo_y_rate);
        self.ui.set_animate_lfo_y_amount(animate_lfo_y_amount);


        self.ui
            .set_animate_sequencer_current_step(animate_sequencer_current_step);
        self.ui
            .set_animate_sequencer_grid(ModelRc::from(std::rc::Rc::new(VecModel::from(
                animate_sequencer_grid,
            ))));

        self.ui.set_kick_pitch(kick_pitch);
        self.ui.set_kick_decay(kick_decay);
        self.ui.set_kick_attack(kick_attack);
        self.ui.set_kick_pitch_env_amount(kick_pitch_env_amount);
        self.ui.set_kick_drive(kick_drive);
        self.ui.set_kick_level(kick_level);
        self.ui.set_kick_filter_type(kick_filter_type as i32);
        self.ui.set_kick_filter_cutoff(kick_filter_cutoff);
        self.ui.set_kick_filter_resonance(kick_filter_resonance);
        self.ui.set_kick_filter_pre_drive(kick_filter_pre_drive);
        self.ui
            .set_kick_sequencer_current_step(kick_sequencer_current_step);
        self.ui
            .set_kick_sequencer_grid(ModelRc::from(std::rc::Rc::new(VecModel::from(
                kick_sequencer_grid,
            ))));
        self.ui.set_snare_tone(snare_tone);
        self.ui.set_snare_decay(snare_decay);
        self.ui.set_snare_snappy(snare_snappy);
        self.ui.set_snare_attack(snare_attack);
        self.ui.set_snare_drive(snare_drive);
        self.ui.set_snare_level(snare_level);
        self.ui.set_snare_filter_type(snare_filter_type as i32);
        self.ui.set_snare_filter_cutoff(snare_filter_cutoff);
        self.ui.set_snare_filter_resonance(snare_filter_resonance);
        self.ui.set_snare_filter_pre_drive(snare_filter_pre_drive);
        self.ui
            .set_snare_sequencer_current_step(snare_sequencer_current_step);
        self.ui
            .set_snare_sequencer_grid(ModelRc::from(std::rc::Rc::new(VecModel::from(
                snare_sequencer_grid,
            ))));
        self.ui.set_syndrm_page(syndrm_page);
        self.ui.set_syndrm_edit_lane(syndrm_edit_lane);
        self.ui.set_syndrm_edit_step(syndrm_edit_step);
        self.ui.set_syndrm_step_hold(syndrm_step_hold);
        self.ui.set_syndrm_step_override(syndrm_step_override);
        self.ui.set_syndrm_step_kick_pitch(syndrm_step_kick_pitch);
        self.ui.set_syndrm_step_kick_decay(syndrm_step_kick_decay);
        self.ui.set_syndrm_step_kick_attack(syndrm_step_kick_attack);
        self.ui.set_syndrm_step_kick_drive(syndrm_step_kick_drive);
        self.ui.set_syndrm_step_kick_level(syndrm_step_kick_level);
        self.ui.set_syndrm_step_kick_filter_type(syndrm_step_kick_filter_type as i32);
        self.ui
            .set_syndrm_step_kick_filter_cutoff(syndrm_step_kick_filter_cutoff);
        self.ui
            .set_syndrm_step_kick_filter_resonance(syndrm_step_kick_filter_resonance);
        self.ui.set_syndrm_step_snare_tone(syndrm_step_snare_tone);
        self.ui.set_syndrm_step_snare_decay(syndrm_step_snare_decay);
        self.ui.set_syndrm_step_snare_snappy(syndrm_step_snare_snappy);
        self.ui.set_syndrm_step_snare_attack(syndrm_step_snare_attack);
        self.ui.set_syndrm_step_snare_drive(syndrm_step_snare_drive);
        self.ui.set_syndrm_step_snare_level(syndrm_step_snare_level);
        self.ui
            .set_syndrm_step_snare_filter_type(syndrm_step_snare_filter_type as i32);
        self.ui
            .set_syndrm_step_snare_filter_cutoff(syndrm_step_snare_filter_cutoff);
        self.ui
            .set_syndrm_step_snare_filter_resonance(syndrm_step_snare_filter_resonance);

        self.ui.set_void_base_freq(void_base_freq);
        self.ui.set_void_chaos_depth(void_chaos_depth);
        self.ui.set_void_entropy(void_entropy);
        self.ui.set_void_feedback(void_feedback);
        self.ui.set_void_diffusion(void_diffusion);
        self.ui.set_void_mod_rate(void_mod_rate);
        self.ui.set_void_level(void_level);
        self.ui.set_void_enabled(void_enabled);

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
        // Force focus on the first frame
        static FIRST_FRAME: AtomicBool = AtomicBool::new(true);
        if FIRST_FRAME.swap(false, Ordering::Relaxed) {
            #[cfg(target_os = "windows")]
            {
                if let RawWindowHandle::Win32(handle) = _window.raw_window_handle() {
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow};
                        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus};
                        SetForegroundWindow(handle.hwnd as _);
                        SetFocus(handle.hwnd as _);
                    }
                }
            }

            self.dispatch_slint_event(WindowEvent::WindowActiveChanged(true));
        }
        if let Some(pending) = self.pending_project_params.lock().take() {
            let setter = ParamSetter::new(self.gui_context.as_ref());
            setter.begin_set_parameter(&self.params.gain);
            setter.set_parameter(&self.params.gain, pending.gain);
            setter.end_set_parameter(&self.params.gain);

            setter.begin_set_parameter(&self.params.master_filter);
            setter.set_parameter(&self.params.master_filter, pending.master_filter);
            setter.end_set_parameter(&self.params.master_filter);

            setter.begin_set_parameter(&self.params.master_comp);
            setter.set_parameter(&self.params.master_comp, pending.master_comp);
            setter.end_set_parameter(&self.params.master_comp);
        }
        while let Ok(action) = self.sample_dialog_rx.try_recv() {
            match action {
                SampleDialogAction::Load { track_idx, path } => {
                    if track_idx < NUM_TRACKS {
                        if let Some(path) = path {
                            self.async_executor
                                .execute_background(TLBX1Task::LoadSample(track_idx, path));
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
                        .execute_background(TLBX1Task::SaveProject {
                            path,
                            title: "Project".into(),
                            description: "".into(),
                        });
                }
                ProjectDialogAction::Load(path) => {
                    self.async_executor
                        .execute_background(TLBX1Task::LoadProject(path));
                }
                ProjectDialogAction::SaveWithInfo {
                    path,
                    title,
                    description,
                } => {
                    self.async_executor
                        .execute_background(TLBX1Task::SaveProject {
                            path,
                            title,
                            description,
                        });
                }
                ProjectDialogAction::ExportZip {
                    path,
                    title,
                    description,
                } => {
                    self.async_executor
                        .execute_background(TLBX1Task::ExportProjectZip {
                            path,
                            title,
                            description,
                        });
                }
            }
        }
        platform::update_timers_and_animations();
        self.update_ui_state();
        self.slint_window.request_redraw();
        self.render();
    }

    fn on_event(&mut self, _window: &mut BaseWindow, event: BaseEvent) -> BaseEventStatus {
        // Add at the very top - this WILL print even if logging is filtered
        match &event {
            BaseEvent::Keyboard(e) => {
                println!("========================================");
                println!("KEYBOARD EVENT RECEIVED!!!");
                println!("Key: {:?}", e.key);
                println!("State: {:?}", e.state);
                println!("Repeat: {}", e.repeat);
                println!("========================================");
                std::io::stdout().flush().unwrap();
            }
            _ => {}
        }
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
                        self.dispatch_slint_event(WindowEvent::WindowActiveChanged(true));
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
            BaseEvent::Keyboard(event) => {
                eprintln!("Keyboard event: {:?} state: {:?}", event.key, event.state); // Debug output
                if event.state == KeyState::Down && !event.repeat {
                    let is_escape = match &event.key {
                        Key::Escape => true,
                        Key::Character(c) if c == "\u{1b}" => true,
                        _ => false,
                    };

                    if is_escape {
                        if self.ui.get_show_settings() {
                            self.ui.set_show_settings(false);
                            return BaseEventStatus::Captured;
                        }
                        if self.ui.get_show_engine_confirm() {
                            self.ui.set_show_engine_confirm(false);
                            return BaseEventStatus::Captured;
                        }
                        if self.ui.get_show_browser() {
                            self.ui.set_show_browser(false);
                            return BaseEventStatus::Captured;
                        }
                    }
                }

                // **CRITICAL FIX**: Dispatch the event with text, not just key pressed/released
                if let Some(text) = key_to_slint_string(&event.key) {
                    let slint_event = match event.state {
                        KeyState::Down => {
                            WindowEvent::KeyPressed {
                                text: text.clone().into()
                            }
                        }
                        KeyState::Up => {
                            WindowEvent::KeyReleased {
                                text: text.into()
                            }
                        }
                    };
                    self.dispatch_slint_event(slint_event);
                }

                let slint_event = match event.state {
                    KeyState::Down => {
                        key_to_slint_string(&event.key).map(|text| WindowEvent::KeyPressed { text: text.into() })
                    }
                    KeyState::Up => {
                        key_to_slint_string(&event.key).map(|text| WindowEvent::KeyReleased { text: text.into() })
                    }
                };

                if let Some(se) = slint_event {
                    self.dispatch_slint_event(se);
                }

                if event.state == KeyState::Down && !event.repeat {
                    match event.key {
                        Key::Character(ref ch) if ch == " " => {
                            if self.gui_context.plugin_api() == PluginApi::Standalone
                                && !self.ui.get_show_browser()
                                && !self.ui.get_show_settings()
                                && !self.ui.get_show_engine_confirm()
                            {
                                self.ui.invoke_toggle_play();
                                return BaseEventStatus::Captured;
                            }
                        }
                        _ => {}
                    }
                }
                BaseEventStatus::Captured
            }
        }
    }
}

fn spawn_with_stack<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    let _ = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(f);
}

fn initialize_ui(
    ui: &TLBX1UI,
    gui_context: &Arc<dyn GuiContext>,
    params: &Arc<TLBX1Params>,
    tracks: &Arc<[Track; NUM_TRACKS]>,
    global_tempo: &Arc<AtomicU32>,
    _follow_host_tempo: &Arc<AtomicBool>,
    metronome_enabled: &Arc<AtomicBool>,
    metronome_count_in_ticks: &Arc<AtomicU32>,
    metronome_count_in_playback: &Arc<AtomicBool>,
    metronome_count_in_record: &Arc<AtomicBool>,
    _async_executor: &AsyncExecutor<TLBX1>,
    output_devices: &[String],
    input_devices: &[String],
    sample_rates: &[u32],
    buffer_sizes: &[u32],
    sample_dialog_tx: std::sync::mpsc::Sender<SampleDialogAction>,
    project_dialog_tx: std::sync::mpsc::Sender<ProjectDialogAction>,
    library_folders: &Arc<Mutex<Vec<PathBuf>>>,
    current_path: &Arc<Mutex<PathBuf>>,
    _library_folders_model: &std::rc::Rc<VecModel<SharedString>>,
    current_folder_content_model: &std::rc::Rc<VecModel<BrowserEntry>>,
    animate_library: &Arc<AnimateLibrary>,
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
        SharedString::from("Off"),
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
    ui.set_ring_rate_modes(ModelRc::new(VecModel::from(vec![
        SharedString::from("Free"),
        SharedString::from("Straight"),
        SharedString::from("Dotted"),
        SharedString::from("Triplet"),
    ])));
    ui.set_ring_scale_modes(ModelRc::new(VecModel::from(vec![
        SharedString::from("Chromatic"),
        SharedString::from("Major"),
        SharedString::from("Minor"),
    ])));
    ui.set_syndrm_filter_types(ModelRc::new(VecModel::from(vec![
        SharedString::from("Moog LP"),
        SharedString::from("Lowpass"),
        SharedString::from("Highpass"),
        SharedString::from("Bandpass"),
    ])));
    ui.set_engine_types(ModelRc::new(VecModel::from(vec![
        SharedString::from("Tape-Deck"),
        SharedString::from("Animate"),
        SharedString::from("SynDRM"),
        SharedString::from("Void Seed"),
    ])));
    ui.set_engine_index(0);
    ui.set_engine_confirm_text(SharedString::from(
        "Loading a new engine will clear unsaved data for this track. Continue?",
    ));

    ui.set_animate_slot_types(ModelRc::new(VecModel::from(vec![
        SharedString::from("Wavetable"),
        SharedString::from("Sample"),
    ])));

    ui.set_animate_filter_types(ModelRc::new(VecModel::from(vec![
        SharedString::from("Lowpass 24dB"),
        SharedString::from("Lowpass 12dB"),
        SharedString::from("Highpass"),
        SharedString::from("Bandpass"),
    ])));
    ui.set_animate_lfo_waveforms(ModelRc::new(VecModel::from(vec![
        SharedString::from("Sine"),
        SharedString::from("Triangle"),
        SharedString::from("Square"),
        SharedString::from("Saw"),
        SharedString::from("Sample & Hold"),
    ])));
    ui.set_animate_lfo_divisions(ModelRc::new(VecModel::from(vec![
        SharedString::from("1/16"),
        SharedString::from("1/8"),
        SharedString::from("1/4"),
        SharedString::from("1/3"),
        SharedString::from("1/2"),
        SharedString::from("1"),
        SharedString::from("2"),
        SharedString::from("4"),
    ])));

    // Scan for wavetables and samples
    let mut wavetables = Vec::new();
    if let Ok(entries) = std::fs::read_dir("src/library/factory/wavetables") {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Ok(subentries) = std::fs::read_dir(entry.path()) {
                    for subentry in subentries.flatten() {
                        if subentry.path().extension().map_or(false, |ext| ext == "wav") {
                            let label = format!("{}/{}", 
                                entry.file_name().to_string_lossy(),
                                subentry.file_name().to_string_lossy());
                            wavetables.push(SharedString::from(label));
                        }
                    }
                }
            } else if entry.path().extension().map_or(false, |ext| ext == "wav") {
                wavetables.push(SharedString::from(entry.file_name().to_string_lossy().to_string()));
            }
        }
    }
    ui.set_animate_wavetables(ModelRc::new(VecModel::from(wavetables)));

    let mut samples = Vec::new();
    if let Ok(entries) = std::fs::read_dir("src/library/factory/samples") {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Ok(subentries) = std::fs::read_dir(entry.path()) {
                    for subentry in subentries.flatten() {
                        if subentry.path().extension().map_or(false, |ext| ext == "wav" || ext == "mp3") {
                            let label = format!("{}/{}", 
                                entry.file_name().to_string_lossy(),
                                subentry.file_name().to_string_lossy());
                            samples.push(SharedString::from(label));
                        }
                    }
                }
            } else if entry.path().extension().map_or(false, |ext| ext == "wav" || ext == "mp3") {
                samples.push(SharedString::from(entry.file_name().to_string_lossy().to_string()));
            }
        }
    }
    ui.set_animate_samples(ModelRc::new(VecModel::from(samples)));

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

    let ui_toggle = ui_weak.clone();
    ui.on_toggle_browser(move || {
        if let Some(ui) = ui_toggle.upgrade() {
            ui.set_show_browser(!ui.get_show_browser());
        }
    });

    let ui_add_folder = ui_weak.clone();
    let library_folders_add = library_folders.clone();
    ui.on_add_library_folder(move || {
        let ui_weak = ui_add_folder.clone();
        let library_folders = library_folders_add.clone();
        spawn_with_stack(move || {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        let mut folders = library_folders.lock();
                        if !folders.contains(&path) {
                            folders.push(path);
                            let folder_strings: Vec<SharedString> = folders
                                .iter()
                                .map(|p| p.to_string_lossy().to_string().into())
                                .collect();
                            ui.set_library_folders(ModelRc::new(VecModel::from(folder_strings)));
                        }
                    }
                })
                .unwrap();
            }
        });
    });

    let ui_select_folder = ui_weak.clone();
    let library_folders_select = library_folders.clone();
    let current_path_select = current_path.clone();
    let current_folder_content_model_select = current_folder_content_model.clone();
    ui.on_select_library_folder(move |index| {
        let folders = library_folders_select.lock();
        if let Some(path) = folders.get(index as usize) {
            *current_path_select.lock() = path.to_path_buf();
            if let Some(ui) = ui_select_folder.upgrade() {
                refresh_browser_impl(&ui, path, &current_folder_content_model_select);
            }
        }
    });

    let ui_open_entry = ui_weak.clone();
    let current_path_open = current_path.clone();
    let current_folder_content_model_open = current_folder_content_model.clone();
    let project_dialog_tx_open = project_dialog_tx.clone();
    ui.on_open_browser_entry(move |entry| {
        let path = PathBuf::from(entry.path.as_str());
        if entry.is_dir {
            *current_path_open.lock() = path.clone();
            if let Some(ui) = ui_open_entry.upgrade() {
                refresh_browser_impl(&ui, &path, &current_folder_content_model_open);
            }
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "tlbx" {
                let _ = project_dialog_tx_open.send(ProjectDialogAction::Load(path));
                if let Some(ui) = ui_open_entry.upgrade() {
                    ui.set_show_browser(false);
                }
            }
        }
    });

    let project_dialog_tx_save = project_dialog_tx.clone();
    ui.on_save_project_data(move |title, description| {
        let title = title.to_string();
        let description = description.to_string();
        let project_dialog_tx = project_dialog_tx_save.clone();
        spawn_with_stack(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Select Project Folder")
                .pick_folder()
            {
                let _ = project_dialog_tx.send(ProjectDialogAction::SaveWithInfo {
                    path,
                    title,
                    description,
                });
            }
        });
    });

    let project_dialog_tx_export = project_dialog_tx.clone();
    ui.on_export_project_data(move |title, description| {
        let title = title.to_string();
        let description = description.to_string();
        let project_dialog_tx = project_dialog_tx_export.clone();
        spawn_with_stack(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Zip Archive", &["zip"])
                .save_file()
            {
                let _ = project_dialog_tx.send(ProjectDialogAction::ExportZip {
                    path,
                    title,
                    description,
                });
            }
        });
    });

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
                1 => 2,
                2 => 3,
                3 => 4,
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

    let gui_context_filter = Arc::clone(gui_context);
    let params_filter = Arc::clone(params);
    ui.on_master_filter_changed(move |value| {
        let setter = ParamSetter::new(gui_context_filter.as_ref());
        setter.begin_set_parameter(&params_filter.master_filter);
        setter.set_parameter_normalized(&params_filter.master_filter, value);
        setter.end_set_parameter(&params_filter.master_filter);
    });

    let gui_context_comp = Arc::clone(gui_context);
    let params_comp = Arc::clone(params);
    ui.on_master_comp_changed(move |value| {
        let setter = ParamSetter::new(gui_context_comp.as_ref());
        setter.begin_set_parameter(&params_comp.master_comp);
        setter.set_parameter_normalized(&params_comp.master_comp, value);
        setter.end_set_parameter(&params_comp.master_comp);
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
            let reverse_active = track.tape_reverse.load(Ordering::Relaxed) || loop_mode == 3;
            let loop_start = if loop_enabled {
                if let Some(samples) = track.samples.try_lock() {
                    let len = samples.get(0).map(|ch| ch.len()).unwrap_or(0);
                    let rotate_offset = (rotate_norm * len as f32) as usize;
                    if reverse_active {
                        let base_end = ((1.0 - loop_start_norm) * len as f32) as usize;
                        let loop_end = (base_end + rotate_offset).min(len);
                        let mut loop_len =
                            (f32::from_bits(track.loop_length.load(Ordering::Relaxed)) * len as f32)
                                as usize;
                        if loop_len == 0 {
                            loop_len = loop_end.max(1);
                        }
                        loop_end.saturating_sub(loop_len) as f32
                    } else {
                        let base_start = (loop_start_norm * len as f32) as usize;
                        ((base_start + rotate_offset) % len.max(1)) as f32
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            };
            let trigger_start_norm =
                f32::from_bits(track.trigger_start.load(Ordering::Relaxed)).clamp(0.0, 0.999);
            let trigger_start = if let Some(samples) = track.samples.try_lock() {
                let len = samples.get(0).map(|ch| ch.len()).unwrap_or(0);
                let start_norm = if reverse_active {
                    (1.0 - trigger_start_norm).clamp(0.0, 0.999)
                } else {
                    trigger_start_norm
                };
                (start_norm * len as f32) as f32
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
                        track.play_pos.store(trigger_start.to_bits(), Ordering::Relaxed);
                    }
                } else {
                    track.play_pos.store(trigger_start.to_bits(), Ordering::Relaxed);
                }
            } else {
                track.play_pos.store(trigger_start.to_bits(), Ordering::Relaxed);
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

    let tracks_audition = Arc::clone(tracks);
    let params_audition = Arc::clone(params);
    ui.on_audition_start({
        let tracks = Arc::clone(&tracks_audition);
        let params = Arc::clone(&params_audition);
        move || {
            let track_idx = params.selected_track.value().saturating_sub(1) as usize;
            if track_idx >= NUM_TRACKS {
                return;
            }
            let track = &tracks[track_idx];

            let loop_enabled = track.loop_enabled.load(Ordering::Relaxed);
            let loop_mode = track.loop_mode.load(Ordering::Relaxed);
            let loop_start_norm =
                f32::from_bits(track.loop_start.load(Ordering::Relaxed)).clamp(0.0, 0.999);
            let rotate_norm =
                f32::from_bits(track.tape_rotate.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let reverse_active = track.tape_reverse.load(Ordering::Relaxed) || loop_mode == 3;

            let loop_start = if loop_enabled {
                if let Some(samples) = track.samples.try_lock() {
                    let len = samples.get(0).map(|ch| ch.len()).unwrap_or(0);
                    let rotate_offset = (rotate_norm * len as f32) as usize;
                    if reverse_active {
                        let base_end = ((1.0 - loop_start_norm) * len as f32) as usize;
                        let loop_end = (base_end + rotate_offset).min(len);
                        let mut loop_len =
                            (f32::from_bits(track.loop_length.load(Ordering::Relaxed)) * len as f32)
                                as usize;
                        if loop_len == 0 {
                            loop_len = loop_end.max(1);
                        }
                        loop_end.saturating_sub(loop_len) as f32
                    } else {
                        let base_start = (loop_start_norm * len as f32) as usize;
                        ((base_start + rotate_offset) % len.max(1)) as f32
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let trigger_start_norm =
                f32::from_bits(track.trigger_start.load(Ordering::Relaxed)).clamp(0.0, 0.999);
            let trigger_start = if let Some(samples) = track.samples.try_lock() {
                let len = samples.get(0).map(|ch| ch.len()).unwrap_or(0);
                let start_norm = if reverse_active {
                    (1.0 - trigger_start_norm).clamp(0.0, 0.999)
                } else {
                    trigger_start_norm
                };
                (start_norm * len as f32) as f32
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
                        track.play_pos.store(trigger_start.to_bits(), Ordering::Relaxed);
                    }
                } else {
                    track.play_pos.store(trigger_start.to_bits(), Ordering::Relaxed);
                }
            } else {
                track.play_pos.store(trigger_start.to_bits(), Ordering::Relaxed);
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

            track.pending_play.store(false, Ordering::Relaxed);
            track.count_in_remaining.store(0, Ordering::Relaxed);
            track.is_playing.store(true, Ordering::Relaxed);
        }
    });

    ui.on_audition_end({
        let tracks = Arc::clone(&tracks_audition);
        let params = Arc::clone(&params_audition);
        move || {
            let track_idx = params.selected_track.value().saturating_sub(1) as usize;
            if track_idx < NUM_TRACKS {
                tracks[track_idx].is_playing.store(false, Ordering::Relaxed);
                tracks[track_idx].pending_play.store(false, Ordering::Relaxed);
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
        spawn_with_stack(move || {
            let path = rfd::FileDialog::new()
                .add_filter(
                    "Media",
                    &[
                        "wav", "flac", "mp3", "ogg", "aif", "aiff", "m4a", "mp4", "mov", "mkv",
                        "avi", "webm", "m4v",
                    ],
                )
                .add_filter("All Files", &["*"])
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
        spawn_with_stack(move || {
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
    let tracks_mute_for = Arc::clone(tracks);
    ui.on_toggle_track_mute_for(move |track: i32| {
        let track_idx = track.max(1) as usize - 1;
        if track_idx < NUM_TRACKS {
            let muted = tracks_mute_for[track_idx].is_muted.load(Ordering::Relaxed);
            tracks_mute_for[track_idx]
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

    let tracks_trigger = Arc::clone(tracks);
    let params_trigger = Arc::clone(params);
    ui.on_trigger_start_changed(move |value| {
        let track_idx = params_trigger.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_trigger[track_idx]
                .trigger_start
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
            tracks_tape[track_idx]
                .tape_sync_requested
                .store(true, Ordering::Relaxed);
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
            tracks_tape[track_idx]
                .tape_sync_requested
                .store(true, Ordering::Relaxed);
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
    ui.on_mosaic_spatial_changed(move |value| {
        let track_idx = params_mosaic.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_mosaic[track_idx]
                .mosaic_spatial
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
    ui.on_ring_waves_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_waves
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_waves_rate_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_waves_rate
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_noise_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_noise
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_ring = Arc::clone(tracks);
    let params_ring = Arc::clone(params);
    ui.on_ring_noise_rate_changed(move |value| {
        let track_idx = params_ring.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_ring[track_idx]
                .ring_noise_rate
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
            // Request focus when opening
            if ui.get_show_settings() {
                ui.window().request_redraw();
            }
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
            spawn_with_stack(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("TLBX-1 Project", &["json"])
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
            spawn_with_stack(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("TLBX-1 Project", &["json"])
                    .pick_file();
                if let Some(path) = path {
                    let _ = project_dialog_tx.send(ProjectDialogAction::Load(path));
                }
            });
        }
    });

    // Animate Engine Callbacks
    for i in 0..4 {
        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        let slot_idx = i;
        match i {
            0 => ui.on_animate_slot_a_type_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_types[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_type_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_types[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_type_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_types[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_type_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_types[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        let animate_library_a = Arc::clone(&animate_library);
        let animate_library_b = Arc::clone(&animate_library);
        let animate_library_c = Arc::clone(&animate_library);
        let animate_library_d = Arc::clone(&animate_library);
        match i {
            0 => ui.on_animate_slot_a_wavetable_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wavetables[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_a.ensure_wavetable_loaded(index as usize);
            }),
            1 => ui.on_animate_slot_b_wavetable_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wavetables[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_b.ensure_wavetable_loaded(index as usize);
            }),
            2 => ui.on_animate_slot_c_wavetable_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wavetables[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_c.ensure_wavetable_loaded(index as usize);
            }),
            3 => ui.on_animate_slot_d_wavetable_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wavetables[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_d.ensure_wavetable_loaded(index as usize);
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        let animate_library_a = Arc::clone(&animate_library);
        let animate_library_b = Arc::clone(&animate_library);
        let animate_library_c = Arc::clone(&animate_library);
        let animate_library_d = Arc::clone(&animate_library);
        match i {
            0 => ui.on_animate_slot_a_sample_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_samples[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_a.ensure_sample_loaded(index as usize);
            }),
            1 => ui.on_animate_slot_b_sample_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_samples[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_b.ensure_sample_loaded(index as usize);
            }),
            2 => ui.on_animate_slot_c_sample_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_samples[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_c.ensure_sample_loaded(index as usize);
            }),
            3 => ui.on_animate_slot_d_sample_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_samples[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
                animate_library_d.ensure_sample_loaded(index as usize);
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_coarse_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_coarse[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_coarse_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_coarse[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_coarse_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_coarse[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_coarse_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_coarse[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_fine_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_fine[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_fine_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_fine[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_fine_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_fine[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_fine_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_fine[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_level_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_level[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_level_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_level[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_level_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_level[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_level_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_level[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_pan_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_pan[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_pan_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_pan[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_pan_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_pan[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_pan_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_pan[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_wt_lfo_amount_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_amount[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_wt_lfo_amount_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_amount[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_wt_lfo_amount_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_amount[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_wt_lfo_amount_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_amount[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_wt_lfo_shape_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_shape[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_wt_lfo_shape_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_shape[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_wt_lfo_shape_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_shape[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_wt_lfo_shape_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_shape[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_wt_lfo_rate_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_rate[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_wt_lfo_rate_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_rate[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_wt_lfo_rate_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_rate[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_wt_lfo_rate_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_rate[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_wt_lfo_sync_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_sync[slot_idx]
                        .store(value, Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_wt_lfo_sync_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_sync[slot_idx]
                        .store(value, Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_wt_lfo_sync_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_sync[slot_idx]
                        .store(value, Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_wt_lfo_sync_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_sync[slot_idx]
                        .store(value, Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_wt_lfo_division_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_division[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_wt_lfo_division_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_division[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_wt_lfo_division_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_division[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_wt_lfo_division_changed(move |index| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_wt_lfo_division[slot_idx]
                        .store(index as u32, Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_sample_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_sample_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_sample_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_sample_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_sample_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_sample_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_sample_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_sample_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_loop_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_loop_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_loop_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_loop_start_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_start[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }

        let tracks_animate = Arc::clone(tracks);
        let params_animate = Arc::clone(params);
        match i {
            0 => ui.on_animate_slot_a_loop_end_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_end[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            1 => ui.on_animate_slot_b_loop_end_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_end[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            2 => ui.on_animate_slot_c_loop_end_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_end[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            3 => ui.on_animate_slot_d_loop_end_changed(move |value| {
                let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
                if track_idx < NUM_TRACKS {
                    tracks_animate[track_idx].animate_slot_loop_end[slot_idx]
                        .store(value.to_bits(), Ordering::Relaxed);
                }
            }),
            _ => (),
        }
    }

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_slot_a_filter_type_changed(move |index| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_slot_filter_type[0]
                .store(index as u32, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_slot_a_filter_cutoff_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_slot_filter_cutoff[0]
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_slot_a_filter_resonance_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_slot_filter_resonance[0]
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_vector_changed(move |x, y| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_vector_x
                .store(x.to_bits(), Ordering::Relaxed);
            tracks_animate[track_idx]
                .animate_vector_y
                .store(y.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_x_waveform_changed(move |index| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_x_waveform
                .store(index as u32, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_x_sync_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx].animate_lfo_x_sync.store(value, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_x_division_changed(move |index| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_x_division
                .store(index as u32, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_x_rate_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_x_rate
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_x_amount_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_x_amount
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_y_waveform_changed(move |index| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_y_waveform
                .store(index as u32, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_y_sync_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx].animate_lfo_y_sync.store(value, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_y_division_changed(move |index| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_y_division
                .store(index as u32, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_y_rate_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_y_rate
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_lfo_y_amount_changed(move |value| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_lfo_y_amount
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.global::<RDSKeybedBus>().on_note_triggered(move |note| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_animate[track_idx]
                .animate_keybed_note
                .store(note, Ordering::Relaxed);
            tracks_animate[track_idx]
                .animate_keybed_trigger
                .store(true, Ordering::Relaxed);
        }
    });

    let tracks_animate = Arc::clone(tracks);
    let params_animate = Arc::clone(params);
    ui.on_animate_sequencer_grid_toggled(move |row, step| {
        let track_idx = params_animate.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let index = (row * 16 + step) as usize;
            if index < 160 {
                let current = tracks_animate[track_idx].animate_sequencer_grid[index].load(Ordering::Relaxed);
                tracks_animate[track_idx].animate_sequencer_grid[index].store(!current, Ordering::Relaxed);
            }
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_pitch_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_pitch
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_decay_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_decay
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_attack_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_attack
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_pitch_env_amount_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_pitch_env_amount
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_drive_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_drive
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_level_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_level
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_filter_type_changed(move |index| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_filter_type
                .store(index as u32, Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_filter_cutoff_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_filter_cutoff
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_filter_resonance_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_filter_resonance
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_filter_pre_drive_changed(move |value| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_kick[track_idx]
                .kick_filter_pre_drive
                .store(value, Ordering::Relaxed);
        }
    });

    let tracks_kick = Arc::clone(tracks);
    let params_kick = Arc::clone(params);
    ui.on_kick_sequencer_grid_toggled(move |step| {
        let track_idx = params_kick.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let index = step as usize;
            if index < SYNDRM_STEPS {
                let current = tracks_kick[track_idx].kick_sequencer_grid[index].load(Ordering::Relaxed);
                tracks_kick[track_idx].kick_sequencer_grid[index].store(!current, Ordering::Relaxed);
            }
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_tone_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_tone
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_decay_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_decay
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_snappy_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_snappy
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_attack_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_attack
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_drive_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_drive
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_level_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_level
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_filter_type_changed(move |index| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_filter_type
                .store(index as u32, Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_filter_cutoff_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_filter_cutoff
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_filter_resonance_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_filter_resonance
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_filter_pre_drive_changed(move |value| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_snare[track_idx]
                .snare_filter_pre_drive
                .store(value, Ordering::Relaxed);
        }
    });

    let tracks_snare = Arc::clone(tracks);
    let params_snare = Arc::clone(params);
    ui.on_snare_sequencer_grid_toggled(move |step| {
        let track_idx = params_snare.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let index = step as usize;
            if index < SYNDRM_STEPS {
                let current = tracks_snare[track_idx].snare_sequencer_grid[index].load(Ordering::Relaxed);
                tracks_snare[track_idx].snare_sequencer_grid[index].store(!current, Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_page_changed(move |page| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let clamped = page.clamp(0, (SYNDRM_PAGES - 1) as i32) as u32;
            tracks_syndrm[track_idx]
                .syndrm_page
                .store(clamped, Ordering::Relaxed);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_edit_lane_changed(move |lane| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let clamped = lane.clamp(0, (SYNDRM_LANES - 1) as i32) as u32;
            tracks_syndrm[track_idx]
                .syndrm_edit_lane
                .store(clamped, Ordering::Relaxed);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_edit_step_changed(move |step| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let clamped = step.clamp(0, (SYNDRM_STEPS - 1) as i32) as u32;
            tracks_syndrm[track_idx]
                .syndrm_edit_step
                .store(clamped, Ordering::Relaxed);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_hold_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_syndrm[track_idx]
                .syndrm_step_hold
                .store(value, Ordering::Relaxed);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_override_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                let edit_lane =
                    tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
                if edit_lane == 0 {
                    tracks_syndrm[track_idx].kick_step_override_enabled[edit_step]
                        .store(value, Ordering::Relaxed);
                } else if edit_lane == 1 {
                    tracks_syndrm[track_idx].snare_step_override_enabled[edit_step]
                        .store(value, Ordering::Relaxed);
                }
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_pitch_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_pitch[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_decay_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_decay[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_attack_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_attack[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_drive_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_drive[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_level_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_level[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_filter_type_changed(move |index| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_filter_type[edit_step]
                    .store(index as u32, Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_filter_cutoff_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_filter_cutoff[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_kick_filter_resonance_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].kick_step_filter_resonance[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_tone_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_tone[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_decay_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_decay[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_snappy_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_snappy[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_attack_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_attack[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_drive_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_drive[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_level_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_level[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_filter_type_changed(move |index| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_filter_type[edit_step]
                    .store(index as u32, Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_filter_cutoff_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_filter_cutoff[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_step_snare_filter_resonance_changed(move |value| {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let edit_step =
                tracks_syndrm[track_idx].syndrm_edit_step.load(Ordering::Relaxed) as usize;
            if edit_step < SYNDRM_STEPS {
                tracks_syndrm[track_idx].snare_step_filter_resonance[edit_step]
                    .store(value.to_bits(), Ordering::Relaxed);
            }
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_steps_lane_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_randomize_apply(&tracks_syndrm[track_idx], lanes, start, SYNDRM_PAGE_SIZE, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_steps_lane_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            syndrm_randomize_apply(&tracks_syndrm[track_idx], lanes, 0, SYNDRM_STEPS, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_steps_all_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_randomize_apply(&tracks_syndrm[track_idx], 0b11, start, SYNDRM_PAGE_SIZE, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_steps_all_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            syndrm_randomize_apply(&tracks_syndrm[track_idx], 0b11, 0, SYNDRM_STEPS, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_params_lane_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_randomize_apply(&tracks_syndrm[track_idx], lanes, start, SYNDRM_PAGE_SIZE, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_params_lane_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            syndrm_randomize_apply(&tracks_syndrm[track_idx], lanes, 0, SYNDRM_STEPS, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_params_all_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_randomize_apply(&tracks_syndrm[track_idx], 0b11, start, SYNDRM_PAGE_SIZE, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_params_all_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            syndrm_randomize_apply(&tracks_syndrm[track_idx], 0b11, 0, SYNDRM_STEPS, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_both_lane_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_randomize_apply(&tracks_syndrm[track_idx], lanes, start, SYNDRM_PAGE_SIZE, true, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_both_lane_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            syndrm_randomize_apply(&tracks_syndrm[track_idx], lanes, 0, SYNDRM_STEPS, true, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_both_all_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_randomize_apply(&tracks_syndrm[track_idx], 0b11, start, SYNDRM_PAGE_SIZE, true, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_randomize_both_all_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            syndrm_randomize_apply(&tracks_syndrm[track_idx], 0b11, 0, SYNDRM_STEPS, true, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_steps_lane_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_clear_apply(&tracks_syndrm[track_idx], lanes, start, SYNDRM_PAGE_SIZE, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_steps_lane_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            syndrm_clear_apply(&tracks_syndrm[track_idx], lanes, 0, SYNDRM_STEPS, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_steps_all_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_clear_apply(&tracks_syndrm[track_idx], 0b11, start, SYNDRM_PAGE_SIZE, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_steps_all_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            syndrm_clear_apply(&tracks_syndrm[track_idx], 0b11, 0, SYNDRM_STEPS, true, false);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_params_lane_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_clear_apply(&tracks_syndrm[track_idx], lanes, start, SYNDRM_PAGE_SIZE, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_params_lane_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            syndrm_clear_apply(&tracks_syndrm[track_idx], lanes, 0, SYNDRM_STEPS, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_params_all_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_clear_apply(&tracks_syndrm[track_idx], 0b11, start, SYNDRM_PAGE_SIZE, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_params_all_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            syndrm_clear_apply(&tracks_syndrm[track_idx], 0b11, 0, SYNDRM_STEPS, false, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_both_lane_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_clear_apply(&tracks_syndrm[track_idx], lanes, start, SYNDRM_PAGE_SIZE, true, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_both_lane_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let lane = tracks_syndrm[track_idx].syndrm_edit_lane.load(Ordering::Relaxed);
            let lanes = if lane == 0 { 0b01 } else { 0b10 };
            syndrm_clear_apply(&tracks_syndrm[track_idx], lanes, 0, SYNDRM_STEPS, true, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_both_all_page(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let page = tracks_syndrm[track_idx].syndrm_page.load(Ordering::Relaxed) as usize;
            let start = page * SYNDRM_PAGE_SIZE;
            syndrm_clear_apply(&tracks_syndrm[track_idx], 0b11, start, SYNDRM_PAGE_SIZE, true, true);
        }
    });

    let tracks_syndrm = Arc::clone(tracks);
    let params_syndrm = Arc::clone(params);
    ui.on_syndrm_clear_both_all_all(move || {
        let track_idx = params_syndrm.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            syndrm_clear_apply(&tracks_syndrm[track_idx], 0b11, 0, SYNDRM_STEPS, true, true);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_void_base_freq_changed(move |value| {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_void[track_idx]
                .void_base_freq
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_void_chaos_depth_changed(move |value| {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_void[track_idx]
                .void_chaos_depth
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_void_entropy_changed(move |value| {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_void[track_idx]
                .void_entropy
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_void_feedback_changed(move |value| {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_void[track_idx]
                .void_feedback
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_void_diffusion_changed(move |value| {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_void[track_idx]
                .void_diffusion
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_void_mod_rate_changed(move |value| {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_void[track_idx]
                .void_mod_rate
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_void_level_changed(move |value| {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            tracks_void[track_idx]
                .void_level
                .store(value.to_bits(), Ordering::Relaxed);
        }
    });

    let tracks_void = Arc::clone(tracks);
    let params_void = Arc::clone(params);
    ui.on_toggle_void(move || {
        let track_idx = params_void.selected_track.value().saturating_sub(1) as usize;
        if track_idx < NUM_TRACKS {
            let current = tracks_void[track_idx].void_enabled.load(Ordering::Relaxed);
            tracks_void[track_idx].void_enabled.store(!current, Ordering::Relaxed);
        }
    });

    refresh_browser_impl(ui, &current_path.lock(), current_folder_content_model);
}

#[derive(Clone)]
enum ProjectDialogAction {
    Save(PathBuf),
    Load(PathBuf),
    SaveWithInfo {
        path: PathBuf,
        title: String,
        description: String,
    },
    ExportZip {
        path: PathBuf,
        title: String,
        description: String,
    },
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

fn create_slint_ui() -> (std::rc::Rc<MinimalSoftwareWindow>, Box<TLBX1UI>) {
    SLINT_WINDOW_SLOT.with(|slot| {
        *slot.borrow_mut() = None;
    });
    let ui = Box::new(TLBX1UI::new().expect("Failed to create Slint UI"));
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

fn key_to_slint_string(key: &Key) -> Option<String> {
    match key {
        Key::Character(c) => Some(c.clone()),
        Key::Escape => Some("\u{001b}".to_string()),
        Key::Enter => Some("\r".to_string()),
        Key::Backspace => Some("\u{0008}".to_string()),
        Key::Tab => Some("\t".to_string()),
        Key::ArrowUp => Some("\u{f000}".to_string()),
        Key::ArrowDown => Some("\u{f001}".to_string()),
        Key::ArrowLeft => Some("\u{f002}".to_string()),
        Key::ArrowRight => Some("\u{f003}".to_string()),
        Key::Delete => Some("\u{007f}".to_string()),
        _ => None,
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

impl Vst3Plugin for TLBX1 {
    const VST3_CLASS_ID: [u8; 16] = *b"TLBX1Zencode____";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Sampler];
}

impl ClapPlugin for TLBX1 {
    const CLAP_ID: &'static str = "com.zencoder.tlbx-1";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("An audio toolbox plugin");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Sampler, ClapFeature::Stereo];
}

nih_export_vst3!(TLBX1);
nih_export_clap!(TLBX1);

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
