use nih_plug::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait};
use parking_lot::Mutex;
use slint::{LogicalPosition, LogicalSize, ModelRc, SharedString, VecModel};
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::cell::RefCell;
use std::sync::mpsc;
use std::sync::{Arc, Once};
use std::time::Instant;

pub const NUM_TRACKS: usize = 4;
pub const WAVEFORM_SUMMARY_SIZE: usize = 100;

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
    /// Whether the track is currently playing.
    is_playing: AtomicBool,
    /// Playback position in samples. Stored as u32 bits for f32.
    play_pos: AtomicU32,
    /// Track output level (linear gain).
    level: AtomicU32,
    /// Smoothed track output level (linear gain).
    level_smooth: AtomicU32,
    /// Track mute state.
    is_muted: AtomicBool,
    /// Loop start position as normalized 0..1.
    loop_start: AtomicU32,
    /// Loop length as normalized 0..1.
    loop_length: AtomicU32,
    /// Loop crossfade amount as normalized 0..0.5.
    loop_xfade: AtomicU32,
    /// Loop enabled.
    loop_enabled: AtomicBool,
    /// Logs one debug line per playback start to confirm audio thread output.
    debug_logged: AtomicBool,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            samples: Arc::new(Mutex::new(vec![vec![]; 2])),
            sample_path: Arc::new(Mutex::new(None)),
            waveform_summary: Arc::new(Mutex::new(vec![0.0; WAVEFORM_SUMMARY_SIZE])),
            is_recording: AtomicBool::new(false),
            is_playing: AtomicBool::new(false),
            play_pos: AtomicU32::new(0.0f32.to_bits()),
            level: AtomicU32::new(1.0f32.to_bits()),
            level_smooth: AtomicU32::new(1.0f32.to_bits()),
            is_muted: AtomicBool::new(false),
            loop_start: AtomicU32::new(0.0f32.to_bits()),
            loop_length: AtomicU32::new(1.0f32.to_bits()),
            loop_xfade: AtomicU32::new(0.0f32.to_bits()),
            loop_enabled: AtomicBool::new(true),
            debug_logged: AtomicBool::new(false),
        }
    }
}

pub struct GrainRust {
    params: Arc<GrainRustParams>,
    tracks: Arc<[Track; NUM_TRACKS]>,
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
        }
    }
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

impl Plugin for GrainRust {
    const NAME: &'static str = "GrainRust";
    const VENDOR: &'static str = "Zencoder";
    const URL: &'static str = "https://example.com";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        // This is a generator (sampler), so it should not require input channels.
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

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
            async_executor,
        }))
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let tracks = self.tracks.clone();
        Box::new(move |task| match task {
            GrainRustTask::LoadSample(track_idx, path) => {
                if track_idx >= NUM_TRACKS {
                    return;
                }
                
                match load_audio_file(&path) {
                    Ok(new_samples) => {
                        let mut samples = tracks[track_idx].samples.lock();
                        let mut summary = tracks[track_idx].waveform_summary.lock();
                        let mut sample_path = tracks[track_idx].sample_path.lock();
                        
                        *samples = new_samples;
                        *sample_path = Some(path.clone());
                        if !samples.is_empty() {
                            calculate_waveform_summary(&samples[0], &mut summary);
                        }
                        
                        nih_log!("Loaded sample: {:?}", path);
                    }
                    Err(e) => {
                        nih_log!("Failed to load sample: {:?}", e);
                    }
                }
            }
            GrainRustTask::SaveProject(path) => {
                if let Err(err) = save_project(&tracks, &path) {
                    nih_log!("Failed to save project: {:?}", err);
                } else {
                    nih_log!("Saved project: {:?}", path);
                }
            }
            GrainRustTask::LoadProject(path) => {
                if let Err(err) = load_project(&tracks, &path) {
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
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut keep_alive = false;

        // Handle recording for all tracks
        for track in self.tracks.iter() {
            if track.is_recording.load(Ordering::Relaxed) {
                keep_alive = true;
            }

            if track.is_recording.load(Ordering::Relaxed) {
                if let Some(mut samples) = track.samples.try_lock() {
                    // Ensure we have enough channels
                    while samples.len() < buffer.channels() {
                        samples.push(vec![]);
                    }

                    for channel_idx in 0..buffer.channels() {
                        let channel_data = &buffer.as_slice_immutable()[channel_idx];
                        samples[channel_idx].extend_from_slice(channel_data);
                    }
                }
            }
        }

        let any_playing = self
            .tracks
            .iter()
            .any(|track| track.is_playing.load(Ordering::Relaxed));
        if any_playing {
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
                    let track_level =
                        f32::from_bits(track.level.load(Ordering::Relaxed));
                    let track_muted = track.is_muted.load(Ordering::Relaxed);
                    let target_level = if track_muted { 0.0 } else { track_level };
                    let mut smooth_level =
                        f32::from_bits(track.level_smooth.load(Ordering::Relaxed));
                    let level_step = if num_buffer_samples > 0 {
                        (target_level - smooth_level) / num_buffer_samples as f32
                    } else {
                        0.0
                    };
                    let loop_enabled = track.loop_enabled.load(Ordering::Relaxed);
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

                    let loop_start = (loop_start_norm * num_samples as f32) as usize;
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
                        let mut pos = play_pos as usize;
                        if pos >= num_samples {
                            if loop_enabled {
                                pos = loop_start.min(num_samples.saturating_sub(1));
                                play_pos = pos as f32;
                            } else {
                                track.is_playing.store(false, Ordering::Relaxed);
                                break;
                            }
                        }

                        for channel_idx in 0..output.len() {
                            let src_channel = if num_channels == 1 {
                                0
                            } else if channel_idx < num_channels {
                                channel_idx
                            } else {
                                continue;
                            };
                            let mut sample_value = samples[src_channel][pos];
                            if loop_enabled && xfade_samples > 0 {
                                let xfade_start = loop_end.saturating_sub(xfade_samples);
                                if pos >= xfade_start && loop_end > loop_start {
                                    let tail_idx = pos - xfade_start;
                                    let head_pos = loop_start + tail_idx;
                                    if head_pos < loop_end {
                                        let fade_in = tail_idx as f32 / xfade_samples as f32;
                                        let fade_out = 1.0 - fade_in;
                                        let head_sample = samples[src_channel][head_pos];
                                        sample_value = sample_value * fade_out + head_sample * fade_in;
                                    }
                                }
                            }
                            let level = smooth_level + level_step * sample_idx as f32;
                            output[channel_idx][sample_idx] += sample_value * level;
                        }

                        play_pos += 1.0;
                        if loop_enabled && loop_end > loop_start {
                            if play_pos as usize >= loop_end {
                                play_pos = loop_start as f32;
                            }
                        }
                    }
                    
                    track.play_pos.store(play_pos.to_bits(), Ordering::Relaxed);
                    smooth_level += level_step * num_buffer_samples as f32;
                    track
                        .level_smooth
                        .store(smooth_level.to_bits(), Ordering::Relaxed);
                }
            }
        }

        // Apply global gain
        for channel_samples in buffer.iter_samples() {
            let gain = self.params.gain.smoothed.next();

            for sample in channel_samples {
                *sample *= gain;
            }
        }

        if keep_alive {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}

fn load_audio_file(path: &std::path::Path) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
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

    Ok(samples)
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    version: u32,
    tracks: Vec<ProjectTrack>,
}

#[derive(Serialize, Deserialize)]
struct ProjectTrack {
    sample_path: Option<String>,
    level: f32,
    muted: bool,
    loop_start: f32,
    loop_length: f32,
    loop_xfade: f32,
    loop_enabled: bool,
}

fn save_project(
    tracks: &Arc<[Track; NUM_TRACKS]>,
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
            loop_start: f32::from_bits(track.loop_start.load(Ordering::Relaxed)),
            loop_length: f32::from_bits(track.loop_length.load(Ordering::Relaxed)),
            loop_xfade: f32::from_bits(track.loop_xfade.load(Ordering::Relaxed)),
            loop_enabled: track.loop_enabled.load(Ordering::Relaxed),
        });
    }

    let project = ProjectFile {
        version: 1,
        tracks: track_states,
    };
    let json = serde_json::to_string_pretty(&project)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn load_project(
    tracks: &Arc<[Track; NUM_TRACKS]>,
    path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(path)?;
    let project: ProjectFile = serde_json::from_str(&json)?;
    for (track_idx, track_state) in project.tracks.iter().enumerate() {
        if track_idx >= NUM_TRACKS {
            break;
        }
        let track = &tracks[track_idx];
        track.level.store(track_state.level.to_bits(), Ordering::Relaxed);
        track
            .is_muted
            .store(track_state.muted, Ordering::Relaxed);
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
                Ok(new_samples) => {
                    *samples = new_samples;
                    *sample_path = Some(path);
                    if !samples.is_empty() {
                        calculate_waveform_summary(&samples[0], &mut summary);
                    }
                }
                Err(err) => {
                    nih_log!("Failed to load sample for track {}: {:?}", track_idx, err);
                    *samples = vec![vec![]; 2];
                    *summary = vec![0.0; WAVEFORM_SUMMARY_SIZE];
                    *sample_path = None;
                }
            }
        } else {
            *samples = vec![vec![]; 2];
            *summary = vec![0.0; WAVEFORM_SUMMARY_SIZE];
            *sample_path = None;
        }
    }
    Ok(())
}

struct SlintEditor {
    params: Arc<GrainRustParams>,
    tracks: Arc<[Track; NUM_TRACKS]>,
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
        let async_executor = self.async_executor.clone();

        let window_handle = baseview::Window::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: "GrainRust".to_string(),
                size: baseview::Size::new(1280.0, 800.0),
                scale: WindowScalePolicy::SystemScaleFactor,
                gl_config: None,
            },
            move |window| SlintWindow::new(window, context, params, tracks, async_executor),
        );

        Box::new(SlintEditorHandle { window: window_handle })
    }

    fn size(&self) -> (u32, u32) {
        (1280, 800)
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
    async_executor: AsyncExecutor<GrainRust>,
    slint_window: std::rc::Rc<MinimalSoftwareWindow>,
    ui: GrainRustUI,
    waveform_model: std::rc::Rc<VecModel<f32>>,
    sample_dialog_rx: std::sync::mpsc::Receiver<(usize, Option<PathBuf>)>,
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
        gui_context: Arc<dyn GuiContext>,
        params: Arc<GrainRustParams>,
        tracks: Arc<[Track; NUM_TRACKS]>,
        async_executor: AsyncExecutor<GrainRust>,
    ) -> Self {
        ensure_slint_platform();
        let (slint_window, ui) = create_slint_ui();
        let waveform_model =
            std::rc::Rc::new(VecModel::from(vec![0.0; WAVEFORM_SUMMARY_SIZE]));
        ui.set_waveform(ModelRc::from(waveform_model.clone()));
        let (sample_dialog_tx, sample_dialog_rx) = mpsc::channel();
        let (project_dialog_tx, project_dialog_rx) = mpsc::channel();

        let scale_factor = 1.0_f32;
        let logical_width = 1280.0_f32;
        let logical_height = 800.0_f32;
        let physical_width = (logical_width * scale_factor).round() as u32;
        let physical_height = (logical_height * scale_factor).round() as u32;

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
        slint_window.dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(logical_width, logical_height),
        });

        let output_devices = available_output_devices();
        let sample_rates = vec![44100, 48000, 88200, 96000];
        let buffer_sizes = vec![256, 512, 1024, 2048, 4096];

        initialize_ui(
            &ui,
            &gui_context,
            &params,
            &tracks,
            &async_executor,
            &output_devices,
            &sample_rates,
            &buffer_sizes,
            sample_dialog_tx,
            project_dialog_tx,
        );

        Self {
            gui_context,
            params,
            tracks,
            async_executor,
            slint_window,
            ui,
            waveform_model,
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
        let track_muted = self.tracks[track_idx].is_muted.load(Ordering::Relaxed);
        let loop_start =
            f32::from_bits(self.tracks[track_idx].loop_start.load(Ordering::Relaxed));
        let loop_length =
            f32::from_bits(self.tracks[track_idx].loop_length.load(Ordering::Relaxed));
        let loop_xfade =
            f32::from_bits(self.tracks[track_idx].loop_xfade.load(Ordering::Relaxed));
        let loop_enabled =
            self.tracks[track_idx].loop_enabled.load(Ordering::Relaxed);

        let play_pos = f32::from_bits(self.tracks[track_idx].play_pos.load(Ordering::Relaxed));
        let total_samples = if let Some(samples) = self.tracks[track_idx].samples.try_lock() {
            samples.get(0).map(|ch| ch.len()).unwrap_or(0)
        } else {
            0
        };
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

        self.ui.set_selected_track((track_idx + 1) as i32);
        self.ui.set_is_playing(is_playing);
        self.ui.set_is_recording(is_recording);
        self.ui.set_gain(gain);
        self.ui.set_track_level(track_level);
        self.ui.set_track_muted(track_muted);
        self.ui.set_loop_start(loop_start);
        self.ui.set_loop_length(loop_length);
        self.ui.set_loop_xfade(loop_xfade);
        self.ui.set_loop_enabled(loop_enabled);
        self.ui.set_playhead_index(playhead_index);
        self.waveform_model.set_vec(waveform);
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
        let logical = window_info.logical_size();
        let physical = window_info.physical_size();

        self.physical_width = physical.width;
        self.physical_height = physical.height;

        self.slint_window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: self.scale_factor,
        });
        self.slint_window.dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(logical.width as f32, logical.height as f32),
        });

        let _ = self.sb_surface.resize(
            std::num::NonZeroU32::new(self.physical_width).unwrap(),
            std::num::NonZeroU32::new(self.physical_height).unwrap(),
        );
    }
}

impl BaseWindowHandler for SlintWindow {
    fn on_frame(&mut self, _window: &mut BaseWindow) {
        while let Ok((track_idx, path)) = self.sample_dialog_rx.try_recv() {
            if track_idx < NUM_TRACKS {
                if let Some(path) = path {
                    self.async_executor
                        .execute_background(GrainRustTask::LoadSample(track_idx, path));
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
    _async_executor: &AsyncExecutor<GrainRust>,
    output_devices: &[String],
    sample_rates: &[u32],
    buffer_sizes: &[u32],
    sample_dialog_tx: std::sync::mpsc::Sender<(usize, Option<PathBuf>)>,
    project_dialog_tx: std::sync::mpsc::Sender<ProjectDialogAction>,
) {
    ui.set_output_devices(ModelRc::new(VecModel::from(
        output_devices
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

    let output_device_index = current_arg_value("--output-device")
        .and_then(|name| output_devices.iter().position(|device| device == &name))
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
    ui.set_sample_rate_index(sample_rate_index as i32);
    ui.set_buffer_size_index(buffer_size_index as i32);

    let ui_weak = ui.as_weak();

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

    let gui_context_gain = Arc::clone(gui_context);
    let params_gain = Arc::clone(params);
    ui.on_gain_changed(move |value| {
        let setter = ParamSetter::new(gui_context_gain.as_ref());
        setter.begin_set_parameter(&params_gain.gain);
        setter.set_parameter_normalized(&params_gain.gain, value);
        setter.end_set_parameter(&params_gain.gain);
    });

    let tracks_play = Arc::clone(tracks);
    ui.on_toggle_play(move || {
        let any_playing = tracks_play
            .iter()
            .any(|track| track.is_playing.load(Ordering::Relaxed));
        for track in tracks_play.iter() {
            if any_playing {
                track.is_playing.store(false, Ordering::Relaxed);
            } else {
                let loop_enabled = track.loop_enabled.load(Ordering::Relaxed);
                let loop_start_norm =
                    f32::from_bits(track.loop_start.load(Ordering::Relaxed)).clamp(0.0, 0.999);
                let loop_start = if loop_enabled {
                    if let Some(samples) = track.samples.try_lock() {
                        let len = samples.get(0).map(|ch| ch.len()).unwrap_or(0);
                        (loop_start_norm * len as f32) as f32
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                track.play_pos.store(loop_start.to_bits(), Ordering::Relaxed);
                track.debug_logged.store(false, Ordering::Relaxed);
                track.is_playing.store(true, Ordering::Relaxed);
            }
        }
    });

    let tracks_record = Arc::clone(tracks);
    let params_record = Arc::clone(params);
    ui.on_toggle_record(move || {
        let track_idx = params_record.selected_track.value().saturating_sub(1) as usize;
        if track_idx >= NUM_TRACKS {
            return;
        }
        let recording = tracks_record[track_idx]
            .is_recording
            .load(Ordering::Relaxed);
        if !recording {
            if let Some(mut samples) = tracks_record[track_idx].samples.try_lock() {
                for channel in samples.iter_mut() {
                    channel.clear();
                }
                *tracks_record[track_idx].sample_path.lock() = None;
                tracks_record[track_idx]
                    .is_recording
                    .store(true, Ordering::Relaxed);
                tracks_record[track_idx]
                    .is_playing
                    .store(false, Ordering::Relaxed);
            }
        } else {
            tracks_record[track_idx]
                .is_recording
                .store(false, Ordering::Relaxed);
            if let (Some(samples), Some(mut summary)) = (
                tracks_record[track_idx].samples.try_lock(),
                tracks_record[track_idx].waveform_summary.try_lock(),
            ) {
                if !samples.is_empty() {
                    calculate_waveform_summary(&samples[0], &mut summary);
                }
            }
        }
    });

    let params_load = Arc::clone(params);
    let sample_dialog_tx = sample_dialog_tx.clone();
    ui.on_load_sample(move || {
        let track_idx = params_load.selected_track.value().saturating_sub(1) as usize;
        if track_idx >= NUM_TRACKS {
            return;
        }
        let sample_dialog_tx = sample_dialog_tx.clone();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .add_filter("Audio", &["wav", "flac", "mp3", "ogg"])
                .pick_file();
            let _ = sample_dialog_tx.send((track_idx, path));
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

    ui.on_quit(|| {
        std::process::exit(0);
    });

    let ui_toggle = ui_weak.clone();
    ui.on_toggle_settings(move || {
        if let Some(ui) = ui_toggle.upgrade() {
            ui.set_show_settings(!ui.get_show_settings());
        }
    });

    let ui_output_device = ui_weak.clone();
    ui.on_output_device_selected(move |index| {
        if let Some(ui) = ui_output_device.upgrade() {
            ui.set_output_device_index(index);
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
        let output_device = output_devices.get(ui.get_output_device_index() as usize);
        let sample_rate = sample_rates.get(ui.get_sample_rate_index() as usize).copied();
        let buffer_size = buffer_sizes.get(ui.get_buffer_size_index() as usize).copied();
        if let Err(err) = restart_with_audio_settings(output_device, sample_rate, buffer_size) {
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

fn create_slint_ui() -> (std::rc::Rc<MinimalSoftwareWindow>, GrainRustUI) {
    SLINT_WINDOW_SLOT.with(|slot| {
        *slot.borrow_mut() = None;
    });
    let ui = GrainRustUI::new().expect("Failed to create Slint UI");
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

fn restart_with_audio_settings(
    output_device: Option<&String>,
    sample_rate: Option<u32>,
    buffer_size: Option<u32>,
) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let mut cmd = ProcessCommand::new(exe);

    if let Some(device) = output_device {
        cmd.arg("--output-device").arg(device);
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
