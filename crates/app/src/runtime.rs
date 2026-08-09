use crate::runtime_mailbox::{CommandInbox, CommandMailbox};
use crate::{CommandDispatch, RuntimeClock, SystemRuntimeClock};
use stageswap_core::{
    AppConfig, AppSnapshot, Command, DebouncedDetector, DetectorSettings, DeviceState, Frame,
    FrameCompositor, FrameMetadata, FramePacer, GrayImage, PIPELINE_FPS, PIPELINE_SIZE, RunState,
    Size, Source, SourceAvailability, TransitionController, bgra_to_gray, decide, image_similarity,
    off_frame, resize_bgra_to_gray, resize_bilinear,
};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(any(windows, test))]
use stageswap_core::MonitorDescriptor;
#[cfg(windows)]
use stageswap_core::{MonitorScore, MonitorTracker, MonitorTrackerSettings, RestartTarget};
#[cfg(windows)]
use stageswap_windows::{
    FramePublisher, MediaFoundationVideoInput, ScreenInput, VideoInput, VirtualCameraController,
    WindowsGraphicsScreenInput, choose_video_device, frame_pipe_name,
};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, SyncSender};

const COMMAND_CAPACITY: usize = 32;
const MAX_COMMANDS_PER_OUTPUT_CYCLE: usize = 8;
const ACTIVITY_LIMIT: usize = 20;
const FPS_TRACKING_WINDOW: Duration = Duration::from_secs(1);
#[cfg(any(windows, test))]
use stageswap_core::ComponentLifecycle;
#[cfg(windows)]
use stageswap_core::FRAME_STALE_AFTER;
#[cfg(windows)]
use stageswap_core::{ComponentFailureKind, ScreenFailureKind, WebcamFailureKind};
#[cfg(any(windows, test))]
const WEBCAM_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(any(windows, test))]
const SCREEN_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(windows)]
fn screen_failure_kind(message: &str) -> ScreenFailureKind {
    if message.contains("Graphics Capture is unavailable") {
        ScreenFailureKind::UnsupportedCapture
    } else if message.contains("no longer available") || message.contains("unavailable") {
        ScreenFailureKind::MissingSelectedMonitor
    } else if message.contains("HDR") || message.contains("10-bit") {
        ScreenFailureKind::UnsupportedHdr
    } else if message.contains("closed") {
        ScreenFailureKind::Closed
    } else {
        ScreenFailureKind::CaptureFailure
    }
}

#[cfg(windows)]
fn webcam_failure_kind(message: &str) -> WebcamFailureKind {
    let message = message.to_ascii_lowercase();
    if message.contains("privacy") {
        WebcamFailureKind::PrivacyDisabled
    } else if message.contains("access denied") || message.contains("0x80070005") {
        WebcamFailureKind::AccessDenied
    } else if message.contains("preempted")
        || message.contains("0xc00d3ea3")
        || message.contains("device busy")
    {
        WebcamFailureKind::DeviceBusy
    } else if message.contains("invalidated") || message.contains("0xc00d3ea2") {
        WebcamFailureKind::DeviceInvalidated
    } else if message.contains("media-type change") {
        WebcamFailureKind::IncompatibleTypeChange
    } else if message.contains("format")
        || message.contains("subtype")
        || message.contains("interlaced")
    {
        WebcamFailureKind::UnsupportedFormat
    } else {
        WebcamFailureKind::DriverFailure
    }
}
#[cfg(any(windows, test))]
const SCREEN_RESTART_BACKOFF_BASE: Duration = Duration::from_secs(5);
#[cfg(any(windows, test))]
const SCREEN_RESTART_BACKOFF_MAX: Duration = Duration::from_secs(60);
#[cfg(any(windows, test))]
const TARGET_WEBCAM_ASPECT_RATIO: f64 = 16.0 / 9.0;
#[cfg(any(windows, test))]
const WEBCAM_ASPECT_RATIO_TOLERANCE: f64 = 0.01;
#[cfg(any(windows, test))]
const BLACK_LUMA_THRESHOLD: u8 = 16;
#[cfg(any(windows, test))]
const BLACK_PIXEL_PERCENT: usize = 99;
#[cfg(any(windows, test))]
const SCREEN_CAPTURE_FAILURE_CONFIRMATIONS: u8 = 2;
#[cfg(any(windows, test))]
const AUTOMATIC_SCREEN_CHECK_INTERVAL: Duration = Duration::from_secs(30);

const DISCO_PALETTE_BGRA: [[u8; 3]; 6] = [
    [190, 28, 255],
    [255, 176, 24],
    [82, 245, 36],
    [34, 224, 255],
    [255, 70, 110],
    [214, 38, 255],
];

struct DiscoEffect {
    x_band: Vec<u8>,
    x_boost: Vec<u8>,
    y_boost: Vec<u8>,
}

impl DiscoEffect {
    fn new(size: Size) -> Self {
        Self {
            x_band: vec![0; size.width as usize],
            x_boost: vec![0; size.width as usize],
            y_boost: vec![0; size.height as usize],
        }
    }

    fn apply(&mut self, source: &Frame, elapsed: Duration) -> Frame {
        let width = source.size.width as usize;
        let height = source.size.height as usize;
        self.x_band.resize(width, 0);
        self.x_boost.resize(width, 0);
        self.y_boost.resize(height, 0);

        let frame_phase = usize::try_from(elapsed.as_millis() / 33).unwrap_or(usize::MAX);
        let palette_len = DISCO_PALETTE_BGRA.len();
        let horizontal_offset = frame_phase.wrapping_mul(13) % width.max(1);
        let horizontal_sweep = reflected_position(frame_phase.wrapping_mul(23), width);
        let vertical_sweep = reflected_position(frame_phase.wrapping_mul(11), height);
        let horizontal_radius = (width / 7).max(1);
        let vertical_radius = (height / 6).max(1);

        for x in 0..width {
            self.x_band[x] = ((((x + horizontal_offset) % width.max(1)) * palette_len
                / width.max(1)
                + frame_phase / 18)
                % palette_len) as u8;
            self.x_boost[x] = beam_boost(x, horizontal_sweep, horizontal_radius);
        }
        for y in 0..height {
            self.y_boost[y] = beam_boost(y, vertical_sweep, vertical_radius);
        }

        let pulse_position = frame_phase % 60;
        let pulse = pulse_position.min(60 - pulse_position) as u16;
        let base_strength = 76_u16 + pulse * 2;
        let flash_lift = disco_flash_lift(frame_phase);
        let mut pixels = Vec::with_capacity(source.pixels().len());
        pixels.extend_from_slice(source.pixels());
        for y in 0..height {
            let row_shift = (y * palette_len / height.max(1) + frame_phase / 12) % palette_len;
            let row_offset = y * source.stride as usize;
            for x in 0..width {
                let offset = row_offset + x * 4;
                let palette =
                    DISCO_PALETTE_BGRA[(self.x_band[x] as usize + row_shift) % palette_len];
                let strength =
                    (base_strength + u16::from(self.x_boost[x]) + u16::from(self.y_boost[y]))
                        .min(188);
                let inverse = 256 - strength;
                for channel in 0..3 {
                    let tinted = ((u16::from(pixels[offset + channel]) * inverse
                        + u16::from(palette[channel]) * strength
                        + 128)
                        >> 8) as u8;
                    pixels[offset + channel] = (u16::from(tinted)
                        + ((255 - u16::from(tinted)) * flash_lift + 128) / 256)
                        .min(255) as u8;
                }
            }
        }

        paint_disco_sparkles(
            &mut pixels,
            source.stride as usize,
            width,
            height,
            frame_phase / 3,
        );
        Frame::new(
            pixels.into(),
            source.size,
            source.stride,
            source.sequence,
            source.timestamp_100ns,
            source.received_at,
        )
        .expect("disco output preserves the source frame layout")
    }
}

fn reflected_position(phase: usize, length: usize) -> usize {
    if length <= 1 {
        return 0;
    }
    let span = length * 2 - 2;
    let position = phase % span;
    if position < length {
        position
    } else {
        span - position
    }
}

fn beam_boost(position: usize, center: usize, radius: usize) -> u8 {
    let distance = position.abs_diff(center);
    if distance >= radius {
        0
    } else {
        (((radius - distance) * 48) / radius) as u8
    }
}

fn disco_flash_lift(frame_phase: usize) -> u16 {
    let beat = match frame_phase % 36 {
        0 => 30,
        1 => 20,
        2 => 9,
        8 => 22,
        9 => 8,
        _ => 0,
    };
    let major = match frame_phase % 90 {
        0 => 38,
        1 => 26,
        2 => 14,
        3 => 6,
        _ => 0,
    };
    beat.max(major)
}

fn paint_disco_sparkles(
    pixels: &mut [u8],
    stride: usize,
    width: usize,
    height: usize,
    phase: usize,
) {
    if width == 0 || height == 0 {
        return;
    }
    for index in 0..18_usize {
        let seed = phase
            .wrapping_mul(1_103_515_245)
            .wrapping_add(index.wrapping_mul(12_345))
            .wrapping_add(0x9e37_79b9);
        let x = seed % width;
        let y = seed.rotate_left(13) % height;
        let radius = 3 + seed.rotate_left(7) % 5;
        for distance in 0..=radius {
            let intensity = 224_u8.saturating_sub((distance * 18) as u8);
            for (sparkle_x, sparkle_y) in [
                (x.saturating_sub(distance), y),
                ((x + distance).min(width - 1), y),
                (x, y.saturating_sub(distance)),
                (x, (y + distance).min(height - 1)),
            ] {
                let offset = sparkle_y * stride + sparkle_x * 4;
                pixels[offset] = pixels[offset].max(intensity);
                pixels[offset + 1] = pixels[offset + 1].max(intensity);
                pixels[offset + 2] = pixels[offset + 2].max(intensity);
            }
        }
    }
}

pub struct RuntimeHandle {
    commands: CommandMailbox,
    snapshot: Arc<RwLock<AppSnapshot>>,
    worker: Option<JoinHandle<()>>,
}

impl RuntimeHandle {
    pub fn spawn(config: AppConfig) -> Self {
        Self::spawn_with_clock(config, SystemRuntimeClock)
    }

    pub fn spawn_with_clock<C: RuntimeClock>(config: AppConfig, clock: C) -> Self {
        let (commands, receiver) = CommandMailbox::bounded(COMMAND_CAPACITY);
        let now = clock.now();
        let mut transition = TransitionController::default();
        let mut initial_snapshot = AppSnapshot {
            mode: config.output_mode,
            ..AppSnapshot::default()
        };
        initial_snapshot.actual_output = Source::Placeholder;
        initial_snapshot.automatic_target = Source::Placeholder;
        initial_snapshot.transition = transition.request(Source::Placeholder, now);
        initial_snapshot.previews.final_output = Some(Arc::new(off_frame(FrameMetadata {
            sequence: 1,
            timestamp_100ns: 0,
            received_at: now,
        })));
        let snapshot = Arc::new(RwLock::new(initial_snapshot));
        let worker_snapshot = Arc::clone(&snapshot);
        let worker = thread::Builder::new()
            .name("stageswap-runtime".into())
            .spawn(move || run(config, receiver, worker_snapshot, clock))
            .expect("runtime thread can be created");
        Self {
            commands,
            snapshot,
            worker: Some(worker),
        }
    }

    pub fn send(&self, command: Command) -> CommandDispatch {
        self.commands.dispatch(command)
    }

    pub fn try_send(&self, command: Command) -> CommandDispatch {
        self.commands.dispatch(command)
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.snapshot
            .read()
            .expect("runtime snapshot lock is not poisoned")
            .clone()
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.commands.request_shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Default)]
struct WebcamCropCache {
    source: Option<Arc<Frame>>,
    cropped: Option<Arc<Frame>>,
    format: Option<(Size, u32, u64)>,
    source_x: Vec<usize>,
    source_rows: Vec<usize>,
}

#[cfg(any(windows, test))]
impl WebcamCropCache {
    fn apply(
        &mut self,
        source: Arc<Frame>,
        enabled: bool,
        native_aspect_ratio: Option<f64>,
    ) -> Arc<Frame> {
        let Some(zoom) = webcam_crop_zoom(enabled, native_aspect_ratio) else {
            return source;
        };
        let format = (source.size, source.stride, zoom.to_bits());
        if self
            .source
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, &source))
            && self.format == Some(format)
            && let Some(cropped) = &self.cropped
        {
            return Arc::clone(cropped);
        }
        if self.format != Some(format) {
            let crop_width = (source.size.width as f64 / zoom).round().max(1.0) as u32;
            let crop_height = (source.size.height as f64 / zoom).round().max(1.0) as u32;
            let x_offset = (source.size.width - crop_width) / 2;
            let y_offset = (source.size.height - crop_height) / 2;
            self.source_x = (0..source.size.width)
                .map(|x| {
                    let source_x = x_offset
                        + ((u64::from(x) * u64::from(crop_width)) / u64::from(source.size.width))
                            as u32;
                    source_x as usize * 4
                })
                .collect();
            self.source_rows = (0..source.size.height)
                .map(|y| {
                    let source_y = y_offset
                        + ((u64::from(y) * u64::from(crop_height)) / u64::from(source.size.height))
                            as u32;
                    source_y as usize * source.stride as usize
                })
                .collect();
            self.format = Some(format);
        }
        let mut pixels = vec![0; source.pixels().len()];
        for (y, source_row) in self.source_rows.iter().copied().enumerate() {
            let destination_row = y * source.stride as usize;
            for (x, source_x) in self.source_x.iter().copied().enumerate() {
                let source_offset = source_row + source_x;
                let destination = destination_row + x * 4;
                pixels[destination..destination + 4]
                    .copy_from_slice(&source.pixels()[source_offset..source_offset + 4]);
            }
        }
        let cropped = Arc::new(
            Frame::new(
                pixels.into(),
                source.size,
                source.stride,
                source.sequence,
                source.timestamp_100ns,
                source.received_at,
            )
            .expect("webcam crop preserves valid frame dimensions"),
        );
        self.source = Some(source);
        self.cropped = Some(Arc::clone(&cropped));
        cropped
    }
}

#[cfg(any(windows, test))]
fn webcam_crop_zoom(enabled: bool, native_aspect_ratio: Option<f64>) -> Option<f64> {
    let aspect_ratio = enabled
        .then_some(native_aspect_ratio)
        .flatten()
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)?;
    let relative_difference =
        ((aspect_ratio - TARGET_WEBCAM_ASPECT_RATIO) / TARGET_WEBCAM_ASPECT_RATIO).abs();
    (relative_difference > WEBCAM_ASPECT_RATIO_TOLERANCE).then(|| {
        (aspect_ratio / TARGET_WEBCAM_ASPECT_RATIO).max(TARGET_WEBCAM_ASPECT_RATIO / aspect_ratio)
    })
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScreenCaptureRecoveryObservation {
    Clear,
    AwaitingConfirmation,
    Restart,
}

#[cfg(any(windows, test))]
#[derive(Default)]
struct ScreenCaptureRecovery {
    consecutive_failures: u8,
}

#[cfg(any(windows, test))]
impl ScreenCaptureRecovery {
    fn reset(&mut self) {
        self.consecutive_failures = 0;
    }

    fn observe(&mut self, frame: Option<&Frame>) -> ScreenCaptureRecoveryObservation {
        if frame.is_some_and(|frame| !is_nearly_black(frame)) {
            self.reset();
            return ScreenCaptureRecoveryObservation::Clear;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures < SCREEN_CAPTURE_FAILURE_CONFIRMATIONS {
            ScreenCaptureRecoveryObservation::AwaitingConfirmation
        } else {
            self.reset();
            ScreenCaptureRecoveryObservation::Restart
        }
    }
}

#[cfg(any(windows, test))]
fn is_nearly_black(frame: &Frame) -> bool {
    let Some(thumbnail) = gray_thumbnail(frame) else {
        return false;
    };
    let pixels = &thumbnail.pixels;
    let black_pixels = pixels
        .iter()
        .filter(|pixel| **pixel <= BLACK_LUMA_THRESHOLD)
        .count();
    black_pixels * 100 >= pixels.len() * BLACK_PIXEL_PERCENT
}

#[cfg(any(windows, test))]
fn automatic_screen_tasks_due(
    automatic_monitor_rescans: bool,
    automatic_screen_capture_recovery: bool,
    last_monitor_scan: Instant,
    last_screen_capture_recovery_check: Instant,
    now: Instant,
) -> (bool, bool) {
    (
        automatic_monitor_rescans
            && now.duration_since(last_monitor_scan) >= AUTOMATIC_SCREEN_CHECK_INTERVAL,
        automatic_screen_capture_recovery
            && now.duration_since(last_screen_capture_recovery_check)
                >= AUTOMATIC_SCREEN_CHECK_INTERVAL,
    )
}

#[cfg(any(windows, test))]
fn screen_restart_backoff(consecutive_failures: u32) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(4);
    SCREEN_RESTART_BACKOFF_BASE
        .checked_mul(1_u32 << exponent)
        .unwrap_or(SCREEN_RESTART_BACKOFF_MAX)
        .min(SCREEN_RESTART_BACKOFF_MAX)
}

struct RuntimeState {
    config: AppConfig,
    snapshot: AppSnapshot,
    transition: TransitionController,
    compositor: FrameCompositor,
    disco: Option<DiscoEffect>,
    webcam_fps: SourceFpsTracker,
    screen_fps: SourceFpsTracker,
    output_fps: OutputFpsTracker,
    activity: VecDeque<String>,
    sequence: u64,
    started_at: Instant,
    reference: Option<GrayImage>,
    detector: DebouncedDetector,
    last_detection: Instant,
    #[cfg(any(windows, test))]
    webcam_crop: WebcamCropCache,
}

impl RuntimeState {
    #[cfg(test)]
    fn new(config: AppConfig) -> Self {
        Self::new_at(config, Instant::now())
    }

    fn new_at(config: AppConfig, now: Instant) -> Self {
        let mode = config.output_mode;
        let (reference, reference_preview) = load_reference(&config.reference_image_path)
            .map_or((None, None), |(reference, preview)| {
                (Some(reference), Some(preview))
            });
        let detector = DebouncedDetector::new(DetectorSettings {
            threshold: config.similarity_threshold,
            ..DetectorSettings::default()
        });
        let sequence = 1;
        let mut transition = TransitionController::default();
        let mut snapshot = AppSnapshot {
            mode,
            selected_video_device_id: config.selected_video_device_id.clone(),
            ..AppSnapshot::default()
        };
        snapshot.actual_output = Source::Placeholder;
        snapshot.automatic_target = Source::Placeholder;
        snapshot.transition = transition.request(Source::Placeholder, now);
        snapshot.previews.final_output = Some(Arc::new(off_frame(FrameMetadata {
            sequence,
            timestamp_100ns: 0,
            received_at: now,
        })));
        snapshot.previews.reference = reference_preview;
        Self {
            snapshot,
            config,
            transition,
            compositor: FrameCompositor::default(),
            disco: None,
            webcam_fps: SourceFpsTracker::default(),
            screen_fps: SourceFpsTracker::default(),
            output_fps: OutputFpsTracker::default(),
            activity: VecDeque::with_capacity(ACTIVITY_LIMIT),
            sequence,
            started_at: now,
            reference,
            detector,
            last_detection: now - Duration::from_millis(250),
            #[cfg(any(windows, test))]
            webcam_crop: WebcamCropCache::default(),
        }
    }

    fn record(&mut self, message: impl Into<String>) {
        if self.activity.len() == ACTIVITY_LIMIT {
            self.activity.pop_front();
        }
        self.activity.push_back(message.into());
        self.snapshot.recent_activity = self.activity.iter().cloned().collect::<Vec<_>>().into();
    }

    #[cfg(any(windows, test))]
    fn stage_reference_candidate(&mut self, frame: Arc<Frame>) {
        self.snapshot.previews.reference_candidate = Some(frame);
    }

    #[cfg(any(windows, test))]
    fn take_reference_candidate(&mut self) -> Option<Arc<Frame>> {
        self.snapshot.previews.reference_candidate.take()
    }

    #[cfg(any(windows, test))]
    fn confirm_reference_candidate(&mut self) -> Result<(), String> {
        let frame = self
            .snapshot
            .previews
            .reference_candidate
            .as_ref()
            .cloned()
            .ok_or_else(|| "no reference candidate".to_owned())?;
        let reference = gray_thumbnail(&frame)
            .ok_or_else(|| "reference candidate is not a valid screen frame".to_owned())?;

        // Persist first so a failed save leaves both the candidate and the active
        // reference untouched. The UI can then offer an exact retry or retake.
        save_reference(&frame, &self.config.reference_image_path)?;
        self.take_reference_candidate();
        self.install_reference(reference, &frame, Instant::now());
        Ok(())
    }

    fn discard_reference_candidate(&mut self) {
        self.snapshot.previews.reference_candidate = None;
    }

    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Start => {
                self.snapshot.run_state = RunState::Running;
                self.record("Automation started");
            }
            Command::Stop => {
                self.snapshot.run_state = RunState::Stopped;
                self.show_off_output(Instant::now());
                self.record("Automation stopped");
            }
            Command::ToggleDisco => {
                if self.disco.is_some() {
                    self.disco = None;
                    self.snapshot.disco_enabled = false;
                    self.record("Disco mode disabled");
                } else {
                    self.disco = Some(DiscoEffect::new(PIPELINE_SIZE));
                    self.snapshot.disco_enabled = true;
                    self.record("Disco mode enabled");
                }
            }
            Command::SetMode(mode) => {
                self.snapshot.mode = mode;
                self.config.output_mode = mode;
                self.record(format!("Output mode changed to {mode:?}"));
            }
            Command::UpdateSettings(config) | Command::ReloadSettings(config) => {
                self.config = *config;
                self.snapshot.mode = self.config.output_mode;
                self.snapshot.selected_video_device_id =
                    self.config.selected_video_device_id.clone();
                let (reference, preview) = load_reference(&self.config.reference_image_path)
                    .map_or((None, None), |(reference, preview)| {
                        (Some(reference), Some(preview))
                    });
                self.reference = reference;
                self.snapshot.previews.reference = preview;
                self.discard_reference_candidate();
                self.detector = DebouncedDetector::new(DetectorSettings {
                    threshold: self.config.similarity_threshold,
                    ..DetectorSettings::default()
                });
                self.record("Settings updated");
            }
            Command::CaptureReference => {
                self.discard_reference_candidate();
                self.record("Reference capture requested");
            }
            Command::CaptureReferenceCandidate => {
                self.record("Reference candidate capture requested")
            }
            Command::ConfirmReferenceCandidate => {
                self.record("Reference candidate confirmation requested")
            }
            Command::DiscardReferenceCandidate => {
                self.discard_reference_candidate();
                self.record("Reference candidate discarded")
            }
            Command::ImportReference(path) => {
                self.discard_reference_candidate();
                let _ = path;
                self.record("Reference import requested");
            }
            Command::SelectMonitor(_) => self.record("Tracked monitor selection requested"),
            Command::RefreshVideoDevices => self.record("Video device list refreshed"),
            Command::Rescan => self.record("Monitor rescan requested"),
            Command::Restart(target) => self.record(format!("Restart requested: {target:?}")),
            Command::Exit => return false,
        }
        true
    }

    fn show_off_output(&mut self, now: Instant) {
        self.sequence = self.sequence.wrapping_add(1).max(1);
        let timestamp = now.saturating_duration_since(self.started_at).as_nanos() / 100;
        self.snapshot.actual_output = Source::Placeholder;
        self.snapshot.automatic_target = Source::Placeholder;
        self.snapshot.transition = self.transition.request(Source::Placeholder, now);
        self.snapshot.previews.final_output = Some(Arc::new(off_frame(FrameMetadata {
            sequence: self.sequence,
            timestamp_100ns: i64::try_from(timestamp).unwrap_or(i64::MAX),
            received_at: now,
        })));
    }

    fn tick(&mut self, now: Instant) {
        if self.snapshot.run_state != RunState::Running {
            self.show_off_output(now);
            return;
        }
        let decision = decide(
            self.snapshot.mode,
            self.snapshot.detection,
            self.snapshot.availability,
        );
        self.snapshot.automatic_target = decision.automatic_target;
        if self.snapshot.actual_output != decision.desired_output {
            self.snapshot.transition = self.transition.request(decision.desired_output, now);
            self.snapshot.actual_output = decision.desired_output;
        } else {
            self.snapshot.transition = self.transition.tick(now);
        }
        self.sequence = self.sequence.wrapping_add(1).max(1);
        let timestamp = now.saturating_duration_since(self.started_at).as_nanos() / 100;
        let output = self.compositor.compose(
            self.snapshot.previews.webcam.as_ref(),
            self.snapshot.previews.screen.as_ref(),
            self.snapshot.transition.screen_mix,
            self.config.placeholder_color_bgra,
            FrameMetadata {
                sequence: self.sequence,
                timestamp_100ns: i64::try_from(timestamp).unwrap_or(i64::MAX),
                received_at: now,
            },
        );
        let output = if let Some(disco) = self.disco.as_mut() {
            disco.apply(&output, now.saturating_duration_since(self.started_at))
        } else {
            output
        };
        self.snapshot.previews.final_output = Some(Arc::new(output));
    }

    #[cfg(any(windows, test))]
    fn install_reference(&mut self, reference: GrayImage, preview: &Frame, now: Instant) {
        self.sequence = self.sequence.wrapping_add(1).max(1);
        self.reference = Some(reference);
        self.snapshot.previews.reference = Frame::new(
            preview.pixels_arc(),
            preview.size,
            preview.stride,
            self.sequence,
            preview.timestamp_100ns,
            now,
        )
        .ok()
        .map(Arc::new);
        self.detector.reset();
        self.snapshot.detection = stageswap_core::DetectionState::Unknown;
        self.last_detection = now - Duration::from_millis(250);
    }

    fn detect(&mut self, now: Instant) {
        if now.duration_since(self.last_detection) < Duration::from_millis(250) {
            return;
        }
        self.last_detection = now;
        let candidate = self
            .snapshot
            .availability
            .screen_ready
            .then_some(self.snapshot.previews.screen.as_deref())
            .flatten()
            .and_then(gray_thumbnail);
        let (similarity, valid) = match (&self.reference, candidate) {
            (Some(reference), Some(candidate)) => (image_similarity(reference, &candidate), true),
            _ => (0.0, false),
        };
        self.snapshot.detection = self.detector.update(similarity, valid);
    }

    fn refresh_input_fps(&mut self, now: Instant) {
        let webcam = self
            .snapshot
            .availability
            .camera_ready
            .then_some(self.snapshot.previews.webcam.as_deref())
            .flatten();
        let screen = self
            .snapshot
            .availability
            .screen_ready
            .then_some(self.snapshot.previews.screen.as_deref())
            .flatten();
        self.snapshot.webcam_fps = self.webcam_fps.observe(webcam, now);
        self.snapshot.screen_fps = self.screen_fps.observe(screen, now);
    }
}

#[derive(Default)]
struct SourceFpsTracker {
    samples: VecDeque<(Instant, u64)>,
    last_sequence: Option<u64>,
    last_received_at: Option<Instant>,
    displayed: Option<u32>,
}

impl SourceFpsTracker {
    fn observe(&mut self, frame: Option<&Frame>, now: Instant) -> Option<u32> {
        if let Some(frame) = frame
            && self.last_sequence != Some(frame.sequence)
        {
            if self
                .last_sequence
                .is_some_and(|sequence| frame.sequence <= sequence)
            {
                self.samples.clear();
                self.displayed = None;
            }
            self.last_sequence = Some(frame.sequence);
            self.last_received_at = Some(frame.received_at);
            self.samples.push_back((frame.received_at, frame.sequence));
            while self.samples.front().is_some_and(|(received_at, _)| {
                frame.received_at.saturating_duration_since(*received_at) > FPS_TRACKING_WINDOW
            }) {
                self.samples.pop_front();
            }
            if let (Some((first_time, first_sequence)), Some((last_time, last_sequence))) =
                (self.samples.front(), self.samples.back())
            {
                let elapsed = last_time.saturating_duration_since(*first_time);
                let frames = last_sequence.saturating_sub(*first_sequence);
                if frames > 0 && !elapsed.is_zero() {
                    self.displayed =
                        Some(((frames as f64 / elapsed.as_secs_f64()).round() as u32).min(999));
                }
            }
        }

        if self
            .last_received_at
            .is_some_and(|received| now.saturating_duration_since(received) >= FPS_TRACKING_WINDOW)
        {
            Some(0)
        } else {
            self.displayed
        }
    }
}

#[derive(Default)]
struct OutputFpsTracker {
    samples: VecDeque<Instant>,
}

impl OutputFpsTracker {
    fn observe(&mut self, now: Instant) -> Option<u32> {
        self.samples.push_back(now);
        while self
            .samples
            .front()
            .is_some_and(|sample| now.saturating_duration_since(*sample) > FPS_TRACKING_WINDOW)
        {
            self.samples.pop_front();
        }
        let first = self.samples.front()?;
        let last = self.samples.back()?;
        let elapsed = last.saturating_duration_since(*first);
        let frames = self.samples.len().saturating_sub(1);
        (frames > 0 && !elapsed.is_zero())
            .then(|| ((frames as f64 / elapsed.as_secs_f64()).round() as u32).min(999))
    }
}

fn gray_thumbnail(frame: &Frame) -> Option<GrayImage> {
    resize_bgra_to_gray(
        frame.pixels(),
        frame.size,
        frame.stride as usize,
        Size::new(160, 90),
    )
    .ok()
}

fn load_reference(path: &str) -> Option<(GrayImage, Arc<Frame>)> {
    if path.is_empty() {
        return None;
    }
    let image = image::open(path).ok()?.to_rgba8();
    let size = Size::new(image.width(), image.height());
    let mut bgra = image.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let pixels: Arc<[u8]> = bgra.into();
    let reference = bgra_to_gray(&pixels, size, size.width as usize * 4)
        .ok()
        .and_then(|image| resize_bilinear(&image, Size::new(160, 90)).ok())?;
    let preview = Frame::new(pixels, size, size.width * 4, 0, 0, Instant::now()).ok()?;
    Some((reference, Arc::new(preview)))
}

#[cfg(any(windows, test))]
fn save_reference(frame: &Frame, path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("reference image path is empty".into());
    }
    let mut rgba = frame.pixels().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create reference directory: {error}"))?;
    }
    image::save_buffer(
        path,
        &rgba,
        frame.size.width,
        frame.size.height,
        image::ColorType::Rgba8,
    )
    .map_err(|error| format!("could not save reference image: {error}"))
}

#[cfg(windows)]
fn import_reference(
    source: &std::path::Path,
    destination: &str,
) -> Result<(GrayImage, Arc<Frame>), String> {
    if destination.is_empty() {
        return Err("reference image path is empty".into());
    }
    let image = image::open(source)
        .map_err(|error| format!("could not decode reference image: {error}"))?
        .to_rgba8();
    if let Some(parent) = std::path::Path::new(destination).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create reference directory: {error}"))?;
    }
    image
        .save(destination)
        .map_err(|error| format!("could not save local reference image: {error}"))?;
    load_reference(destination).ok_or_else(|| "could not load imported reference image".into())
}

trait RuntimePorts {
    fn command(&mut self, command: &Command, state: &mut RuntimeState);
    fn refresh_inputs(&mut self, state: &mut RuntimeState, now: Instant);
    fn publish(&self, state: &mut RuntimeState, now: Instant);
}

struct RuntimeEngine<P> {
    state: RuntimeState,
    platform: P,
    pacer: FramePacer,
}

impl<P: RuntimePorts> RuntimeEngine<P> {
    fn from_parts(state: RuntimeState, platform: P, now: Instant) -> Self {
        let frame_interval = Duration::from_nanos(1_000_000_000 / u64::from(PIPELINE_FPS));
        Self {
            state,
            platform,
            pacer: FramePacer::new(now, frame_interval),
        }
    }

    fn wait_duration(&self, now: Instant) -> Duration {
        self.pacer.wait_duration(now)
    }

    fn command(&mut self, command: Command) -> bool {
        self.platform.command(&command, &mut self.state);
        self.state.command(command)
    }

    fn step(&mut self, now: Instant) -> bool {
        if !self.pacer.is_due(now, Duration::ZERO) {
            return false;
        }
        self.pacer.advance(now);
        self.platform.refresh_inputs(&mut self.state, now);
        self.state.refresh_input_fps(now);
        self.state.detect(now);
        self.state.tick(now);
        self.state.snapshot.output_fps = self.state.output_fps.observe(now);
        self.platform.publish(&mut self.state, now);
        true
    }
}

fn run<C: RuntimeClock>(
    config: AppConfig,
    commands: CommandInbox,
    shared: Arc<RwLock<AppSnapshot>>,
    clock: C,
) {
    let start_automatically = config.start_automatically;
    let now = clock.now();
    let mut state = RuntimeState::new_at(config, now);
    state.snapshot.availability = SourceAvailability::default();
    state.snapshot.webcam_state = DeviceState::Unavailable;
    state.snapshot.screen_state = DeviceState::Unavailable;
    state.snapshot.virtual_camera_state = DeviceState::Unavailable;
    state.snapshot.actual_output = Source::Placeholder;
    let platform = Platform::new(&mut state);
    if start_automatically {
        state.command(Command::Start);
    }
    let mut engine = RuntimeEngine::from_parts(state, platform, now);
    loop {
        if commands.shutdown_requested() {
            break;
        }
        let mut processed = 0;
        if let Some(command) = commands.recv_timeout(engine.wait_duration(clock.now())) {
            if !engine.command(command) {
                break;
            }
            processed += 1;
        }
        while processed < MAX_COMMANDS_PER_OUTPUT_CYCLE && !commands.shutdown_requested() {
            let Some(command) = commands.try_recv() else {
                break;
            };
            if !engine.command(command) {
                return;
            }
            processed += 1;
        }
        if commands.shutdown_requested() {
            break;
        }
        let now = clock.now();
        if engine.step(now) {
            *shared
                .write()
                .expect("runtime snapshot lock is not poisoned") = engine.state.snapshot.clone();
        }
    }
}

#[cfg(any(windows, test))]
fn choose_initial_monitor(
    saved_label: &str,
    monitors: &[MonitorDescriptor],
) -> Option<MonitorDescriptor> {
    monitors
        .iter()
        .find(|monitor| monitor.label == saved_label)
        .or_else(|| monitors.get(1))
        .or_else(|| monitors.first())
        .cloned()
}

#[cfg(windows)]
struct MonitorScanRequest {
    generation: u64,
    reference: Option<GrayImage>,
    cursor_visible: bool,
}

#[cfg(windows)]
struct MonitorScanResult {
    generation: u64,
    monitors: Vec<stageswap_core::MonitorDescriptor>,
    scores: Vec<MonitorScore>,
}

#[cfg(windows)]
struct MonitorScanWorker {
    requests: Option<SyncSender<MonitorScanRequest>>,
    results: Option<Receiver<MonitorScanResult>>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    in_flight: bool,
    pending: Option<MonitorScanRequest>,
}

#[cfg(windows)]
impl MonitorScanWorker {
    fn start() -> Result<Self, String> {
        let (request_sender, request_receiver) = mpsc::sync_channel::<MonitorScanRequest>(1);
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("stageswap-monitor-scan".into())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let result = scan_monitors(request, &worker_stop);
                    if result_sender.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("could not start monitor scan worker: {error}"))?;
        Ok(Self {
            requests: Some(request_sender),
            results: Some(result_receiver),
            worker: Some(worker),
            stop,
            in_flight: false,
            pending: None,
        })
    }

    fn request(&mut self, request: MonitorScanRequest) -> bool {
        if self.in_flight {
            self.pending = Some(request);
            return true;
        }
        let sent = self
            .requests
            .as_ref()
            .is_some_and(|sender| sender.try_send(request).is_ok());
        self.in_flight = sent;
        sent
    }

    fn poll(&mut self) -> Option<MonitorScanResult> {
        let result = self.results.as_ref()?.try_recv().ok()?;
        self.in_flight = false;
        if let Some(request) = self.pending.take() {
            self.in_flight = self
                .requests
                .as_ref()
                .is_some_and(|sender| sender.try_send(request).is_ok());
        }
        Some(result)
    }

    fn clear_pending(&mut self) {
        self.pending = None;
    }
}

#[cfg(windows)]
impl Drop for MonitorScanWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.requests.take();
        self.results.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
fn scan_monitors(request: MonitorScanRequest, stop: &AtomicBool) -> MonitorScanResult {
    let input = WindowsGraphicsScreenInput::default();
    let monitors = input.enumerate().unwrap_or_default();
    let scores = request
        .reference
        .as_ref()
        .map_or_else(Vec::new, |reference| {
            monitors
                .iter()
                .cloned()
                .take_while(|_| !stop.load(Ordering::Acquire))
                .map(|monitor| {
                    let mut capture = WindowsGraphicsScreenInput::default();
                    let valid = capture.start(&monitor, request.cursor_visible).is_ok();
                    let deadline = Instant::now() + Duration::from_millis(750);
                    let frame = loop {
                        if stop.load(Ordering::Acquire) {
                            break None;
                        }
                        if let Some(frame) = capture.latest_frame() {
                            break Some(frame);
                        }
                        if Instant::now() >= deadline {
                            break None;
                        }
                        thread::sleep(Duration::from_millis(25));
                    };
                    capture.stop();
                    let similarity = frame
                        .as_deref()
                        .and_then(gray_thumbnail)
                        .map_or(0.0, |candidate| image_similarity(reference, &candidate));
                    MonitorScore {
                        monitor,
                        similarity,
                        capture_valid: valid && frame.is_some(),
                    }
                })
                .collect()
        });
    MonitorScanResult {
        generation: request.generation,
        monitors,
        scores,
    }
}

#[cfg(windows)]
struct Platform {
    publisher: Option<FramePublisher>,
    camera: Option<VirtualCameraController>,
    pipe_name: Option<String>,
    webcam: MediaFoundationVideoInput,
    screen: WindowsGraphicsScreenInput,
    selected_monitor: Option<MonitorDescriptor>,
    selected_display_hdr_unsupported: bool,
    monitor_tracker: MonitorTracker,
    screen_capture_recovery: ScreenCaptureRecovery,
    last_monitor_scan: Instant,
    last_screen_capture_recovery_check: Instant,
    monitor_scan_generation: u64,
    monitor_scan_worker: Option<MonitorScanWorker>,
}

#[cfg(windows)]
impl Platform {
    fn new(state: &mut RuntimeState) -> Self {
        let now = Instant::now();
        state
            .snapshot
            .publisher_component
            .transition(ComponentLifecycle::Starting, now);
        state
            .snapshot
            .virtual_camera_component
            .transition(ComponentLifecycle::Starting, now);
        let pipe_name = match frame_pipe_name() {
            Ok(name) => Some(name),
            Err(error) => {
                state.snapshot.warning = Some(error.clone());
                state.record(format!("Virtual camera pipe failed: {error}"));
                None
            }
        };
        let publisher =
            pipe_name
                .as_ref()
                .and_then(|pipe_name| match FramePublisher::start(pipe_name) {
                    Ok(publisher) => Some(publisher),
                    Err(error) => {
                        state
                            .snapshot
                            .publisher_component
                            .mark_failed(now, error.clone());
                        state.snapshot.warning = Some(error.clone());
                        state.record(format!("Frame publisher failed: {error}"));
                        None
                    }
                });
        let camera = publisher.as_ref().and_then(|_| {
            pipe_name.as_ref().and_then(|pipe_name| {
                match VirtualCameraController::start(pipe_name.clone()) {
                    Ok(camera) => {
                        state.snapshot.virtual_camera_state = DeviceState::Ready;
                        state.snapshot.virtual_camera_component.mark_ready(now);
                        state.record("Virtual camera initialized");
                        Some(camera)
                    }
                    Err(error) => {
                        state.snapshot.virtual_camera_state = DeviceState::Failed;
                        state
                            .snapshot
                            .virtual_camera_component
                            .mark_failed(now, error.clone());
                        state.snapshot.warning = Some(error.clone());
                        state.record(format!("Virtual camera failed: {error}"));
                        None
                    }
                }
            })
        });
        if publisher.is_none() {
            state.snapshot.virtual_camera_state = DeviceState::Failed;
            if state.snapshot.publisher_component.lifecycle != ComponentLifecycle::Failed {
                state
                    .snapshot
                    .publisher_component
                    .mark_failed(now, "frame publisher is unavailable");
            }
            if state.snapshot.virtual_camera_component.lifecycle != ComponentLifecycle::Failed {
                state
                    .snapshot
                    .virtual_camera_component
                    .mark_failed(now, "frame publisher is unavailable");
            }
        }
        let mut webcam = MediaFoundationVideoInput::default();
        state
            .snapshot
            .webcam_component
            .transition(ComponentLifecycle::Starting, now);
        match webcam.enumerate() {
            Ok(devices) => {
                state.snapshot.video_devices = devices
                    .iter()
                    .map(|device| stageswap_core::VideoDeviceChoice {
                        id: device.id.clone(),
                        name: device.name.clone(),
                    })
                    .collect::<Vec<_>>()
                    .into();
                let saved = state.config.selected_video_device_id.clone();
                let saved_is_physical = devices.iter().any(|device| device.id == saved);
                let saved_opened = saved_is_physical && webcam.start(&saved).is_ok();
                let selected = choose_video_device(&saved, &devices, saved_opened);
                let opened = if saved_opened {
                    selected
                } else if let Some(selected) = selected {
                    match webcam.start(&selected) {
                        Ok(()) => Some(selected),
                        Err(error) => {
                            state.record(format!("Webcam initialization failed: {error}"));
                            None
                        }
                    }
                } else {
                    None
                };
                if let Some(selected) = opened {
                    state.config.selected_video_device_id = selected;
                    state.snapshot.selected_video_device_id =
                        state.config.selected_video_device_id.clone();
                    state.snapshot.webcam_state = DeviceState::Initializing;
                    state
                        .snapshot
                        .webcam_component
                        .waiting_for_first_frame(now, now + WEBCAM_FIRST_FRAME_TIMEOUT);
                    state.snapshot.webcam_native_format =
                        webcam.selected_native_format().map(str::to_owned);
                    state.snapshot.webcam_output_format =
                        webcam.selected_output_format().map(str::to_owned);
                    state.record("Webcam initialized");
                } else if devices.is_empty() {
                    state
                        .snapshot
                        .webcam_component
                        .transition(ComponentLifecycle::Stopped, now);
                    state.record("No physical webcam found");
                } else {
                    state
                        .snapshot
                        .webcam_component
                        .transition(ComponentLifecycle::Stopped, now);
                    state.record("Webcam selection required");
                }
            }
            Err(error) => {
                state.snapshot.webcam_state = DeviceState::Failed;
                state.snapshot.webcam_component.mark_failed_with_kind(
                    now,
                    ComponentFailureKind::Webcam(webcam_failure_kind(&error)),
                    error.clone(),
                );
                state.record(format!("Webcam enumeration failed: {error}"));
            }
        }
        let mut screen = WindowsGraphicsScreenInput::default();
        state
            .snapshot
            .screen_component
            .transition(ComponentLifecycle::Starting, now);
        let selected_monitor = match screen.enumerate() {
            Ok(monitors) => {
                state.snapshot.monitors = monitors.clone().into();
                choose_initial_monitor(&state.config.selected_monitor_label, &monitors).and_then(
                    |monitor| match screen.start(&monitor, state.config.cursor_visible) {
                        Ok(()) => {
                            state.snapshot.screen_state = DeviceState::Initializing;
                            state
                                .snapshot
                                .screen_component
                                .waiting_for_first_frame(now, now + SCREEN_FIRST_FRAME_TIMEOUT);
                            state.record(format!("Screen capture initialized: {}", monitor.label));
                            Some(monitor)
                        }
                        Err(error) => {
                            state.snapshot.screen_state = DeviceState::Failed;
                            state.snapshot.screen_component.mark_failed_with_kind(
                                now,
                                ComponentFailureKind::Screen(screen_failure_kind(&error)),
                                error.clone(),
                            );
                            state.record(format!("Screen capture failed: {error}"));
                            None
                        }
                    },
                )
            }
            Err(error) => {
                state.snapshot.screen_state = DeviceState::Failed;
                state.snapshot.screen_component.mark_failed_with_kind(
                    now,
                    ComponentFailureKind::Screen(screen_failure_kind(&error)),
                    error.clone(),
                );
                state.record(format!("Monitor enumeration failed: {error}"));
                None
            }
        };
        let mut monitor_tracker = MonitorTracker::new(MonitorTrackerSettings {
            match_threshold: state.config.similarity_threshold,
        });
        if let Some(monitor) = selected_monitor.clone() {
            monitor_tracker.select(monitor);
        }
        if let Some(monitor) = selected_monitor.as_ref() {
            state
                .config
                .selected_monitor_label
                .clone_from(&monitor.label);
        }
        state.snapshot.selected_monitor = selected_monitor.clone();
        let selected_display_hdr_unsupported = selected_monitor.as_ref().is_some_and(|monitor| {
            match screen.display_uses_hdr_or_ten_bit(monitor) {
                Ok(unsupported) => unsupported,
                Err(error) => {
                    state.record(format!("Display color capability check failed: {error}"));
                    false
                }
            }
        });
        if selected_display_hdr_unsupported {
            state.snapshot.warning = Some(
                "HDR or 10-bit color is enabled on the selected display; disable HDR in Windows Display settings before using automatic matching or reference capture"
                    .into(),
            );
        }
        let monitor_scan_worker = match MonitorScanWorker::start() {
            Ok(worker) => Some(worker),
            Err(error) => {
                state.record(error);
                None
            }
        };
        let mut platform = Self {
            publisher,
            camera,
            pipe_name,
            webcam,
            screen,
            selected_monitor,
            selected_display_hdr_unsupported,
            monitor_tracker,
            screen_capture_recovery: ScreenCaptureRecovery::default(),
            last_monitor_scan: Instant::now(),
            last_screen_capture_recovery_check: Instant::now(),
            monitor_scan_generation: 1,
            monitor_scan_worker,
        };
        if state.config.automatic_monitor_rescans {
            platform.request_monitor_scan(state, state.config.cursor_visible);
        }
        platform
    }

    fn command(&mut self, command: &Command, state: &mut RuntimeState) {
        match command {
            Command::UpdateSettings(config) | Command::ReloadSettings(config) => {
                let reload_settings = matches!(command, Command::ReloadSettings(_));
                if config.selected_video_device_id != state.config.selected_video_device_id {
                    let now = Instant::now();
                    self.webcam.stop();
                    if config.selected_video_device_id.is_empty() {
                        state.snapshot.webcam_state = DeviceState::Unavailable;
                        state
                            .snapshot
                            .webcam_component
                            .transition(ComponentLifecycle::Stopped, now);
                    } else {
                        match self.webcam.start(&config.selected_video_device_id) {
                            Ok(()) => {
                                state.snapshot.webcam_state = DeviceState::Initializing;
                                state
                                    .snapshot
                                    .webcam_component
                                    .waiting_for_first_frame(now, now + WEBCAM_FIRST_FRAME_TIMEOUT);
                                state.snapshot.webcam_native_format =
                                    self.webcam.selected_native_format().map(str::to_owned);
                                state.snapshot.webcam_output_format =
                                    self.webcam.selected_output_format().map(str::to_owned);
                            }
                            Err(error) => {
                                state.snapshot.webcam_state = DeviceState::Failed;
                                state.snapshot.webcam_component.mark_failed_with_kind(
                                    now,
                                    ComponentFailureKind::Webcam(webcam_failure_kind(&error)),
                                    error.clone(),
                                );
                                state.record(format!("Webcam selection failed: {error}"));
                            }
                        }
                    }
                }
                let cursor_changed = config.cursor_visible != state.config.cursor_visible;
                let threshold_changed =
                    config.similarity_threshold != state.config.similarity_threshold;
                let automatic_rescans_changed =
                    config.automatic_monitor_rescans != state.config.automatic_monitor_rescans;
                let automatic_recovery_changed = config.automatic_screen_capture_recovery
                    != state.config.automatic_screen_capture_recovery;
                if cursor_changed {
                    let old = state.config.cursor_visible;
                    state.config.cursor_visible = config.cursor_visible;
                    self.restart_screen(state);
                    state.config.cursor_visible = old;
                }
                if reload_settings {
                    self.invalidate_monitor_scans(config.similarity_threshold);
                } else if cursor_changed || threshold_changed || automatic_rescans_changed {
                    self.invalidate_monitor_scans(config.similarity_threshold);
                    if config.automatic_monitor_rescans {
                        self.request_monitor_scan(state, config.cursor_visible);
                    }
                }
                if automatic_recovery_changed {
                    self.screen_capture_recovery.reset();
                    self.last_screen_capture_recovery_check = Instant::now();
                }
            }
            Command::CaptureReference => {
                if self.selected_display_hdr_unsupported {
                    state.record(
                        "Reference capture is unavailable while HDR or 10-bit color is enabled",
                    );
                    return;
                }
                if let Some(frame) = self.screen.latest_frame() {
                    self.commit_reference(state, frame, "Reference captured");
                } else {
                    state.record("Reference capture failed: no screen frame");
                }
            }
            Command::CaptureReferenceCandidate => {
                if self.selected_display_hdr_unsupported {
                    state.record(
                        "Reference capture is unavailable while HDR or 10-bit color is enabled",
                    );
                    return;
                }
                if let Some(frame) = self.screen.latest_frame() {
                    state.stage_reference_candidate(frame);
                    state.record("Reference candidate captured for review");
                } else {
                    state.record("Reference candidate capture failed: no screen frame");
                }
            }
            Command::ConfirmReferenceCandidate => match state.confirm_reference_candidate() {
                Ok(()) => {
                    state.record("Reference candidate confirmed");
                    self.invalidate_monitor_scans(state.config.similarity_threshold);
                    if state.config.automatic_monitor_rescans {
                        self.request_monitor_scan(state, state.config.cursor_visible);
                    }
                }
                Err(error) => {
                    state.record(format!("Reference candidate confirmation failed: {error}"));
                }
            },
            Command::DiscardReferenceCandidate => {
                state.discard_reference_candidate();
                state.record("Reference candidate discarded");
            }
            Command::ImportReference(path) => {
                match import_reference(path, &state.config.reference_image_path) {
                    Ok((reference, preview)) => {
                        state.install_reference(reference, &preview, Instant::now());
                        state.record("Reference imported into local storage");
                        self.invalidate_monitor_scans(state.config.similarity_threshold);
                        if state.config.automatic_monitor_rescans {
                            self.request_monitor_scan(state, state.config.cursor_visible);
                        }
                    }
                    Err(error) => state.record(format!("Reference import failed: {error}")),
                }
            }
            Command::SelectMonitor(monitor) => {
                let available = self.screen.enumerate().unwrap_or_default();
                if let Some(monitor) = available
                    .into_iter()
                    .find(|candidate| candidate.display_name == monitor.display_name)
                {
                    self.selected_monitor = Some(monitor.clone());
                    self.selected_display_hdr_unsupported = self
                        .screen
                        .display_uses_hdr_or_ten_bit(&monitor)
                        .unwrap_or(false);
                    self.monitor_tracker.select(monitor.clone());
                    self.monitor_scan_generation = self.monitor_scan_generation.wrapping_add(1);
                    if let Some(worker) = self.monitor_scan_worker.as_mut() {
                        worker.clear_pending();
                    }
                    state
                        .config
                        .selected_monitor_label
                        .clone_from(&monitor.label);
                    state.snapshot.selected_monitor = Some(monitor);
                    self.restart_screen(state);
                } else {
                    state.record("Tracked monitor selection failed: monitor unavailable");
                }
            }
            Command::RefreshVideoDevices => match self.webcam.enumerate() {
                Ok(devices) => {
                    state.snapshot.video_devices = devices
                        .into_iter()
                        .map(|device| stageswap_core::VideoDeviceChoice {
                            id: device.id,
                            name: device.name,
                        })
                        .collect::<Vec<_>>()
                        .into();
                    state.record("Video device list refreshed from settings");
                }
                Err(error) => state.record(format!("Video device refresh failed: {error}")),
            },
            Command::Rescan => {
                self.request_monitor_scan(state, state.config.cursor_visible);
            }
            Command::Stop => {
                if let Some(publisher) = &self.publisher {
                    let _ = publisher.invalidate();
                }
            }
            Command::Restart(RestartTarget::VirtualCamera)
            | Command::Restart(RestartTarget::All) => {
                self.restart_virtual_camera(state);
            }
            Command::Restart(RestartTarget::Webcam) => self.restart_webcam(state),
            Command::Restart(RestartTarget::ScreenCapture) => self.restart_screen(state),
            _ => {}
        }
        if matches!(command, Command::Restart(RestartTarget::All)) {
            self.restart_webcam(state);
            self.restart_screen(state);
        }
    }

    fn commit_reference(
        &mut self,
        state: &mut RuntimeState,
        frame: Arc<Frame>,
        success_message: &str,
    ) {
        let Some(reference) = gray_thumbnail(&frame) else {
            state.record("Reference capture failed: invalid screen frame");
            return;
        };
        state.install_reference(reference, &frame, Instant::now());
        match save_reference(&frame, &state.config.reference_image_path) {
            Ok(()) => state.record(success_message),
            Err(error) => state.record(format!(
                "Reference captured for this session but could not be saved: {error}"
            )),
        }
        self.invalidate_monitor_scans(state.config.similarity_threshold);
        if state.config.automatic_monitor_rescans {
            self.request_monitor_scan(state, state.config.cursor_visible);
        }
    }

    fn restart_webcam(&mut self, state: &mut RuntimeState) {
        let now = Instant::now();
        let id = state.config.selected_video_device_id.clone();
        if state.snapshot.webcam_component.lifecycle == ComponentLifecycle::Restarting {
            return;
        }
        state
            .snapshot
            .webcam_component
            .transition(ComponentLifecycle::Restarting, now);
        self.webcam.stop();
        if id.is_empty() {
            state.snapshot.webcam_state = DeviceState::Unavailable;
            state
                .snapshot
                .webcam_component
                .transition(ComponentLifecycle::Stopped, now);
            state.record("Webcam restart skipped: no video input selected");
            return;
        }
        match self.webcam.start(&id) {
            Ok(()) => {
                state.snapshot.webcam_state = DeviceState::Initializing;
                state
                    .snapshot
                    .webcam_component
                    .waiting_for_first_frame(now, now + WEBCAM_FIRST_FRAME_TIMEOUT);
                state.snapshot.webcam_native_format =
                    self.webcam.selected_native_format().map(str::to_owned);
                state.snapshot.webcam_output_format =
                    self.webcam.selected_output_format().map(str::to_owned);
                state.record("Webcam restarted");
            }
            Err(error) => {
                state.snapshot.webcam_state = DeviceState::Failed;
                state.snapshot.webcam_component.mark_failed_with_kind(
                    now,
                    ComponentFailureKind::Webcam(webcam_failure_kind(&error)),
                    error.clone(),
                );
                state.record(format!("Webcam restart failed: {error}"));
            }
        }
    }

    fn restart_virtual_camera(&mut self, state: &mut RuntimeState) {
        let now = Instant::now();
        if state.snapshot.virtual_camera_component.lifecycle == ComponentLifecycle::Restarting {
            return;
        }
        state
            .snapshot
            .virtual_camera_component
            .transition(ComponentLifecycle::Restarting, now);
        let result = if let Some(camera) = &mut self.camera {
            camera.restart()
        } else if self.publisher.is_none() {
            Err("virtual camera frame publisher is unavailable".into())
        } else if let Some(pipe_name) = &self.pipe_name {
            VirtualCameraController::start(pipe_name.clone())
                .map(|camera| self.camera = Some(camera))
        } else {
            Err("virtual camera pipe is unavailable".into())
        };
        match result {
            Ok(()) => {
                state.snapshot.virtual_camera_state = DeviceState::Ready;
                state.snapshot.virtual_camera_component.mark_ready(now);
                state.snapshot.warning = None;
                state.record("Virtual camera restarted");
            }
            Err(error) => {
                state.snapshot.virtual_camera_state = DeviceState::Failed;
                state
                    .snapshot
                    .virtual_camera_component
                    .mark_failed(now, error.clone());
                state.snapshot.warning = Some(error.clone());
                state.record(format!("Virtual camera restart failed: {error}"));
            }
        }
    }

    fn refresh_inputs(&mut self, state: &mut RuntimeState, now: Instant) {
        self.refresh_monitor_scan(state);
        let (monitor_scan_due, screen_capture_recovery_due) = automatic_screen_tasks_due(
            state.config.automatic_monitor_rescans,
            state.config.automatic_screen_capture_recovery,
            self.last_monitor_scan,
            self.last_screen_capture_recovery_check,
            now,
        );
        if monitor_scan_due {
            self.request_monitor_scan(state, state.config.cursor_visible);
        }
        if screen_capture_recovery_due {
            self.check_screen_capture_recovery(state, now);
        }
        let webcam = self.webcam.latest_frame();
        let webcam_is_stale = webcam.as_ref().is_some_and(|frame| {
            now.saturating_duration_since(frame.received_at) > FRAME_STALE_AFTER
        });
        let webcam = (!webcam_is_stale).then_some(webcam).flatten();
        state.snapshot.availability.camera_ready = webcam.is_some();
        if let Some(error) = self.webcam.last_error() {
            if state.snapshot.webcam_state != DeviceState::Failed {
                state.record(error.clone());
            }
            state.snapshot.webcam_state = DeviceState::Failed;
            state.snapshot.webcam_component.mark_failed_with_kind(
                now,
                ComponentFailureKind::Webcam(webcam_failure_kind(&error)),
                error,
            );
            state.snapshot.previews.webcam = None;
        } else if webcam_is_stale {
            state.snapshot.webcam_state = DeviceState::Failed;
            state
                .snapshot
                .webcam_component
                .transition(ComponentLifecycle::Stale, now);
            state.snapshot.previews.webcam = None;
        } else if webcam.is_some() {
            state.snapshot.webcam_state = DeviceState::Ready;
            let was_ready = state.snapshot.webcam_component.lifecycle == ComponentLifecycle::Ready;
            state.snapshot.webcam_component.mark_ready(now);
            state.snapshot.webcam_component.last_success_at =
                webcam.as_ref().map(|frame| frame.received_at);
            if !was_ready {
                state.record("Webcam first frame received");
            }
        } else if state
            .snapshot
            .webcam_component
            .first_frame_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.webcam.stop();
            state.snapshot.webcam_state = DeviceState::Failed;
            state.snapshot.webcam_component.mark_failed(
                now,
                "webcam did not deliver a frame before the startup deadline",
            );
            state.snapshot.webcam_component.last_failure_kind = Some(ComponentFailureKind::Webcam(
                WebcamFailureKind::DriverFailure,
            ));
            state.snapshot.previews.webcam = None;
            state.record("Webcam first-frame deadline expired");
        }
        if let Some(webcam) = webcam {
            state.snapshot.previews.webcam = Some(state.webcam_crop.apply(
                webcam,
                state.config.crop_webcam_to_16_9,
                self.webcam.native_display_aspect_ratio(),
            ));
        }
        let screen = self.screen.latest_frame();
        let screen_is_stale = screen.as_ref().is_some_and(|frame| {
            now.saturating_duration_since(frame.received_at) > FRAME_STALE_AFTER
        });
        let screen = (!screen_is_stale).then_some(screen).flatten();
        state.snapshot.availability.screen_ready = screen.is_some()
            && !(self.selected_display_hdr_unsupported
                && state.snapshot.mode == stageswap_core::OutputMode::Automatic);
        if self.selected_display_hdr_unsupported
            && state.snapshot.mode == stageswap_core::OutputMode::Automatic
        {
            state.snapshot.screen_state = DeviceState::Failed;
            state.snapshot.screen_component.mark_failed_with_kind(
                now,
                ComponentFailureKind::Screen(ScreenFailureKind::UnsupportedHdr),
                "HDR or 10-bit color must be disabled for automatic matching",
            );
        } else if let Some(error) = self.screen.last_error() {
            if state.snapshot.screen_state != DeviceState::Failed {
                state.record(error.clone());
            }
            state.snapshot.screen_state = DeviceState::Failed;
            state.snapshot.screen_component.mark_failed_with_kind(
                now,
                ComponentFailureKind::Screen(screen_failure_kind(&error)),
                error,
            );
            state.snapshot.previews.screen = None;
        } else if screen_is_stale {
            state.snapshot.screen_state = DeviceState::Failed;
            state
                .snapshot
                .screen_component
                .transition(ComponentLifecycle::Stale, now);
            state.snapshot.previews.screen = None;
        } else if let Some(screen) = screen {
            let received_at = screen.received_at;
            state.snapshot.screen_state = DeviceState::Ready;
            let was_ready = state.snapshot.screen_component.lifecycle == ComponentLifecycle::Ready;
            state.snapshot.screen_component.mark_ready(now);
            state.snapshot.screen_component.last_success_at = Some(received_at);
            state.snapshot.previews.screen = Some(screen);
            if !was_ready {
                state.record("Screen capture first frame received");
            }
        } else if state
            .snapshot
            .screen_component
            .first_frame_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            state.snapshot.screen_state = DeviceState::Failed;
            state.snapshot.screen_component.mark_failed(
                now,
                "screen capture did not deliver a frame before the startup deadline",
            );
            state.snapshot.screen_component.last_failure_kind = Some(ComponentFailureKind::Screen(
                ScreenFailureKind::CaptureFailure,
            ));
            state.snapshot.previews.screen = None;
            state.record("Screen capture first-frame deadline expired");
        }
    }

    fn restart_screen(&mut self, state: &mut RuntimeState) {
        self.restart_screen_with_policy(state, false);
    }

    fn restart_screen_with_policy(&mut self, state: &mut RuntimeState, automatic: bool) {
        let now = Instant::now();
        if automatic
            && state
                .snapshot
                .screen_component
                .next_permitted_retry
                .is_some_and(|retry_at| now < retry_at)
        {
            return;
        }
        state
            .snapshot
            .screen_component
            .transition(ComponentLifecycle::Restarting, now);
        self.screen_capture_recovery.reset();
        self.last_screen_capture_recovery_check = now;
        self.screen.stop();
        let Some(monitor) = self.selected_monitor.as_ref() else {
            state.snapshot.screen_state = DeviceState::Unavailable;
            state.snapshot.screen_component.mark_failed(
                now,
                "selected display is unavailable; automatic monitor reselection is disabled",
            );
            state.snapshot.screen_component.last_failure_kind = Some(ComponentFailureKind::Screen(
                ScreenFailureKind::MissingSelectedMonitor,
            ));
            return;
        };
        match self.screen.start(monitor, state.config.cursor_visible) {
            Ok(()) => {
                state.snapshot.screen_state = DeviceState::Initializing;
                state
                    .snapshot
                    .screen_component
                    .waiting_for_first_frame(now, now + SCREEN_FIRST_FRAME_TIMEOUT);
                state.record("Screen capture restarted");
            }
            Err(error) => {
                state.snapshot.screen_state = DeviceState::Failed;
                let failures = state
                    .snapshot
                    .screen_component
                    .consecutive_restart_failures
                    .saturating_add(1);
                state.snapshot.screen_component.mark_failed_with_kind(
                    now,
                    ComponentFailureKind::Screen(screen_failure_kind(&error)),
                    error.clone(),
                );
                state.snapshot.screen_component.consecutive_restart_failures = failures;
                if automatic {
                    state.snapshot.screen_component.next_permitted_retry =
                        Some(now + screen_restart_backoff(failures));
                }
                state.record(format!("Screen capture restart failed: {error}"));
            }
        }
    }

    fn invalidate_monitor_scans(&mut self, match_threshold: f64) {
        self.monitor_scan_generation = self.monitor_scan_generation.wrapping_add(1);
        if let Some(worker) = self.monitor_scan_worker.as_mut() {
            worker.clear_pending();
        }
        let mut tracker = MonitorTracker::new(MonitorTrackerSettings { match_threshold });
        if let Some(monitor) = self.selected_monitor.clone() {
            tracker.select(monitor);
        }
        self.monitor_tracker = tracker;
    }

    fn check_screen_capture_recovery(&mut self, state: &mut RuntimeState, now: Instant) {
        self.last_screen_capture_recovery_check = now;
        match self.screen_capture_recovery.observe(
            self.screen.latest_frame().as_deref().filter(|frame| {
                now.saturating_duration_since(frame.received_at) <= FRAME_STALE_AFTER
            }),
        ) {
            ScreenCaptureRecoveryObservation::Clear => {}
            ScreenCaptureRecoveryObservation::AwaitingConfirmation => {
                state.record(
                    "Screen capture is black or unavailable; awaiting the next automatic recovery check",
                );
            }
            ScreenCaptureRecoveryObservation::Restart => {
                state.record(
                    "Screen capture remains black or unavailable; restarting screen capture",
                );
                self.restart_screen_with_policy(state, true);
            }
        }
    }

    fn request_monitor_scan(&mut self, state: &RuntimeState, cursor_visible: bool) {
        if !self.queue_monitor_scan(state, cursor_visible) {
            self.last_monitor_scan = Instant::now();
        }
    }

    fn queue_monitor_scan(&mut self, state: &RuntimeState, cursor_visible: bool) -> bool {
        let Some(worker) = &mut self.monitor_scan_worker else {
            return false;
        };
        if worker.request(MonitorScanRequest {
            generation: self.monitor_scan_generation,
            reference: state.reference.clone(),
            cursor_visible,
        }) {
            self.last_monitor_scan = Instant::now();
            true
        } else {
            false
        }
    }

    fn refresh_monitor_scan(&mut self, state: &mut RuntimeState) {
        let Some(result) = self
            .monitor_scan_worker
            .as_mut()
            .and_then(MonitorScanWorker::poll)
        else {
            return;
        };
        if result.generation != self.monitor_scan_generation {
            return;
        }
        state.snapshot.monitors = result.monitors.into();
        if result.scores.is_empty() {
            return;
        }
        let tracking = self.monitor_tracker.apply_scan(&result.scores);
        if tracking.confirmation_pending {
            self.queue_monitor_scan(state, state.config.cursor_visible);
            return;
        }
        if let Some(monitor) = tracking.tracked
            && self.selected_monitor.as_ref() != Some(&monitor)
        {
            self.selected_monitor = Some(monitor);
            self.selected_display_hdr_unsupported = self
                .selected_monitor
                .as_ref()
                .and_then(|monitor| self.screen.display_uses_hdr_or_ten_bit(monitor).ok())
                .unwrap_or(false);
            state.snapshot.selected_monitor = self.selected_monitor.clone();
            if let Some(monitor) = self.selected_monitor.as_ref() {
                state
                    .config
                    .selected_monitor_label
                    .clone_from(&monitor.label);
            }
            self.restart_screen(state);
            state.record("Reference monitor changed after two scans");
        }
    }

    fn publish(&self, state: &mut RuntimeState, now: Instant) {
        let Some(frame) = state.snapshot.previews.final_output.as_deref() else {
            return;
        };
        if let Some(publisher) = &self.publisher {
            match publisher.publish(frame) {
                Ok(()) => state.snapshot.publisher_component.mark_ready(now),
                Err(error) => {
                    state
                        .snapshot
                        .publisher_component
                        .mark_failed(now, error.clone());
                    if state.snapshot.warning.as_deref() != Some(error.as_str()) {
                        state.snapshot.warning = Some(error.clone());
                        state.record(format!("Frame publish failed: {error}"));
                    }
                }
            }
        }
        if self
            .camera
            .as_ref()
            .is_some_and(|camera| !camera.is_running())
        {
            state.snapshot.virtual_camera_state = DeviceState::Failed;
            state
                .snapshot
                .virtual_camera_component
                .mark_failed(now, "virtual camera stopped unexpectedly");
        }
    }
}

#[cfg(windows)]
impl Drop for Platform {
    fn drop(&mut self) {
        if let Some(publisher) = &self.publisher {
            let _ = publisher.invalidate();
        }
    }
}

#[cfg(not(windows))]
struct Platform;

#[cfg(not(windows))]
impl Platform {
    fn new(_state: &mut RuntimeState) -> Self {
        Self
    }
    fn command(&mut self, _command: &Command, _state: &mut RuntimeState) {}
    fn refresh_inputs(&mut self, _state: &mut RuntimeState, _now: Instant) {}
    fn publish(&self, _state: &mut RuntimeState, _now: Instant) {}
}

impl RuntimePorts for Platform {
    fn command(&mut self, command: &Command, state: &mut RuntimeState) {
        Platform::command(self, command, state);
    }

    fn refresh_inputs(&mut self, state: &mut RuntimeState, now: Instant) {
        Platform::refresh_inputs(self, state, now);
    }

    fn publish(&self, state: &mut RuntimeState, now: Instant) {
        Platform::publish(self, state, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_clock::VirtualRuntimeClock;
    use stageswap_core::OutputMode;

    #[derive(Default)]
    struct FakeComponentPort {
        ready_at: Option<Instant>,
        terminal_failure_at: Option<Instant>,
        starts: u32,
        restarts: u32,
    }

    #[derive(Default)]
    struct FakeRuntimePorts {
        webcam: FakeComponentPort,
        screen: FakeComponentPort,
        publisher: FakeComponentPort,
        virtual_camera: FakeComponentPort,
    }

    impl RuntimePorts for FakeRuntimePorts {
        fn command(&mut self, command: &Command, _state: &mut RuntimeState) {
            match command {
                Command::Start => {
                    self.webcam.starts += 1;
                    self.screen.starts += 1;
                    self.publisher.starts += 1;
                    self.virtual_camera.starts += 1;
                }
                Command::Restart(stageswap_core::RestartTarget::Webcam) => {
                    self.webcam.restarts += 1;
                }
                Command::Restart(stageswap_core::RestartTarget::ScreenCapture) => {
                    self.screen.restarts += 1;
                }
                _ => {}
            }
        }

        fn refresh_inputs(&mut self, state: &mut RuntimeState, now: Instant) {
            for (component, status) in [
                (&self.webcam, &mut state.snapshot.webcam_component),
                (&self.screen, &mut state.snapshot.screen_component),
            ] {
                if component
                    .terminal_failure_at
                    .is_some_and(|failure_at| now >= failure_at)
                {
                    status.mark_failed(now, "scripted terminal failure");
                } else if component.ready_at.is_some_and(|ready_at| now >= ready_at) {
                    status.mark_ready(now);
                } else if status
                    .first_frame_deadline
                    .is_some_and(|deadline| now >= deadline)
                {
                    status.mark_failed(now, "scripted first-frame deadline");
                }
            }
        }

        fn publish(&self, _state: &mut RuntimeState, _now: Instant) {}
    }

    fn monitor(display_name: &str, label: &str) -> MonitorDescriptor {
        MonitorDescriptor {
            display_name: display_name.into(),
            label: label.into(),
            ..MonitorDescriptor::default()
        }
    }

    #[test]
    fn runtime_engine_uses_virtual_time_for_delayed_frames_and_deadlines() {
        let start = Instant::now();
        let clock = VirtualRuntimeClock::new(start);
        let mut state = RuntimeState::new_at(AppConfig::default(), start);
        state
            .snapshot
            .webcam_component
            .waiting_for_first_frame(start, start + WEBCAM_FIRST_FRAME_TIMEOUT);
        state
            .snapshot
            .screen_component
            .waiting_for_first_frame(start, start + SCREEN_FIRST_FRAME_TIMEOUT);
        let platform = FakeRuntimePorts {
            webcam: FakeComponentPort {
                ready_at: Some(start + Duration::from_millis(2_900)),
                ..FakeComponentPort::default()
            },
            ..FakeRuntimePorts::default()
        };
        let mut engine = RuntimeEngine::from_parts(state, platform, start);
        assert!(engine.command(Command::Start));
        assert_eq!(engine.platform.webcam.starts, 1);
        assert_eq!(engine.platform.screen.starts, 1);
        assert_eq!(engine.platform.publisher.starts, 1);
        assert_eq!(engine.platform.virtual_camera.starts, 1);
        assert!(engine.command(Command::Restart(stageswap_core::RestartTarget::Webcam)));
        assert_eq!(engine.platform.webcam.restarts, 1);

        clock.advance(Duration::from_secs(2));
        assert!(engine.step(clock.now()));
        assert_eq!(
            engine.state.snapshot.screen_component.lifecycle,
            ComponentLifecycle::Failed
        );
        assert_eq!(
            engine.state.snapshot.webcam_component.lifecycle,
            ComponentLifecycle::WaitingForFirstFrame
        );

        clock.advance(Duration::from_millis(900));
        assert!(engine.step(clock.now()));
        assert_eq!(
            engine.state.snapshot.webcam_component.lifecycle,
            ComponentLifecycle::Ready
        );
    }

    #[test]
    fn initial_monitor_prefers_saved_label_then_secondary() {
        let monitors = [
            monitor("primary", "Desk"),
            monitor("secondary", "Stage"),
            monitor("third", "Stage"),
        ];

        assert_eq!(
            choose_initial_monitor("Desk", &monitors)
                .unwrap()
                .display_name,
            "primary"
        );
        assert_eq!(
            choose_initial_monitor("Stage", &monitors)
                .unwrap()
                .display_name,
            "secondary"
        );
        assert_eq!(
            choose_initial_monitor("Missing", &monitors)
                .unwrap()
                .display_name,
            "secondary"
        );
        assert_eq!(
            choose_initial_monitor("", &monitors).unwrap().display_name,
            "secondary"
        );
    }

    #[test]
    fn initial_monitor_uses_sole_primary_or_none() {
        let primary = [monitor("primary", "Desk")];
        assert_eq!(
            choose_initial_monitor("Missing", &primary)
                .unwrap()
                .display_name,
            "primary"
        );
        assert!(choose_initial_monitor("Missing", &[]).is_none());
    }

    #[test]
    fn output_fps_tracker_measures_generated_output_independently_of_ui() {
        let start = Instant::now();
        let mut tracker = OutputFpsTracker::default();
        let mut reading = None;
        for frame in 0..=30 {
            reading = tracker.observe(start + Duration::from_secs_f64(frame as f64 / 30.0));
        }
        assert_eq!(reading, Some(30));

        let resumed = tracker.observe(start + Duration::from_secs(3));
        assert_eq!(resumed, None);
    }

    #[test]
    fn disco_effect_changes_only_rgb_and_preserves_frame_metadata() {
        let now = Instant::now();
        let source = Frame::placeholder(Size::new(32, 18), 0xff30_6090, 17, 42, now);
        let mut effect = DiscoEffect::new(source.size);

        let first = effect.apply(&source, Duration::ZERO);
        let later = effect.apply(&source, Duration::from_millis(990));

        assert_eq!(first.size, source.size);
        assert_eq!(first.stride, source.stride);
        assert_eq!(first.sequence, source.sequence);
        assert_eq!(first.timestamp_100ns, source.timestamp_100ns);
        assert_eq!(first.received_at, source.received_at);
        assert!(first.pixels().chunks_exact(4).all(|pixel| pixel[3] == 0xff));
        assert!(later.pixels().chunks_exact(4).all(|pixel| pixel[3] == 0xff));
        assert_ne!(first.pixels(), source.pixels());
        assert_ne!(later.pixels(), first.pixels());
    }

    #[test]
    fn disco_flash_pattern_has_primary_secondary_and_major_hits() {
        assert_eq!(disco_flash_lift(0), 38);
        assert_eq!(disco_flash_lift(1), 26);
        assert_eq!(disco_flash_lift(8), 22);
        assert_eq!(disco_flash_lift(9), 8);
        assert_eq!(disco_flash_lift(12), 0);
        assert_eq!(disco_flash_lift(36), 30);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn disco_effect_keeps_release_720p_processing_inside_the_frame_budget() {
        let now = Instant::now();
        let source = Frame::placeholder(PIPELINE_SIZE, 0xff30_6090, 1, 0, now);
        let mut effect = DiscoEffect::new(PIPELINE_SIZE);
        let started_at = Instant::now();
        for sequence in 0..300 {
            std::hint::black_box(effect.apply(
                &source,
                Duration::from_nanos(sequence * 1_000_000_000 / u64::from(PIPELINE_FPS)),
            ));
        }
        assert!(
            started_at.elapsed() <= Duration::from_secs(10),
            "300 disco frames exceeded the 30 fps processing budget"
        );
    }

    #[test]
    fn source_fps_tracker_measures_capture_rate_and_reports_stalls() {
        let start = Instant::now();
        let mut tracker = SourceFpsTracker::default();
        let mut reading = None;
        for sequence in 1..=31 {
            let received_at = start + Duration::from_secs_f64((sequence - 1) as f64 / 30.0);
            let frame = Frame::placeholder(Size::new(1, 1), 0xff00_0000, sequence, 0, received_at);
            reading = tracker.observe(Some(&frame), received_at);
        }
        assert_eq!(reading, Some(30));

        let last = Frame::placeholder(
            Size::new(1, 1),
            0xff00_0000,
            31,
            0,
            start + Duration::from_secs(1),
        );
        assert_eq!(
            tracker.observe(Some(&last), start + Duration::from_secs(2)),
            Some(0)
        );
    }

    #[test]
    fn webcam_crop_is_aspect_aware_and_shares_processed_output() {
        let now = Instant::now();
        let size = Size::new(1280, 720);
        let mut pixels = vec![0; size.width as usize * size.height as usize * 4];
        for y in 0..size.height as usize {
            for x in 0..size.width as usize {
                let offset = (y * size.width as usize + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(
                    if !(160..1120).contains(&x) || !(90..630).contains(&y) {
                        &[0, 0, 255, 255]
                    } else {
                        &[0, 255, 0, 255]
                    },
                );
            }
        }
        let raw = Arc::new(Frame::new(pixels.into(), size, size.width * 4, 9, 321, now).unwrap());
        let mut state = RuntimeState::new(AppConfig::default());
        let processed = state
            .webcam_crop
            .apply(Arc::clone(&raw), true, Some(4.0 / 3.0));
        let cached = state
            .webcam_crop
            .apply(Arc::clone(&raw), true, Some(4.0 / 3.0));
        assert!(!Arc::ptr_eq(&processed, &raw));
        assert!(Arc::ptr_eq(&processed, &cached));
        assert_eq!(processed.size, size);
        assert_eq!(processed.sequence, 9);
        assert_eq!(processed.timestamp_100ns, 321);
        assert_eq!(processed.received_at, now);
        assert!(
            processed
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel == [0, 255, 0, 255])
        );
        assert!(Arc::ptr_eq(
            &state
                .webcam_crop
                .apply(Arc::clone(&raw), false, Some(4.0 / 3.0)),
            &raw
        ));
        assert!(Arc::ptr_eq(
            &state
                .webcam_crop
                .apply(Arc::clone(&raw), true, Some(16.0 / 9.0)),
            &raw
        ));
        assert!(Arc::ptr_eq(
            &state.webcam_crop.apply(Arc::clone(&raw), true, None),
            &raw
        ));

        state.snapshot.previews.webcam = Some(Arc::clone(&processed));
        state.snapshot.availability.camera_ready = true;
        state.snapshot.run_state = RunState::Running;
        state.snapshot.mode = OutputMode::ForceCamera;
        state.tick(now + Duration::from_millis(33));

        let output = state
            .snapshot
            .previews
            .final_output
            .as_ref()
            .expect("camera output is present");
        assert!(Arc::ptr_eq(&processed.pixels_arc(), &output.pixels_arc()));
    }

    #[test]
    fn webcam_crop_tolerance_and_cache_include_native_aspect_ratio() {
        let now = Instant::now();
        let size = Size::new(1280, 720);
        let pixels = (0..size.height)
            .flat_map(|y| {
                (0..size.width).flat_map(move |x| {
                    let value = ((x + y) % 255) as u8;
                    [value, value, value, 255]
                })
            })
            .collect::<Vec<_>>();
        let raw = Arc::new(Frame::new(pixels.into(), size, size.width * 4, 1, 2, now).unwrap());
        let mut crop = WebcamCropCache::default();

        let within_tolerance =
            TARGET_WEBCAM_ASPECT_RATIO * (1.0 + WEBCAM_ASPECT_RATIO_TOLERANCE / 2.0);
        assert!(Arc::ptr_eq(
            &crop.apply(Arc::clone(&raw), true, Some(within_tolerance)),
            &raw
        ));

        let four_by_three = crop.apply(Arc::clone(&raw), true, Some(4.0 / 3.0));
        let square = crop.apply(Arc::clone(&raw), true, Some(1.0));
        let wider = crop.apply(Arc::clone(&raw), true, Some(21.0 / 9.0));
        assert!(!Arc::ptr_eq(&four_by_three, &square));
        assert_ne!(four_by_three.pixels(), square.pixels());
        assert_ne!(square.pixels(), wider.pixels());
    }

    fn frame_with_bright_pixels(bright_pixels: usize) -> Frame {
        let size = Size::new(160, 90);
        let mut pixels = vec![0; size.width as usize * size.height as usize * 4];
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let value = u8::from(index < bright_pixels) * 255;
            pixel.copy_from_slice(&[value, value, value, 255]);
        }
        Frame::new(pixels.into(), size, size.width * 4, 1, 0, Instant::now()).unwrap()
    }

    fn solid_frame(value: u8) -> Frame {
        let size = Size::new(160, 90);
        let mut pixels = vec![value; size.width as usize * size.height as usize * 4];
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        Frame::new(pixels.into(), size, size.width * 4, 1, 0, Instant::now()).unwrap()
    }

    #[test]
    fn black_screen_detection_allows_small_cursor_but_rejects_visible_content() {
        assert!(is_nearly_black(&frame_with_bright_pixels(0)));
        assert!(is_nearly_black(&solid_frame(BLACK_LUMA_THRESHOLD)));
        assert!(!is_nearly_black(&solid_frame(BLACK_LUMA_THRESHOLD + 1)));
        assert!(is_nearly_black(&frame_with_bright_pixels(100)));
        assert!(!is_nearly_black(&frame_with_bright_pixels(200)));
    }

    #[test]
    fn illustrated_jw_library_idle_display_is_not_treated_as_failed_capture() {
        let mut image =
            image::load_from_memory(include_bytes!("../assets/setup-reference-example.png"))
                .unwrap()
                .to_rgba8();
        for pixel in image.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        let size = Size::new(image.width(), image.height());
        let frame = Frame::new(
            image.into_raw().into(),
            size,
            size.width * 4,
            1,
            0,
            Instant::now(),
        )
        .unwrap();
        let mut recovery = ScreenCaptureRecovery::default();

        assert!(!is_nearly_black(&frame));
        assert_eq!(
            recovery.observe(Some(&frame)),
            ScreenCaptureRecoveryObservation::Clear
        );
        assert_eq!(
            recovery.observe(Some(&frame)),
            ScreenCaptureRecoveryObservation::Clear
        );
    }

    #[test]
    fn two_missing_frames_restart_screen_capture() {
        let mut recovery = ScreenCaptureRecovery::default();

        assert_eq!(
            recovery.observe(None),
            ScreenCaptureRecoveryObservation::AwaitingConfirmation
        );
        assert_eq!(
            recovery.observe(None),
            ScreenCaptureRecoveryObservation::Restart
        );
    }

    #[test]
    fn visible_frame_clears_a_single_missing_frame() {
        let visible = frame_with_bright_pixels(200);
        let mut recovery = ScreenCaptureRecovery::default();

        assert_eq!(
            recovery.observe(None),
            ScreenCaptureRecoveryObservation::AwaitingConfirmation
        );
        assert_eq!(
            recovery.observe(Some(&visible)),
            ScreenCaptureRecoveryObservation::Clear
        );
    }

    #[test]
    fn near_black_then_missing_frame_restarts_screen_capture() {
        let black = frame_with_bright_pixels(0);
        let mut recovery = ScreenCaptureRecovery::default();

        assert_eq!(
            recovery.observe(Some(&black)),
            ScreenCaptureRecoveryObservation::AwaitingConfirmation
        );
        assert_eq!(
            recovery.observe(None),
            ScreenCaptureRecoveryObservation::Restart
        );
    }

    #[test]
    fn two_near_black_frames_restart_and_reset_screen_capture_recovery() {
        let black = frame_with_bright_pixels(0);
        let mut recovery = ScreenCaptureRecovery::default();

        assert_eq!(
            recovery.observe(Some(&black)),
            ScreenCaptureRecoveryObservation::AwaitingConfirmation
        );
        assert_eq!(
            recovery.observe(Some(&black)),
            ScreenCaptureRecoveryObservation::Restart
        );
        assert_eq!(
            recovery.observe(Some(&black)),
            ScreenCaptureRecoveryObservation::AwaitingConfirmation
        );
    }

    #[test]
    fn display_discovery_and_screen_capture_recovery_are_scheduled_independently() {
        let started_at = Instant::now();
        let due_at = started_at + AUTOMATIC_SCREEN_CHECK_INTERVAL;
        assert_eq!(
            automatic_screen_tasks_due(false, false, started_at, started_at, due_at),
            (false, false)
        );
        assert_eq!(
            automatic_screen_tasks_due(true, false, started_at, started_at, due_at),
            (true, false)
        );
        assert_eq!(
            automatic_screen_tasks_due(false, true, started_at, started_at, due_at),
            (false, true)
        );
        assert_eq!(
            automatic_screen_tasks_due(true, true, started_at, started_at, due_at),
            (true, true)
        );
        assert_eq!(
            automatic_screen_tasks_due(
                true,
                true,
                due_at,
                started_at,
                due_at + Duration::from_millis(1),
            ),
            (false, true)
        );
    }

    #[test]
    fn screen_restart_backoff_is_bounded_and_resets_externally_on_success() {
        assert_eq!(screen_restart_backoff(1), Duration::from_secs(5));
        assert_eq!(screen_restart_backoff(2), Duration::from_secs(10));
        assert_eq!(screen_restart_backoff(3), Duration::from_secs(20));
        assert_eq!(screen_restart_backoff(10), Duration::from_secs(60));
    }

    #[test]
    fn reference_detection_is_gray_but_preview_stays_in_color() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reference.png");
        let image = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        image.save(&path).unwrap();

        let (reference, preview) = load_reference(path.to_str().unwrap()).unwrap();

        assert_eq!(reference.size, Size::new(160, 90));
        assert_eq!(preview.size, Size::new(2, 1));
        assert_eq!(preview.pixels(), &[0, 0, 255, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn reference_candidate_is_isolated_discardable_and_committed_exactly() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reference.png");
        let config = AppConfig {
            reference_image_path: path.display().to_string(),
            ..AppConfig::default()
        };
        let mut state = RuntimeState::new(config);
        let original = Arc::new(solid_frame(32));
        state.snapshot.previews.reference = Some(Arc::clone(&original));
        let candidate = Arc::new(solid_frame(176));

        state.stage_reference_candidate(Arc::clone(&candidate));
        assert!(Arc::ptr_eq(
            state.snapshot.previews.reference.as_ref().unwrap(),
            &original
        ));
        assert!(Arc::ptr_eq(
            state
                .snapshot
                .previews
                .reference_candidate
                .as_ref()
                .unwrap(),
            &candidate
        ));

        state.discard_reference_candidate();
        assert!(state.snapshot.previews.reference_candidate.is_none());
        assert!(Arc::ptr_eq(
            state.snapshot.previews.reference.as_ref().unwrap(),
            &original
        ));

        state.stage_reference_candidate(Arc::clone(&candidate));
        state.confirm_reference_candidate().unwrap();

        assert!(state.snapshot.previews.reference_candidate.is_none());
        assert_eq!(
            state.snapshot.previews.reference.as_ref().unwrap().pixels(),
            candidate.pixels()
        );
        let (_, loaded) = load_reference(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.pixels(), candidate.pixels());
    }

    #[test]
    fn failed_candidate_save_keeps_candidate_and_active_reference_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let invalid_path = directory.path().join("reference-target-is-a-directory");
        std::fs::create_dir(&invalid_path).unwrap();
        let config = AppConfig {
            reference_image_path: invalid_path.display().to_string(),
            ..AppConfig::default()
        };
        let mut state = RuntimeState::new(config);
        let original = Arc::new(solid_frame(32));
        let candidate = Arc::new(solid_frame(176));
        state.snapshot.previews.reference = Some(Arc::clone(&original));
        state.stage_reference_candidate(Arc::clone(&candidate));

        assert!(state.confirm_reference_candidate().is_err());
        assert!(Arc::ptr_eq(
            state.snapshot.previews.reference.as_ref().unwrap(),
            &original
        ));
        assert!(Arc::ptr_eq(
            state
                .snapshot
                .previews
                .reference_candidate
                .as_ref()
                .unwrap(),
            &candidate
        ));
    }

    #[test]
    fn commands_publish_immutable_snapshots() {
        let runtime = RuntimeHandle::spawn(AppConfig {
            start_automatically: false,
            ..AppConfig::default()
        });
        assert!(runtime.send(Command::Start).is_accepted());
        assert!(
            runtime
                .send(Command::SetMode(OutputMode::ForceScreen))
                .is_accepted()
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let snapshot = loop {
            let snapshot = runtime.snapshot();
            if snapshot.run_state == RunState::Running
                && snapshot.mode == OutputMode::ForceScreen
                && snapshot.previews.final_output.is_some()
            {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "runtime did not publish command results"
            );
            thread::yield_now();
        };
        assert_eq!(snapshot.run_state, RunState::Running);
        assert_eq!(snapshot.mode, OutputMode::ForceScreen);
        assert_eq!(snapshot.actual_output, Source::Placeholder);
        assert!(snapshot.previews.final_output.is_some());
    }

    #[test]
    fn disco_mode_is_session_only_and_never_decorates_the_stopped_output() {
        let runtime = RuntimeHandle::spawn(AppConfig {
            start_automatically: false,
            ..AppConfig::default()
        });
        assert!(!runtime.snapshot().disco_enabled);

        assert!(runtime.send(Command::ToggleDisco).is_accepted());
        let deadline = Instant::now() + Duration::from_secs(1);
        let enabled = loop {
            let snapshot = runtime.snapshot();
            if snapshot.disco_enabled {
                break snapshot;
            }
            assert!(Instant::now() < deadline, "disco mode was not enabled");
            thread::yield_now();
        };
        assert_eq!(enabled.run_state, RunState::Stopped);
        assert_eq!(
            enabled
                .previews
                .final_output
                .expect("stopped output is present")
                .pixels(),
            stageswap_core::off_frame_pixels().as_ref()
        );

        assert!(runtime.send(Command::ToggleDisco).is_accepted());
        loop {
            if !runtime.snapshot().disco_enabled {
                break;
            }
            assert!(Instant::now() < deadline, "disco mode was not disabled");
            thread::yield_now();
        }
    }

    #[test]
    fn stopped_runtime_uses_the_canonical_off_frame_and_running_resumes_output() {
        let runtime = RuntimeHandle::spawn(AppConfig {
            start_automatically: false,
            ..AppConfig::default()
        });
        let initial = runtime.snapshot();
        let initial_output = initial
            .previews
            .final_output
            .expect("stopped runtime should start with an off frame");
        assert_eq!(initial.run_state, RunState::Stopped);
        assert_eq!(initial.actual_output, Source::Placeholder);
        assert_eq!(
            initial_output.pixels(),
            stageswap_core::off_frame_pixels().as_ref()
        );

        assert!(runtime.send(Command::Start).is_accepted());
        let deadline = Instant::now() + Duration::from_secs(2);
        let running_sequence = loop {
            let snapshot = runtime.snapshot();
            if snapshot.run_state == RunState::Running
                && let Some(output) = snapshot.previews.final_output
                && output.sequence > initial_output.sequence
            {
                break output.sequence;
            }
            assert!(Instant::now() < deadline, "runtime did not resume output");
            thread::yield_now();
        };

        assert!(runtime.send(Command::Stop).is_accepted());
        let stopped = loop {
            let snapshot = runtime.snapshot();
            if snapshot.run_state == RunState::Stopped
                && snapshot
                    .previews
                    .final_output
                    .as_ref()
                    .is_some_and(|output| output.sequence > running_sequence)
            {
                break snapshot;
            }
            assert!(Instant::now() < deadline, "runtime did not stop");
            thread::yield_now();
        };
        let stopped_output = stopped.previews.final_output.expect("off frame is present");
        assert_eq!(stopped.actual_output, Source::Placeholder);
        assert_eq!(
            stopped_output.pixels(),
            stageswap_core::off_frame_pixels().as_ref()
        );
    }

    #[test]
    fn stopped_runtime_keeps_the_off_output_clock_running() {
        let runtime = RuntimeHandle::spawn(AppConfig {
            start_automatically: false,
            ..AppConfig::default()
        });
        let first = runtime
            .snapshot()
            .previews
            .final_output
            .expect("initial off frame is present");
        let deadline = Instant::now() + Duration::from_secs(1);
        let later = loop {
            let output = runtime
                .snapshot()
                .previews
                .final_output
                .expect("off frame remains present");
            if output.sequence > first.sequence {
                break output;
            }
            assert!(
                Instant::now() < deadline,
                "stopped output clock did not advance"
            );
            thread::yield_now();
        };
        assert_eq!(later.pixels(), stageswap_core::off_frame_pixels().as_ref());
    }

    #[test]
    fn command_traffic_does_not_accelerate_output_clock() {
        let runtime = RuntimeHandle::spawn(AppConfig {
            start_automatically: true,
            ..AppConfig::default()
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let first_sequence = loop {
            if let Some(frame) = runtime.snapshot().previews.final_output {
                break frame.sequence;
            }
            assert!(Instant::now() < deadline, "runtime did not start");
            thread::yield_now();
        };
        for index in 0..40 {
            let mode = if index % 2 == 0 {
                OutputMode::ForceCamera
            } else {
                OutputMode::ForceScreen
            };
            assert!(runtime.send(Command::SetMode(mode)).is_accepted());
            thread::sleep(Duration::from_millis(2));
        }
        thread::sleep(Duration::from_millis(20));
        let last_sequence = runtime
            .snapshot()
            .previews
            .final_output
            .expect("runtime stopped publishing")
            .sequence;
        assert!(
            last_sequence.saturating_sub(first_sequence) <= 5,
            "commands accelerated output from sequence {first_sequence} to {last_sequence}"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires installed COM source, interactive desktop, and a physical webcam"]
    fn all_four_manual_restart_actions_recover_components() {
        let runtime = RuntimeHandle::spawn(AppConfig::default());
        let ready_deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let snapshot = runtime.snapshot();
            if snapshot.webcam_state == DeviceState::Ready
                && snapshot.screen_state == DeviceState::Ready
                && snapshot.virtual_camera_state == DeviceState::Ready
            {
                break;
            }
            assert!(
                Instant::now() < ready_deadline,
                "components did not initialize"
            );
            thread::sleep(Duration::from_millis(25));
        }
        for (target, expected) in [
            (RestartTarget::Webcam, "Webcam restarted"),
            (RestartTarget::ScreenCapture, "Screen capture restarted"),
            (RestartTarget::VirtualCamera, "Virtual camera restarted"),
            (RestartTarget::All, "Restart requested: All"),
        ] {
            assert!(runtime.send(Command::Restart(target)).is_accepted());
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let snapshot = runtime.snapshot();
                if snapshot
                    .recent_activity
                    .iter()
                    .any(|activity| activity.contains(expected))
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "restart did not complete: {target:?}"
                );
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}
