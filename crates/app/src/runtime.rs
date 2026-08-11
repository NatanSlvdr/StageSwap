use crate::runtime_mailbox::{CommandInbox, CommandMailbox};
use crate::{CommandDispatch, RuntimeClock, SystemRuntimeClock};
use stageswap_core::{
    AppConfig, AppSnapshot, CAPTURE_FRAME_POOL_CAPACITY, Command, DebouncedDetector,
    DetectorSettings, DeviceState, Frame, FrameBufferPool, FrameCompositor, FrameMetadata,
    FramePacer, GrayImage, PIPELINE_FPS, PIPELINE_SIZE, RunState, Size, Source, SourceAvailability,
    TransitionController, bgra_to_gray, decide, image_similarity, off_frame, resize_bgra_to_gray,
    resize_bilinear,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool as StdAtomicBool, Ordering as StdOrdering};
use std::sync::mpsc::{self as std_mpsc, Receiver as StdReceiver, SyncSender as StdSyncSender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(any(windows, test))]
use stageswap_core::{MonitorDescriptor, RestartTarget};
#[cfg(windows)]
use stageswap_core::{MonitorScore, MonitorTracker, MonitorTrackerSettings};
#[cfg(windows)]
use stageswap_windows::{
    FramePublisher, FramePublisherDiagnostics, FramePublisherSink, InputDevice,
    MediaFoundationVideoInput, ScreenInput, VideoInput, VirtualCameraController,
    WindowsGraphicsScreenInput, choose_video_device, frame_pipe_name,
};
#[cfg(any(windows, test))]
use std::collections::HashSet;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver};
#[cfg(any(windows, test))]
use std::sync::{Condvar, Mutex};

const COMMAND_CAPACITY: usize = 32;
const MAX_COMMANDS_PER_OUTPUT_CYCLE: usize = 8;
const ACTIVITY_LIMIT: usize = 20;
const REFERENCE_JOB_CAPACITY: usize = 4;
const REFERENCE_MAX_DIMENSION: u32 = 8192;
const REFERENCE_MAX_ALLOCATION: u64 = 256 * 1024 * 1024;
const REFERENCE_PREVIEW_SIZE: Size = Size::new(1280, 720);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

fn finish_worker_shutdown(
    done: &StdReceiver<()>,
    worker: &mut Option<JoinHandle<()>>,
    timeout: Duration,
) -> bool {
    let completed = done.recv_timeout(timeout).is_ok();
    if completed {
        if let Some(worker) = worker.take() {
            let _ = worker.join();
        }
    } else {
        // Dropping JoinHandle deliberately detaches a worker stuck in an OS
        // driver, filesystem, or decoder call. Callers use this only on exit.
        worker.take();
    }
    completed
}
const FPS_TRACKING_WINDOW: Duration = Duration::from_secs(1);
#[cfg(any(windows, test))]
use stageswap_core::ComponentLifecycle;
#[cfg(any(windows, test))]
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
    frame_pool: FrameBufferPool,
}

impl DiscoEffect {
    fn new(size: Size) -> Self {
        Self {
            x_band: vec![0; size.width as usize],
            x_boost: vec![0; size.width as usize],
            y_boost: vec![0; size.height as usize],
            frame_pool: FrameBufferPool::new(
                size.width as usize * size.height as usize * 4,
                CAPTURE_FRAME_POOL_CAPACITY,
            ),
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
        let pixels = self
            .frame_pool
            .write_with_fallback_sized(source.pixels().len(), |pixels| {
                pixels.copy_from_slice(source.pixels());
                for y in 0..height {
                    let row_shift =
                        (y * palette_len / height.max(1) + frame_phase / 12) % palette_len;
                    let row_offset = y * source.stride as usize;
                    for x in 0..width {
                        let offset = row_offset + x * 4;
                        let palette =
                            DISCO_PALETTE_BGRA[(self.x_band[x] as usize + row_shift) % palette_len];
                        let strength = (base_strength
                            + u16::from(self.x_boost[x])
                            + u16::from(self.y_boost[y]))
                        .min(188);
                        let inverse = 256 - strength;
                        for channel in 0..3 {
                            let tinted = ((u16::from(pixels[offset + channel]) * inverse
                                + u16::from(palette[channel]) * strength
                                + 128)
                                >> 8) as u8;
                            pixels[offset + channel] = (u16::from(tinted)
                                + ((255 - u16::from(tinted)) * flash_lift + 128) / 256)
                                .min(255)
                                as u8;
                        }
                    }
                }
                paint_disco_sparkles(
                    pixels,
                    source.stride as usize,
                    width,
                    height,
                    frame_phase / 3,
                );
                Ok::<(), std::convert::Infallible>(())
            })
            .expect("disco rendering is infallible");
        Frame::new(
            pixels,
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
struct WebcamCropCache {
    source: Option<Arc<Frame>>,
    cropped: Option<Arc<Frame>>,
    format: Option<(Size, u32, u64)>,
    source_x: Vec<usize>,
    source_rows: Vec<usize>,
    frame_pool: FrameBufferPool,
}

#[cfg(any(windows, test))]
impl Default for WebcamCropCache {
    fn default() -> Self {
        Self {
            source: None,
            cropped: None,
            format: None,
            source_x: Vec::new(),
            source_rows: Vec::new(),
            frame_pool: FrameBufferPool::new(
                PIPELINE_SIZE.width as usize * PIPELINE_SIZE.height as usize * 4,
                CAPTURE_FRAME_POOL_CAPACITY,
            ),
        }
    }
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
        let pixels = self
            .frame_pool
            .write_with_fallback_sized(source.pixels().len(), |pixels| {
                for (y, source_row) in self.source_rows.iter().copied().enumerate() {
                    let destination_row = y * source.stride as usize;
                    for (x, source_x) in self.source_x.iter().copied().enumerate() {
                        let source_offset = source_row + source_x;
                        let destination = destination_row + x * 4;
                        pixels[destination..destination + 4]
                            .copy_from_slice(&source.pixels()[source_offset..source_offset + 4]);
                    }
                }
                Ok::<(), std::convert::Infallible>(())
            })
            .expect("webcam crop is infallible");
        let cropped = Arc::new(
            Frame::new(
                pixels,
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

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy)]
#[repr(usize)]
enum WarningSource {
    DeviceWorker = 0,
    PublisherSink = 1,
    PublisherController = 2,
    VirtualCamera = 3,
    WebcamCapture = 4,
    ScreenCapture = 5,
    Hdr = 6,
    Reference = 7,
    Command = 8,
}

const WARNING_SOURCE_COUNT: usize = 9;

#[derive(Clone)]
struct WarningRegistry {
    entries: [Option<String>; WARNING_SOURCE_COUNT],
}

impl Default for WarningRegistry {
    fn default() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
        }
    }
}

impl WarningRegistry {
    fn set(&mut self, source: WarningSource, message: impl Into<String>) -> bool {
        let message = message.into();
        let entry = &mut self.entries[source as usize];
        if entry.as_ref() == Some(&message) {
            return false;
        }
        *entry = Some(message);
        true
    }

    fn clear(&mut self, source: WarningSource) -> bool {
        self.entries[source as usize].take().is_some()
    }

    fn top(&self) -> Option<String> {
        self.entries.iter().find_map(Clone::clone)
    }

    #[cfg(windows)]
    fn copy_device_sources_from(&mut self, source: &Self) -> bool {
        let mut changed = false;
        for warning_source in [
            WarningSource::PublisherController,
            WarningSource::VirtualCamera,
            WarningSource::WebcamCapture,
            WarningSource::ScreenCapture,
            WarningSource::Hdr,
        ] {
            let source_entry = &source.entries[warning_source as usize];
            let destination = &mut self.entries[warning_source as usize];
            if destination != source_entry {
                destination.clone_from(source_entry);
                changed = true;
            }
        }
        changed
    }
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
    warnings: WarningRegistry,
    activity: VecDeque<String>,
    next_activity_id: u64,
    sequence: u64,
    started_at: Instant,
    reference: Option<GrayImage>,
    detector: DebouncedDetector,
    last_detection: Instant,
    #[cfg(windows)]
    pending_reference_capture: Option<Arc<Frame>>,
    #[cfg(any(windows, test))]
    webcam_crop: WebcamCropCache,
    #[cfg(windows)]
    publisher_diagnostics: PublisherDiagnosticsState,
}

#[cfg(windows)]
#[derive(Default)]
struct PublisherDiagnosticsState {
    last_published_sequence: u64,
    last_transmitted_sequence: u64,
    last_connected: Option<bool>,
    last_write_failures: u64,
    last_connection_failures: u64,
    last_reported_at: Option<Instant>,
}

impl RuntimeState {
    #[cfg(test)]
    fn new(config: AppConfig) -> Self {
        Self::new_at(config, Instant::now())
    }

    fn new_at(config: AppConfig, now: Instant) -> Self {
        let mode = config.output_mode;
        let reference = None;
        let reference_preview = None;
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
            warnings: WarningRegistry::default(),
            activity: VecDeque::with_capacity(ACTIVITY_LIMIT),
            next_activity_id: 1,
            sequence,
            started_at: now,
            reference,
            detector,
            last_detection: now - Duration::from_millis(250),
            #[cfg(windows)]
            pending_reference_capture: None,
            #[cfg(any(windows, test))]
            webcam_crop: WebcamCropCache::default(),
            #[cfg(windows)]
            publisher_diagnostics: PublisherDiagnosticsState::default(),
        }
    }

    fn record(&mut self, message: impl Into<String>) {
        if self.activity.len() == ACTIVITY_LIMIT {
            self.activity.pop_front();
        }
        self.activity.push_back(message.into());
        self.next_activity_id = self.next_activity_id.saturating_add(1);
        self.snapshot.recent_activity_first_id = self
            .next_activity_id
            .saturating_sub(self.activity.len() as u64);
        self.snapshot.recent_activity = self.activity.iter().cloned().collect::<Vec<_>>().into();
    }

    fn set_warning(&mut self, source: WarningSource, message: impl Into<String>) {
        if self.warnings.set(source, message) {
            self.snapshot.warning = self.warnings.top();
        }
    }

    fn clear_warning(&mut self, source: WarningSource) {
        if self.warnings.clear(source) {
            self.snapshot.warning = self.warnings.top();
        }
    }

    #[cfg(windows)]
    fn record_publisher_diagnostics(
        &mut self,
        diagnostics: FramePublisherDiagnostics,
        now: Instant,
    ) {
        let previous = &self.publisher_diagnostics;
        let state_changed = previous.last_connected != Some(diagnostics.connected)
            || diagnostics.write_failures != previous.last_write_failures
            || diagnostics.connection_failures != previous.last_connection_failures;
        let progress_changed = diagnostics.published_sequence != previous.last_published_sequence
            || diagnostics.transmitted_sequence != previous.last_transmitted_sequence;
        let heartbeat_due = previous
            .last_reported_at
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(5));
        let should_report = state_changed || (progress_changed && heartbeat_due);
        self.publisher_diagnostics = PublisherDiagnosticsState {
            last_published_sequence: diagnostics.published_sequence,
            last_transmitted_sequence: diagnostics.transmitted_sequence,
            last_connected: Some(diagnostics.connected),
            last_write_failures: diagnostics.write_failures,
            last_connection_failures: diagnostics.connection_failures,
            last_reported_at: should_report.then_some(now),
        };
        if should_report {
            self.record(format!(
                "Virtual camera pipe status: connected={}, published_sequence={}, transmitted_sequence={}, write_failures={}, connection_count={}, connection_failures={}",
                diagnostics.connected,
                diagnostics.published_sequence,
                diagnostics.transmitted_sequence,
                diagnostics.write_failures,
                diagnostics.connection_count,
                diagnostics.connection_failures,
            ));
        }
    }

    #[cfg(windows)]
    fn merge_device_warnings(&mut self, warnings: &WarningRegistry) {
        if self.warnings.copy_device_sources_from(warnings) {
            self.snapshot.warning = self.warnings.top();
        }
    }

    #[cfg(any(windows, test))]
    fn stage_reference_candidate(&mut self, frame: Arc<Frame>) {
        self.snapshot.previews.reference_candidate = Some(frame);
    }

    fn take_reference_candidate(&mut self) -> Option<Arc<Frame>> {
        self.snapshot.previews.reference_candidate.take()
    }

    #[cfg(test)]
    fn confirm_reference_candidate(&mut self) -> Result<(), String> {
        let frame = self
            .snapshot
            .previews
            .reference_candidate
            .as_ref()
            .cloned()
            .ok_or_else(|| "no reference candidate".to_owned())?;
        let data = reference_data_from_frame(&frame)?;
        save_reference(&frame, &self.config.reference_image_path)?;
        self.take_reference_candidate();
        self.install_reference(data.detector, &data.preview, Instant::now());
        Ok(())
    }

    fn discard_reference_candidate(&mut self) {
        self.snapshot.previews.reference_candidate = None;
    }

    fn command(&mut self, command: Command) -> bool {
        self.apply_command(command, true)
    }

    fn apply_command(&mut self, command: Command, record_activity: bool) -> bool {
        match command {
            Command::Start => {
                self.snapshot.run_state = RunState::Running;
                self.record_command(record_activity, "Automation started");
            }
            Command::Stop => {
                self.snapshot.run_state = RunState::Stopped;
                self.show_off_output(Instant::now());
                self.record_command(record_activity, "Automation stopped");
            }
            Command::ToggleDisco => {
                if self.disco.is_some() {
                    self.disco = None;
                    self.snapshot.disco_enabled = false;
                    self.record_command(record_activity, "Disco mode disabled");
                } else {
                    self.disco = Some(DiscoEffect::new(PIPELINE_SIZE));
                    self.snapshot.disco_enabled = true;
                    self.record_command(record_activity, "Disco mode enabled");
                }
            }
            Command::SetMode(mode) => {
                self.snapshot.mode = mode;
                self.config.output_mode = mode;
                self.record_command(record_activity, format!("Output mode changed to {mode:?}"));
            }
            Command::UpdateSettings(config) | Command::ReloadSettings(config) => {
                let threshold_changed =
                    self.config.similarity_threshold != config.similarity_threshold;
                let reference_path_changed =
                    self.config.reference_image_path != config.reference_image_path;
                self.config = *config;
                self.snapshot.mode = self.config.output_mode;
                self.snapshot.selected_video_device_id =
                    self.config.selected_video_device_id.clone();
                if reference_path_changed {
                    self.discard_reference_candidate();
                }
                if threshold_changed {
                    self.detector = DebouncedDetector::new(DetectorSettings {
                        threshold: self.config.similarity_threshold,
                        ..DetectorSettings::default()
                    });
                    self.snapshot.detection = stageswap_core::DetectionState::Unknown;
                }
                self.record_command(record_activity, "Settings updated");
            }
            Command::CaptureReference => {
                self.record_command(record_activity, "Reference capture requested");
            }
            Command::CaptureReferenceCandidate => {
                self.record_command(record_activity, "Reference candidate capture requested")
            }
            Command::ConfirmReferenceCandidate => self.record_command(
                record_activity,
                "Reference candidate confirmation requested",
            ),
            Command::DiscardReferenceCandidate => {
                self.discard_reference_candidate();
                self.record_command(record_activity, "Reference candidate discarded")
            }
            Command::ImportReference(path) => {
                let _ = path;
                self.record_command(record_activity, "Reference import requested");
            }
            Command::SelectMonitor(_) => {
                self.record_command(record_activity, "Tracked monitor selection requested")
            }
            Command::RefreshVideoDevices => {
                self.record_command(record_activity, "Video device list refreshed")
            }
            Command::Rescan => self.record_command(record_activity, "Monitor rescan requested"),
            Command::Restart(target) => {
                self.record_command(record_activity, format!("Restart requested: {target:?}"))
            }
            Command::Exit => return false,
        }
        true
    }

    fn record_command(&mut self, enabled: bool, message: impl Into<String>) {
        if enabled {
            let message = message.into();
            let is_coalesced_device_request = matches!(
                message.as_str(),
                "Video device list refreshed" | "Monitor rescan requested"
            ) && self
                .activity
                .back()
                .is_some_and(|previous| previous == &message);
            if !is_coalesced_device_request {
                self.record(message);
            }
        }
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

struct ReferenceData {
    detector: GrayImage,
    preview: Arc<Frame>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReferenceAction {
    Load,
    LegacyCapture { frame_sequence: u64 },
    ConfirmedCapture { frame_sequence: u64 },
    Import,
}

enum ReferenceJobKind {
    Load {
        path: std::path::PathBuf,
    },
    Persist {
        frame: Arc<Frame>,
        path: std::path::PathBuf,
        confirmed: bool,
    },
    Import {
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
    },
}

struct ReferenceJob {
    generation: u64,
    kind: ReferenceJobKind,
}

struct ReferenceResult {
    generation: u64,
    action: ReferenceAction,
    result: Result<ReferenceData, String>,
}

struct ReferenceWorker {
    jobs: Option<StdSyncSender<ReferenceJob>>,
    results: StdReceiver<ReferenceResult>,
    stop: Arc<StdAtomicBool>,
    done: StdReceiver<()>,
    worker: Option<JoinHandle<()>>,
}

impl ReferenceWorker {
    fn start() -> Result<Self, String> {
        let (job_sender, job_receiver) =
            std_mpsc::sync_channel::<ReferenceJob>(REFERENCE_JOB_CAPACITY);
        let (result_sender, result_receiver) =
            std_mpsc::sync_channel::<ReferenceResult>(REFERENCE_JOB_CAPACITY);
        let (done_sender, done_receiver) = std_mpsc::sync_channel(1);
        let stop = Arc::new(StdAtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("stageswap-reference-io".into())
            .spawn(move || {
                while let Ok(job) = job_receiver.recv() {
                    if worker_stop.load(StdOrdering::Acquire) {
                        break;
                    }
                    let result = execute_reference_job(job);
                    if result_sender.try_send(result).is_err() {
                        break;
                    }
                }
                let _ = done_sender.try_send(());
            })
            .map_err(|error| format!("could not start reference worker: {error}"))?;
        Ok(Self {
            jobs: Some(job_sender),
            results: result_receiver,
            stop,
            done: done_receiver,
            worker: Some(worker),
        })
    }

    fn submit(&self, job: ReferenceJob) -> Result<(), String> {
        self.jobs
            .as_ref()
            .ok_or_else(|| "reference worker is stopped".to_owned())?
            .try_send(job)
            .map_err(|error| match error {
                std_mpsc::TrySendError::Full(_) => "reference worker is busy".to_owned(),
                std_mpsc::TrySendError::Disconnected(_) => {
                    "reference worker is unavailable".to_owned()
                }
            })
    }

    fn poll(&self) -> Option<ReferenceResult> {
        self.results.try_recv().ok()
    }

    fn signal_shutdown(&mut self) {
        self.stop.store(true, StdOrdering::Release);
        self.jobs.take();
    }

    fn finish_shutdown(&mut self, timeout: Duration) -> bool {
        finish_worker_shutdown(&self.done, &mut self.worker, timeout)
    }
}

impl Drop for ReferenceWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.signal_shutdown();
            let _ = self.finish_shutdown(WORKER_SHUTDOWN_TIMEOUT);
        }
    }
}

fn execute_reference_job(job: ReferenceJob) -> ReferenceResult {
    let (action, result) = match job.kind {
        ReferenceJobKind::Load { path } => {
            cleanup_pending_reference(&path);
            (ReferenceAction::Load, decode_reference(&path))
        }
        ReferenceJobKind::Persist {
            frame,
            path,
            confirmed,
        } => {
            let action = if confirmed {
                ReferenceAction::ConfirmedCapture {
                    frame_sequence: frame.sequence,
                }
            } else {
                ReferenceAction::LegacyCapture {
                    frame_sequence: frame.sequence,
                }
            };
            let result = reference_data_from_frame(&frame)
                .and_then(|data| persist_frame_atomic(&frame, &path).map(|()| data));
            (action, result)
        }
        ReferenceJobKind::Import {
            source,
            destination,
        } => {
            let result = decode_rgba_limited(&source).and_then(|rgba| {
                let data = reference_data_from_rgba(&rgba)?;
                persist_rgba_atomic(&rgba, &destination)?;
                Ok(data)
            });
            (ReferenceAction::Import, result)
        }
    };
    ReferenceResult {
        generation: job.generation,
        action,
        result,
    }
}

fn decode_rgba_limited(path: &std::path::Path) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|error| format!("could not open reference image: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("could not identify reference image: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(REFERENCE_MAX_DIMENSION);
    limits.max_image_height = Some(REFERENCE_MAX_DIMENSION);
    limits.max_alloc = Some(REFERENCE_MAX_ALLOCATION);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("could not decode reference image: {error}"))
        .map(|image| image.to_rgba8())
}

fn decode_reference(path: &std::path::Path) -> Result<ReferenceData, String> {
    if path.as_os_str().is_empty() {
        return Err("reference image path is empty".into());
    }
    let rgba = decode_rgba_limited(path)?;
    reference_data_from_rgba(&rgba)
}

fn reference_data_from_frame(frame: &Frame) -> Result<ReferenceData, String> {
    let detector = gray_thumbnail(frame)
        .ok_or_else(|| "reference candidate is not a valid screen frame".to_owned())?;
    let preview = if frame.size.width > REFERENCE_PREVIEW_SIZE.width
        || frame.size.height > REFERENCE_PREVIEW_SIZE.height
    {
        let mut rgba = frame.pixels().to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let rgba = image::RgbaImage::from_raw(frame.size.width, frame.size.height, rgba)
            .ok_or_else(|| "reference frame layout is invalid".to_owned())?;
        reference_data_from_rgba(&rgba)?.preview
    } else {
        Arc::new(frame.clone())
    };
    Ok(ReferenceData { detector, preview })
}

fn reference_data_from_rgba(rgba: &image::RgbaImage) -> Result<ReferenceData, String> {
    let size = Size::new(rgba.width(), rgba.height());
    let mut bgra = rgba.as_raw().clone();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let detector = bgra_to_gray(&bgra, size, size.width as usize * 4)
        .and_then(|image| resize_bilinear(&image, Size::new(160, 90)))
        .map_err(|error| format!("could not prepare reference detector image: {error:?}"))?;
    let preview_rgba = if size.width > REFERENCE_PREVIEW_SIZE.width
        || size.height > REFERENCE_PREVIEW_SIZE.height
    {
        image::DynamicImage::ImageRgba8(rgba.clone())
            .thumbnail(REFERENCE_PREVIEW_SIZE.width, REFERENCE_PREVIEW_SIZE.height)
            .to_rgba8()
    } else {
        rgba.clone()
    };
    let preview_size = Size::new(preview_rgba.width(), preview_rgba.height());
    let mut preview_bgra = preview_rgba.into_raw();
    for pixel in preview_bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let preview = Frame::new(
        preview_bgra.into(),
        preview_size,
        preview_size.width * 4,
        0,
        0,
        Instant::now(),
    )
    .map(Arc::new)
    .map_err(|error| format!("could not prepare reference preview: {error:?}"))?;
    Ok(ReferenceData { detector, preview })
}

fn pending_reference_path(destination: &std::path::Path) -> std::path::PathBuf {
    destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("reference.pending.png")
}

fn cleanup_pending_reference(destination: &std::path::Path) {
    let pending = pending_reference_path(destination);
    if let Err(error) = std::fs::remove_file(&pending)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        // A later atomic write will report an actionable error if this matters.
    }
}

fn persist_frame_atomic(frame: &Frame, destination: &std::path::Path) -> Result<(), String> {
    let mut rgba = frame.pixels().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let rgba = image::RgbaImage::from_raw(frame.size.width, frame.size.height, rgba)
        .ok_or_else(|| "reference frame layout is invalid".to_owned())?;
    persist_rgba_atomic(&rgba, destination)
}

fn persist_rgba_atomic(
    rgba: &image::RgbaImage,
    destination: &std::path::Path,
) -> Result<(), String> {
    if destination.as_os_str().is_empty() {
        return Err("reference image path is empty".into());
    }
    if let Some(parent) = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create reference directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let pending = pending_reference_path(destination);
    let mut last_error = None;
    for attempt in 0..3 {
        if attempt > 0 {
            let delay = Duration::from_millis(50 * (1_u64 << (attempt - 1)));
            thread::sleep(delay);
        }
        cleanup_pending_reference(destination);
        match persist_rgba_once(rgba, destination, &pending) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(format!("attempt {}: {error}", attempt + 1));
            }
        }
    }
    cleanup_pending_reference(destination);
    Err(format!(
        "could not persist reference image at {} (pending {}): {}",
        destination.display(),
        pending.display(),
        last_error.unwrap_or_else(|| "unknown persistence failure".into())
    ))
}

fn persist_rgba_once(
    rgba: &image::RgbaImage,
    destination: &std::path::Path,
    pending: &std::path::Path,
) -> Result<(), String> {
    image::save_buffer(
        pending,
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
        image::ColorType::Rgba8,
    )
    .map_err(|error| {
        format!(
            "could not encode pending reference image {}: {error}",
            pending.display()
        )
    })?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(pending)
        .map_err(|error| {
            format!(
                "could not open pending reference image {} for flush: {error}",
                pending.display()
            )
        })?;
    file.sync_all().map_err(|error| {
        format!(
            "could not flush pending reference image {}: {error}",
            pending.display()
        )
    })?;
    validate_pending_reference(pending, rgba.width(), rgba.height())?;
    replace_reference_atomic(pending, destination)
}

fn validate_pending_reference(
    pending: &std::path::Path,
    expected_width: u32,
    expected_height: u32,
) -> Result<(), String> {
    use image::ImageDecoder;
    let decoder = image::codecs::png::PngDecoder::new(std::io::BufReader::new(
        std::fs::File::open(pending).map_err(|error| {
            format!(
                "could not reopen pending reference image {}: {error}",
                pending.display()
            )
        })?,
    ))
    .map_err(|error| {
        format!(
            "pending reference {} is not a valid PNG: {error}",
            pending.display()
        )
    })?;
    let dimensions = decoder.dimensions();
    if dimensions != (expected_width, expected_height) {
        return Err(format!(
            "pending reference {} dimensions changed from {expected_width}x{expected_height} to {}x{}",
            pending.display(),
            dimensions.0,
            dimensions.1
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn replace_reference_atomic(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    stageswap_windows::replace_file_atomic(source, destination).map_err(|error| {
        format!(
            "could not atomically replace reference image {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(not(windows))]
fn replace_reference_atomic(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    std::fs::rename(source, destination).map_err(|error| {
        format!(
            "could not atomically replace reference image {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(test)]
fn load_reference(path: &str) -> Option<(GrayImage, Arc<Frame>)> {
    let data = decode_reference(std::path::Path::new(path)).ok()?;
    Some((data.detector, data.preview))
}

#[cfg(test)]
fn save_reference(frame: &Frame, path: &str) -> Result<(), String> {
    persist_frame_atomic(frame, std::path::Path::new(path))
}

trait RuntimePorts {
    fn command(&mut self, command: &Command, state: &mut RuntimeState);
    fn refresh_inputs(&mut self, state: &mut RuntimeState, now: Instant);
    fn publish(&self, state: &mut RuntimeState, now: Instant);
    fn reference_updated(&mut self, _state: &mut RuntimeState) {}
    fn signal_shutdown(&mut self) {}
    fn finish_shutdown(&mut self, _timeout: Duration) -> bool {
        true
    }
}

struct RuntimeEngine<P> {
    state: RuntimeState,
    platform: P,
    pacer: FramePacer,
    reference_worker: Option<ReferenceWorker>,
    reference_generation: u64,
    in_flight_candidate: Option<(u64, u64)>,
    last_deadline_log: Instant,
}

impl<P: RuntimePorts> RuntimeEngine<P> {
    fn from_parts(state: RuntimeState, platform: P, now: Instant) -> Self {
        let frame_interval = Duration::from_nanos(1_000_000_000 / u64::from(PIPELINE_FPS));
        let mut state = state;
        let reference_worker = match ReferenceWorker::start() {
            Ok(worker) => Some(worker),
            Err(error) => {
                state.record(error);
                None
            }
        };
        let mut engine = Self {
            state,
            platform,
            pacer: FramePacer::new(now, frame_interval),
            reference_worker,
            reference_generation: 0,
            in_flight_candidate: None,
            last_deadline_log: now - Duration::from_secs(30),
        };
        let path = std::path::PathBuf::from(engine.state.config.reference_image_path.clone());
        engine.submit_reference_job(ReferenceJobKind::Load { path });
        engine
    }

    fn wait_duration(&self, now: Instant) -> Duration {
        self.pacer.wait_duration(now)
    }

    fn command(&mut self, command: Command) -> bool {
        let previous_reference_path = self.state.config.reference_image_path.clone();
        self.platform.command(&command, &mut self.state);
        if !self.state.command(command.clone()) {
            return false;
        }
        match command {
            Command::UpdateSettings(config) => {
                if config.reference_image_path != previous_reference_path {
                    self.submit_reference_job(ReferenceJobKind::Load {
                        path: std::path::PathBuf::from(config.reference_image_path.clone()),
                    });
                }
            }
            Command::ReloadSettings(config) => {
                self.submit_reference_job(ReferenceJobKind::Load {
                    path: std::path::PathBuf::from(config.reference_image_path.clone()),
                });
            }
            Command::CaptureReference => {
                #[cfg(windows)]
                if let Some(frame) = self.state.pending_reference_capture.take() {
                    self.submit_reference_job(ReferenceJobKind::Persist {
                        frame,
                        path: std::path::PathBuf::from(
                            self.state.config.reference_image_path.clone(),
                        ),
                        confirmed: false,
                    });
                }
            }
            Command::ConfirmReferenceCandidate => {
                if let Some(frame) = self.state.snapshot.previews.reference_candidate.clone() {
                    if self
                        .in_flight_candidate
                        .is_some_and(|(sequence, _)| sequence == frame.sequence)
                    {
                        self.state
                            .record("Reference candidate confirmation is already pending");
                    } else {
                        self.submit_reference_job(ReferenceJobKind::Persist {
                            frame,
                            path: std::path::PathBuf::from(
                                self.state.config.reference_image_path.clone(),
                            ),
                            confirmed: true,
                        });
                    }
                } else {
                    self.state
                        .record("Reference candidate confirmation failed: no reference candidate");
                }
            }
            Command::ImportReference(source) => {
                self.submit_reference_job(ReferenceJobKind::Import {
                    source,
                    destination: std::path::PathBuf::from(
                        self.state.config.reference_image_path.clone(),
                    ),
                });
            }
            _ => {}
        }
        true
    }

    fn submit_reference_job(&mut self, kind: ReferenceJobKind) -> bool {
        let generation = self.reference_generation.wrapping_add(1).max(1);
        let candidate_sequence = match &kind {
            ReferenceJobKind::Persist {
                frame,
                confirmed: true,
                ..
            } => Some(frame.sequence),
            _ => None,
        };
        let Some(worker) = self.reference_worker.as_ref() else {
            self.state.record("Reference worker is unavailable");
            return false;
        };
        match worker.submit(ReferenceJob { generation, kind }) {
            Ok(()) => {
                self.reference_generation = generation;
                self.state.clear_warning(WarningSource::Command);
                if let Some(sequence) = candidate_sequence {
                    self.in_flight_candidate = Some((sequence, generation));
                }
                true
            }
            Err(error) => {
                self.state
                    .set_warning(WarningSource::Command, error.clone());
                self.state
                    .record(format!("Reference command failed: {error}"));
                false
            }
        }
    }

    fn poll_reference_results(&mut self, now: Instant) {
        while let Some(result) = self
            .reference_worker
            .as_ref()
            .and_then(ReferenceWorker::poll)
        {
            if self
                .in_flight_candidate
                .is_some_and(|(_, generation)| generation == result.generation)
            {
                self.in_flight_candidate = None;
            }
            if result.generation != self.reference_generation {
                self.state.record(format!(
                    "Ignored obsolete reference result {}",
                    result.generation
                ));
                continue;
            }
            match result.result {
                Ok(data) => {
                    self.state.clear_warning(WarningSource::Reference);
                    if let ReferenceAction::ConfirmedCapture { frame_sequence } = result.action {
                        let candidate_matches = self
                            .state
                            .snapshot
                            .previews
                            .reference_candidate
                            .as_ref()
                            .is_some_and(|candidate| candidate.sequence == frame_sequence);
                        if !candidate_matches {
                            self.state
                                .record("Ignored reference result for a replaced review candidate");
                            continue;
                        }
                        self.state.take_reference_candidate();
                    } else if result.action == ReferenceAction::Import {
                        self.state.discard_reference_candidate();
                    }
                    self.state
                        .install_reference(data.detector, &data.preview, now);
                    self.platform.reference_updated(&mut self.state);
                    let message = match result.action {
                        ReferenceAction::Load => "Reference loaded",
                        ReferenceAction::LegacyCapture { .. } => "Reference captured",
                        ReferenceAction::ConfirmedCapture { .. } => "Reference candidate confirmed",
                        ReferenceAction::Import => "Reference imported into local storage",
                    };
                    self.state.record(message);
                }
                Err(error) => {
                    if result.action == ReferenceAction::Load {
                        self.state.reference = None;
                        self.state.snapshot.previews.reference = None;
                        self.state.detector.reset();
                        self.state.snapshot.detection = stageswap_core::DetectionState::Unknown;
                    }
                    self.state
                        .set_warning(WarningSource::Reference, error.clone());
                    self.state
                        .record(format!("Reference operation failed: {error}"));
                }
            }
        }
    }

    fn shutdown_workers(&mut self) {
        if let Some(worker) = self.reference_worker.as_mut() {
            worker.signal_shutdown();
        }
        self.platform.signal_shutdown();
        let deadline = Instant::now() + WORKER_SHUTDOWN_TIMEOUT;
        if let Some(worker) = self.reference_worker.as_mut() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let _ = worker.finish_shutdown(remaining);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = self.platform.finish_shutdown(remaining);
    }

    fn step(&mut self, now: Instant) -> bool {
        if !self.pacer.is_due(now, Duration::ZERO) {
            return false;
        }
        let skipped = self.pacer.advance(now);
        self.state.snapshot.output_deadline_misses = self
            .state
            .snapshot
            .output_deadline_misses
            .saturating_add(skipped);
        if skipped > 0
            && now.saturating_duration_since(self.last_deadline_log) >= Duration::from_secs(30)
        {
            self.state.record(format!(
                "Output pacer skipped {skipped} overdue deadline(s)"
            ));
            self.last_deadline_log = now;
        }
        self.poll_reference_results(now);
        self.platform.refresh_inputs(&mut self.state, now);
        #[cfg(windows)]
        if let Some(frame) = self.state.pending_reference_capture.take() {
            self.submit_reference_job(ReferenceJobKind::Persist {
                frame,
                path: std::path::PathBuf::from(self.state.config.reference_image_path.clone()),
                confirmed: false,
            });
        }
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
    'runtime: loop {
        if commands.shutdown_requested() {
            break;
        }
        let mut processed = 0;
        if let Some(command) = commands.recv_timeout(engine.wait_duration(clock.now())) {
            if !engine.command(command) {
                break 'runtime;
            }
            processed += 1;
        }
        while processed < MAX_COMMANDS_PER_OUTPUT_CYCLE && !commands.shutdown_requested() {
            let Some(command) = commands.try_recv() else {
                break;
            };
            if !engine.command(command) {
                break 'runtime;
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
    engine.shutdown_workers();
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

#[cfg(any(windows, test))]
struct CoalescingSlot<T> {
    pending: Mutex<Option<T>>,
    changed: Condvar,
}

#[cfg(any(windows, test))]
impl<T> CoalescingSlot<T> {
    fn new() -> Self {
        Self {
            pending: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    fn replace(&self, value: T) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        *pending = Some(value);
        self.changed.notify_one();
        true
    }

    #[cfg(test)]
    fn take(&self) -> Option<T> {
        self.pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
    }

    #[cfg(windows)]
    fn wait_take(&self, stop: &StdAtomicBool) -> Option<T> {
        let mut pending = self
            .pending
            .lock()
            .expect("coalescing worker request state is not poisoned");
        while pending.is_none() && !stop.load(StdOrdering::Acquire) {
            pending = self
                .changed
                .wait(pending)
                .expect("coalescing worker request state is not poisoned");
        }
        if stop.load(StdOrdering::Acquire) {
            None
        } else {
            pending.take()
        }
    }

    fn clear(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = None;
        }
    }

    #[cfg(windows)]
    fn wake(&self) {
        self.changed.notify_all();
    }
}

#[cfg(windows)]
struct MonitorScanWorker {
    pending: Arc<CoalescingSlot<MonitorScanRequest>>,
    result: Arc<Mutex<Option<MonitorScanResult>>>,
    stop: Arc<AtomicBool>,
    done: Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(windows)]
impl MonitorScanWorker {
    fn start() -> Result<Self, String> {
        let pending = Arc::new(CoalescingSlot::new());
        let worker_pending = Arc::clone(&pending);
        let result = Arc::new(Mutex::new(None));
        let worker_result = Arc::clone(&result);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (done_sender, done) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("stageswap-monitor-scan".into())
            .spawn(move || {
                loop {
                    let request = worker_pending.wait_take(&worker_stop);
                    let Some(request) = request else {
                        break;
                    };
                    let scan = scan_monitors(request);
                    if let Ok(mut result) = worker_result.lock() {
                        *result = Some(scan);
                    }
                }
                let _ = done_sender.try_send(());
            })
            .map_err(|error| format!("could not start monitor scan worker: {error}"))?;
        Ok(Self {
            pending,
            result,
            stop,
            done,
            worker: Some(worker),
        })
    }

    fn request(&self, request: MonitorScanRequest) -> bool {
        if self.stop.load(Ordering::Acquire) {
            return false;
        }
        self.pending.replace(request)
    }

    fn poll(&self) -> Option<MonitorScanResult> {
        self.result.lock().ok().and_then(|mut result| result.take())
    }

    fn clear_pending(&self) {
        self.pending.clear();
        if let Ok(mut result) = self.result.lock() {
            *result = None;
        }
    }

    fn signal_shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        self.pending.wake();
    }

    fn finish_shutdown(&mut self) {
        self.signal_shutdown();
        let _ = finish_worker_shutdown(&self.done, &mut self.worker, WORKER_SHUTDOWN_TIMEOUT);
    }
}

#[cfg(windows)]
impl Drop for MonitorScanWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.finish_shutdown();
        }
    }
}

#[cfg(windows)]
type VideoDeviceEnumeration = Result<Vec<InputDevice>, String>;

#[cfg(windows)]
struct VideoDeviceWorker {
    pending: Arc<CoalescingSlot<()>>,
    result: Arc<Mutex<Option<VideoDeviceEnumeration>>>,
    stop: Arc<AtomicBool>,
    done: Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(windows)]
impl VideoDeviceWorker {
    fn start() -> Result<Self, String> {
        let pending = Arc::new(CoalescingSlot::new());
        let worker_pending = Arc::clone(&pending);
        let result = Arc::new(Mutex::new(None));
        let worker_result = Arc::clone(&result);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (done_sender, done) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("stageswap-video-device-enumeration".into())
            .spawn(move || {
                let input = MediaFoundationVideoInput::default();
                loop {
                    if worker_pending.wait_take(&worker_stop).is_none() {
                        break;
                    }
                    let enumeration = input.enumerate();
                    if let Ok(mut result) = worker_result.lock() {
                        *result = Some(enumeration);
                    }
                }
                let _ = done_sender.try_send(());
            })
            .map_err(|error| format!("could not start video device enumeration worker: {error}"))?;
        Ok(Self {
            pending,
            result,
            stop,
            done,
            worker: Some(worker),
        })
    }

    fn request(&self) -> bool {
        if self.stop.load(Ordering::Acquire) {
            return false;
        }
        self.pending.replace(())
    }

    fn poll(&self) -> Option<VideoDeviceEnumeration> {
        self.result.lock().ok().and_then(|mut result| result.take())
    }

    fn signal_shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        self.pending.wake();
    }

    fn finish_shutdown(&mut self) {
        self.signal_shutdown();
        let _ = finish_worker_shutdown(&self.done, &mut self.worker, WORKER_SHUTDOWN_TIMEOUT);
    }
}

#[cfg(windows)]
impl Drop for VideoDeviceWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.finish_shutdown();
        }
    }
}

#[cfg(windows)]
fn scan_monitors(request: MonitorScanRequest) -> MonitorScanResult {
    let input = WindowsGraphicsScreenInput::default();
    let monitors = input.enumerate().unwrap_or_default();
    let scores = request
        .reference
        .as_ref()
        .map_or_else(Vec::new, |reference| {
            monitors
                .iter()
                .cloned()
                .map(|monitor| {
                    let mut capture = WindowsGraphicsScreenInput::default();
                    let valid = capture.start(&monitor, request.cursor_visible).is_ok();
                    let deadline = Instant::now() + Duration::from_millis(750);
                    let frame = valid
                        .then(|| {
                            loop {
                                if let Some(frame) = capture.latest_frame() {
                                    break Some(frame);
                                }
                                if Instant::now() >= deadline {
                                    break None;
                                }
                                thread::sleep(Duration::from_millis(25));
                            }
                        })
                        .flatten();
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
struct DevicePlatform {
    owner_thread: thread::ThreadId,
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
    video_device_worker: Option<VideoDeviceWorker>,
}

#[cfg(windows)]
impl DevicePlatform {
    fn record_webcam_format(state: &mut RuntimeState, webcam: &MediaFoundationVideoInput) {
        let native = webcam.selected_native_format().unwrap_or("unknown");
        let output = webcam.selected_output_format().unwrap_or("unknown");
        state.record(format!(
            "Webcam format negotiated: native={native}; output={output}"
        ));
    }

    fn new(
        state: &mut RuntimeState,
        output_ready: impl FnOnce(&RuntimeState, Option<FramePublisherSink>),
    ) -> Self {
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
                state.set_warning(WarningSource::PublisherController, error.clone());
                state.record(format!("Virtual camera pipe failed: {error}"));
                None
            }
        };
        let publisher =
            pipe_name
                .as_ref()
                .and_then(|pipe_name| match FramePublisher::start(pipe_name) {
                    Ok(publisher) => {
                        state.record("Virtual camera frame pipe created");
                        Some(publisher)
                    }
                    Err(error) => {
                        state
                            .snapshot
                            .publisher_component
                            .mark_failed(now, error.clone());
                        state.set_warning(WarningSource::PublisherController, error.clone());
                        state.record(format!("Frame publisher failed: {error}"));
                        None
                    }
                });
        output_ready(state, publisher.as_ref().map(FramePublisher::sink));
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
                        state.set_warning(WarningSource::VirtualCamera, error.clone());
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
                    Self::record_webcam_format(state, &webcam);
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
                            state.record(format!(
                                "Screen capture initialized: {} generation={}",
                                monitor.label,
                                screen.generation()
                            ));
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
            state.set_warning(
                WarningSource::Hdr,
                "HDR or 10-bit color is enabled on the selected display; disable HDR in Windows Display settings before using automatic matching or reference capture"
                    .to_owned(),
            );
        } else {
            state.clear_warning(WarningSource::Hdr);
        }
        let monitor_scan_worker = match MonitorScanWorker::start() {
            Ok(worker) => Some(worker),
            Err(error) => {
                state.record(error);
                None
            }
        };
        let video_device_worker = match VideoDeviceWorker::start() {
            Ok(worker) => Some(worker),
            Err(error) => {
                state.record(error);
                None
            }
        };
        let mut platform = Self {
            owner_thread: thread::current().id(),
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
            video_device_worker,
        };
        if state.config.automatic_monitor_rescans {
            platform.request_monitor_scan(state, state.config.cursor_visible);
        }
        platform
    }

    fn command(&mut self, command: &Command, state: &mut RuntimeState) {
        self.assert_owner_thread();
        match command {
            Command::UpdateSettings(config) | Command::ReloadSettings(config) => {
                let reload_settings = matches!(command, Command::ReloadSettings(_));
                if config.selected_video_device_id != state.config.selected_video_device_id {
                    let now = Instant::now();
                    self.webcam.stop();
                    state.snapshot.webcam_native_format = None;
                    state.snapshot.webcam_output_format = None;
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
                                Self::record_webcam_format(state, &self.webcam);
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
                state.pending_reference_capture = None;
                if self.selected_display_hdr_unsupported {
                    state.record(
                        "Reference capture is unavailable while HDR or 10-bit color is enabled",
                    );
                    return;
                }
                if let Some(frame) = self.screen.latest_frame() {
                    state.pending_reference_capture = Some(frame);
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
            Command::ConfirmReferenceCandidate => {}
            Command::DiscardReferenceCandidate => {
                state.discard_reference_candidate();
                state.record("Reference candidate discarded");
            }
            Command::ImportReference(_) => {}
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
                    self.synchronize_hdr_warning(state);
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
            Command::RefreshVideoDevices => {
                if self
                    .video_device_worker
                    .as_ref()
                    .is_none_or(|worker| !worker.request())
                {
                    state.record("Video device refresh worker is unavailable");
                }
            }
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

    fn reference_updated(&mut self, state: &mut RuntimeState) {
        self.assert_owner_thread();
        self.invalidate_monitor_scans(state.config.similarity_threshold);
        if state.config.automatic_monitor_rescans {
            self.request_monitor_scan(state, state.config.cursor_visible);
        }
    }

    fn synchronize_hdr_warning(&self, state: &mut RuntimeState) {
        if self.selected_display_hdr_unsupported {
            state.set_warning(
                WarningSource::Hdr,
                "HDR or 10-bit color is enabled on the selected display; disable HDR in Windows Display settings before using automatic matching or reference capture",
            );
        } else {
            state.clear_warning(WarningSource::Hdr);
        }
    }

    fn assert_owner_thread(&self) {
        assert_eq!(
            thread::current().id(),
            self.owner_thread,
            "Windows device objects must only be used by their owner thread"
        );
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
        state.snapshot.webcam_native_format = None;
        state.snapshot.webcam_output_format = None;
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
                Self::record_webcam_format(state, &self.webcam);
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
                state.clear_warning(WarningSource::VirtualCamera);
                state.record("Virtual camera restarted");
            }
            Err(error) => {
                state.snapshot.virtual_camera_state = DeviceState::Failed;
                state
                    .snapshot
                    .virtual_camera_component
                    .mark_failed(now, error.clone());
                state.set_warning(WarningSource::VirtualCamera, error.clone());
                state.record(format!("Virtual camera restart failed: {error}"));
            }
        }
    }

    fn refresh_inputs(&mut self, state: &mut RuntimeState, now: Instant) {
        self.assert_owner_thread();
        self.refresh_video_device_result(state);
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
            state.snapshot.availability.camera_ready = false;
            state.set_warning(WarningSource::WebcamCapture, error.clone());
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
            state.set_warning(
                WarningSource::WebcamCapture,
                "Webcam frames are stale; safe fallback is active",
            );
            state.snapshot.webcam_state = DeviceState::Failed;
            state
                .snapshot
                .webcam_component
                .transition(ComponentLifecycle::Stale, now);
            state.snapshot.previews.webcam = None;
        } else if webcam.is_some() {
            state.clear_warning(WarningSource::WebcamCapture);
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
            state.set_warning(
                WarningSource::WebcamCapture,
                "webcam did not deliver a frame before the startup deadline",
            );
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
            state.snapshot.previews.webcam = Some(webcam);
        }
        let screen = self.screen.latest_frame();
        state.snapshot.previews.screen = None;
        state.snapshot.availability.screen_ready = screen.is_some()
            && !(self.selected_display_hdr_unsupported
                && state.snapshot.mode == stageswap_core::OutputMode::Automatic);
        if self.selected_display_hdr_unsupported
            && state.snapshot.mode == stageswap_core::OutputMode::Automatic
        {
            state.set_warning(
                WarningSource::ScreenCapture,
                "screen capture did not deliver a frame before the startup deadline",
            );
            state.snapshot.screen_state = DeviceState::Failed;
            state.snapshot.screen_component.mark_failed_with_kind(
                now,
                ComponentFailureKind::Screen(ScreenFailureKind::UnsupportedHdr),
                "HDR or 10-bit color must be disabled for automatic matching",
            );
            state.snapshot.previews.screen = None;
        } else if let Some(error) = self.screen.last_error() {
            state.snapshot.availability.screen_ready = false;
            state.set_warning(WarningSource::ScreenCapture, error.clone());
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
        } else if let Some(screen) = screen {
            state.clear_warning(WarningSource::ScreenCapture);
            let received_at = screen.received_at;
            state.snapshot.screen_state = DeviceState::Ready;
            let was_ready = state.snapshot.screen_component.lifecycle == ComponentLifecycle::Ready;
            state.snapshot.screen_component.mark_ready(now);
            state.snapshot.screen_component.last_success_at = Some(received_at);
            state.snapshot.previews.screen = Some(screen);
            if !was_ready {
                state.record(format!(
                    "Screen capture first frame received generation={}",
                    self.screen.generation()
                ));
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
        if self
            .camera
            .as_ref()
            .is_some_and(|camera| !camera.is_running())
        {
            state.snapshot.virtual_camera_state = DeviceState::Failed;
            state.set_warning(
                WarningSource::VirtualCamera,
                "virtual camera stopped unexpectedly",
            );
            state
                .snapshot
                .virtual_camera_component
                .mark_failed(now, "virtual camera stopped unexpectedly");
        }
    }

    fn refresh_video_device_result(&mut self, state: &mut RuntimeState) {
        let Some(result) = self
            .video_device_worker
            .as_ref()
            .and_then(VideoDeviceWorker::poll)
        else {
            return;
        };
        match result {
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
        state.snapshot.previews.screen = None;
        state.snapshot.availability.screen_ready = false;
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
                state.record(format!(
                    "Screen capture restarted generation={}",
                    self.screen.generation()
                ));
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
        match self
            .screen_capture_recovery
            .observe(self.screen.latest_frame().as_deref())
        {
            ScreenCaptureRecoveryObservation::Clear => {}
            ScreenCaptureRecoveryObservation::AwaitingConfirmation => {
                state.record(format!(
                    "Screen capture is black or unavailable; awaiting the next automatic recovery check generation={}",
                    self.screen.generation()
                ));
            }
            ScreenCaptureRecoveryObservation::Restart => {
                state.record(format!(
                    "Screen capture remains black or unavailable; restarting screen capture generation={}",
                    self.screen.generation()
                ));
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
            .as_ref()
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
            self.synchronize_hdr_warning(state);
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
}

#[cfg(windows)]
impl Drop for DevicePlatform {
    fn drop(&mut self) {
        self.assert_owner_thread();
        if let Some(publisher) = &self.publisher {
            let _ = publisher.invalidate();
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Default)]
struct PendingDeviceCommands {
    settings: Option<Command>,
    mode: Option<Command>,
    monitor: Option<MonitorDescriptor>,
    refresh_video_devices: bool,
    rescan: bool,
    stop_output: bool,
    capture_reference: bool,
    capture_candidate: bool,
    discard_candidate: bool,
    restarts: HashSet<RestartTarget>,
    reference: Option<GrayImage>,
}

#[cfg(any(windows, test))]
impl PendingDeviceCommands {
    fn push(&mut self, command: &Command) {
        match command {
            command @ (Command::UpdateSettings(_) | Command::ReloadSettings(_)) => {
                self.settings = Some(command.clone());
            }
            command @ Command::SetMode(_) => self.mode = Some(command.clone()),
            Command::SelectMonitor(monitor) => self.monitor = Some(monitor.clone()),
            Command::RefreshVideoDevices => self.refresh_video_devices = true,
            Command::Rescan => self.rescan = true,
            Command::Stop => self.stop_output = true,
            Command::CaptureReference => self.capture_reference = true,
            Command::CaptureReferenceCandidate => self.capture_candidate = true,
            Command::DiscardReferenceCandidate => self.discard_candidate = true,
            Command::Restart(RestartTarget::All) => {
                self.restarts.clear();
                self.restarts.insert(RestartTarget::All);
            }
            Command::Restart(target) if !self.restarts.contains(&RestartTarget::All) => {
                self.restarts.insert(*target);
            }
            _ => {}
        }
    }

    fn take(&mut self) -> (Vec<Command>, Option<GrayImage>) {
        let mut commands = Vec::with_capacity(9);
        commands.extend(self.settings.take());
        commands.extend(self.mode.take());
        commands.extend(self.monitor.take().map(Command::SelectMonitor));
        if std::mem::take(&mut self.refresh_video_devices) {
            commands.push(Command::RefreshVideoDevices);
        }
        if std::mem::take(&mut self.rescan) {
            commands.push(Command::Rescan);
        }
        if std::mem::take(&mut self.capture_reference) {
            commands.push(Command::CaptureReference);
        }
        if std::mem::take(&mut self.capture_candidate) {
            commands.push(Command::CaptureReferenceCandidate);
        }
        if std::mem::take(&mut self.discard_candidate) {
            commands.push(Command::DiscardReferenceCandidate);
        }
        if std::mem::take(&mut self.stop_output) {
            commands.push(Command::Stop);
        }
        if self.restarts.remove(&RestartTarget::All) {
            self.restarts.clear();
            commands.push(Command::Restart(RestartTarget::All));
        } else {
            commands.extend(self.restarts.drain().map(Command::Restart));
        }
        (commands, self.reference.take())
    }
}

#[cfg(windows)]
#[derive(Default)]
struct DeviceSnapshot {
    app: AppSnapshot,
    warnings: WarningRegistry,
    observed_at: Option<Instant>,
    native_aspect_ratio: Option<f64>,
    publisher: Option<FramePublisherSink>,
    legacy_capture: Option<(u64, Arc<Frame>)>,
    candidate_event: Option<(u64, Option<Arc<Frame>>)>,
}

#[cfg(windows)]
struct DeviceWorker {
    latest: Arc<RwLock<Arc<DeviceSnapshot>>>,
    pending: Arc<(Mutex<PendingDeviceCommands>, Condvar)>,
    stop: Arc<AtomicBool>,
    done: Receiver<()>,
    worker: Option<JoinHandle<()>>,
    last_activity_id: u64,
    last_legacy_capture_id: u64,
    last_candidate_event_id: u64,
}

#[cfg(windows)]
impl DeviceWorker {
    fn start(config: AppConfig) -> Result<Self, String> {
        let latest = Arc::new(RwLock::new(Arc::new(DeviceSnapshot::default())));
        let worker_latest = Arc::clone(&latest);
        let pending = Arc::new((Mutex::new(PendingDeviceCommands::default()), Condvar::new()));
        let worker_pending = Arc::clone(&pending);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("stageswap-device-owner".into())
            .spawn(move || {
                let now = Instant::now();
                let mut state = RuntimeState::new_at(config, now);
                let mut platform = DevicePlatform::new(&mut state, |state, publisher| {
                    let snapshot = DeviceSnapshot {
                        app: state.snapshot.clone(),
                        warnings: state.warnings.clone(),
                        observed_at: Some(Instant::now()),
                        publisher,
                        ..DeviceSnapshot::default()
                    };
                    *worker_latest
                        .write()
                        .expect("device snapshot state is not poisoned") = Arc::new(snapshot);
                });
                let mut pacer = FramePacer::new(
                    now,
                    Duration::from_nanos(1_000_000_000 / u64::from(PIPELINE_FPS)),
                );
                let mut last_slow_command_log = now - Duration::from_secs(60);
                let mut legacy_capture = None;
                let mut legacy_capture_id = 0_u64;
                let mut candidate_event = None;
                let mut candidate_event_id = 0_u64;
                publish_device_snapshot(
                    &worker_latest,
                    &platform,
                    &state,
                    &legacy_capture,
                    &candidate_event,
                );
                while !worker_stop.load(Ordering::Acquire) {
                    let (commands, reference) = {
                        let (lock, _) = &*worker_pending;
                        lock.lock()
                            .expect("device command state is not poisoned")
                            .take()
                    };
                    if let Some(reference) = reference {
                        state.reference = Some(reference);
                        platform.reference_updated(&mut state);
                    }
                    for command in commands {
                        let started = Instant::now();
                        platform.command(&command, &mut state);
                        state.apply_command(command.clone(), false);
                        if matches!(
                            command,
                            Command::CaptureReferenceCandidate | Command::DiscardReferenceCandidate
                        ) {
                            candidate_event_id = candidate_event_id.wrapping_add(1).max(1);
                            candidate_event = Some((
                                candidate_event_id,
                                state.snapshot.previews.reference_candidate.clone(),
                            ));
                        }
                        if matches!(command, Command::CaptureReference)
                            && let Some(frame) = state.pending_reference_capture.take()
                        {
                            legacy_capture_id = legacy_capture_id.wrapping_add(1).max(1);
                            legacy_capture = Some((legacy_capture_id, frame));
                        }
                        let elapsed = started.elapsed();
                        if elapsed >= Duration::from_millis(100)
                            && started.saturating_duration_since(last_slow_command_log)
                                >= Duration::from_secs(30)
                        {
                            state.record(format!("Device command took {} ms", elapsed.as_millis()));
                            last_slow_command_log = started;
                        }
                    }
                    let now = Instant::now();
                    if pacer.is_due(now, Duration::ZERO) {
                        pacer.advance(now);
                        platform.refresh_inputs(&mut state, now);
                        publish_device_snapshot(
                            &worker_latest,
                            &platform,
                            &state,
                            &legacy_capture,
                            &candidate_event,
                        );
                    }
                    let timeout = pacer
                        .wait_duration(Instant::now())
                        .min(Duration::from_millis(34));
                    let (lock, changed) = &*worker_pending;
                    let guard = lock.lock().expect("device command state is not poisoned");
                    if !worker_stop.load(Ordering::Acquire) {
                        let _ = changed.wait_timeout(guard, timeout);
                    }
                }
                let _ = platform.publisher.as_ref().map(FramePublisher::invalidate);
                drop(platform);
                let _ = done_sender.try_send(());
            })
            .map_err(|error| format!("could not start device worker: {error}"))?;
        Ok(Self {
            latest,
            pending,
            stop,
            done: done_receiver,
            worker: Some(worker),
            last_activity_id: 0,
            last_legacy_capture_id: 0,
            last_candidate_event_id: 0,
        })
    }

    fn push(&self, command: &Command) {
        let (lock, changed) = &*self.pending;
        lock.lock()
            .expect("device command state is not poisoned")
            .push(command);
        changed.notify_one();
    }

    fn update_reference(&self, reference: Option<GrayImage>) {
        let Some(reference) = reference else {
            return;
        };
        let (lock, changed) = &*self.pending;
        let mut pending = lock.lock().expect("device command state is not poisoned");
        pending.reference = Some(reference);
        pending.discard_candidate = true;
        drop(pending);
        changed.notify_one();
    }

    fn snapshot(&self) -> Arc<DeviceSnapshot> {
        Arc::clone(
            &self
                .latest
                .read()
                .expect("device snapshot state is not poisoned"),
        )
    }

    fn signal_shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.pending.1.notify_all();
    }

    fn finish_shutdown(&mut self, timeout: Duration) -> bool {
        finish_worker_shutdown(&self.done, &mut self.worker, timeout)
    }
}

#[cfg(windows)]
impl Drop for DeviceWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.signal_shutdown();
            let _ = self.finish_shutdown(WORKER_SHUTDOWN_TIMEOUT);
        }
    }
}

#[cfg(windows)]
fn publish_device_snapshot(
    destination: &RwLock<Arc<DeviceSnapshot>>,
    platform: &DevicePlatform,
    state: &RuntimeState,
    legacy_capture: &Option<(u64, Arc<Frame>)>,
    candidate_event: &Option<(u64, Option<Arc<Frame>>)>,
) {
    let snapshot = DeviceSnapshot {
        app: state.snapshot.clone(),
        warnings: state.warnings.clone(),
        observed_at: Some(Instant::now()),
        native_aspect_ratio: platform.webcam.native_display_aspect_ratio(),
        publisher: platform.publisher.as_ref().map(FramePublisher::sink),
        legacy_capture: legacy_capture.clone(),
        candidate_event: candidate_event.clone(),
    };
    *destination
        .write()
        .expect("device snapshot state is not poisoned") = Arc::new(snapshot);
}

#[cfg(windows)]
struct Platform {
    worker: Option<DeviceWorker>,
}

#[cfg(windows)]
impl Platform {
    fn new(state: &mut RuntimeState) -> Self {
        match DeviceWorker::start(state.config.clone()) {
            Ok(worker) => Self {
                worker: Some(worker),
            },
            Err(error) => {
                state.set_warning(WarningSource::DeviceWorker, error.clone());
                state.snapshot.webcam_state = DeviceState::Failed;
                state.snapshot.screen_state = DeviceState::Failed;
                state.snapshot.virtual_camera_state = DeviceState::Failed;
                state.record(error);
                Self { worker: None }
            }
        }
    }

    fn command(&mut self, command: &Command, _state: &mut RuntimeState) {
        if let Some(worker) = &self.worker {
            worker.push(command);
        }
    }

    fn refresh_inputs(&mut self, state: &mut RuntimeState, now: Instant) {
        let Some(worker) = &mut self.worker else {
            state.snapshot.previews.webcam = None;
            state.snapshot.previews.screen = None;
            state.snapshot.availability = SourceAvailability::default();
            return;
        };
        let device = worker.snapshot();
        let app = &device.app;
        if device.observed_at.is_some_and(|observed_at| {
            now.saturating_duration_since(observed_at) > FRAME_STALE_AFTER
        }) {
            state.set_warning(
                WarningSource::DeviceWorker,
                "Device worker is delayed; safe fallback is active",
            );
        } else {
            state.clear_warning(WarningSource::DeviceWorker);
        }
        state.snapshot.webcam_state = app.webcam_state;
        state.snapshot.screen_state = app.screen_state;
        state.snapshot.virtual_camera_state = app.virtual_camera_state;
        state.snapshot.webcam_component = app.webcam_component.clone();
        state.snapshot.screen_component = app.screen_component.clone();
        state.snapshot.publisher_component = app.publisher_component.clone();
        state.snapshot.virtual_camera_component = app.virtual_camera_component.clone();
        state.snapshot.video_devices = Arc::clone(&app.video_devices);
        state.snapshot.selected_video_device_id = app.selected_video_device_id.clone();
        state.snapshot.webcam_native_format = app.webcam_native_format.clone();
        state.snapshot.webcam_output_format = app.webcam_output_format.clone();
        state.snapshot.monitors = Arc::clone(&app.monitors);
        state.snapshot.selected_monitor = app.selected_monitor.clone();
        state.merge_device_warnings(&device.warnings);

        let webcam_is_stale = app
            .previews
            .webcam
            .as_ref()
            .is_some_and(|frame| !frame.is_fresh_at(now, FRAME_STALE_AFTER));
        let webcam = (!webcam_is_stale)
            .then(|| app.previews.webcam.clone())
            .flatten();
        state.snapshot.previews.webcam = webcam.map(|frame| {
            state.webcam_crop.apply(
                frame,
                state.config.crop_webcam_to_16_9,
                device.native_aspect_ratio,
            )
        });
        if webcam_is_stale {
            state.snapshot.webcam_state = DeviceState::Failed;
            state
                .snapshot
                .webcam_component
                .transition(ComponentLifecycle::Stale, now);
        }
        state.snapshot.previews.screen = app.previews.screen.clone();
        state.snapshot.availability.camera_ready = state.snapshot.previews.webcam.is_some();
        state.snapshot.availability.screen_ready =
            app.availability.screen_ready && state.snapshot.previews.screen.is_some();

        if let Some((capture_id, frame)) = &device.legacy_capture
            && *capture_id > worker.last_legacy_capture_id
        {
            state.pending_reference_capture = Some(Arc::clone(frame));
            worker.last_legacy_capture_id = *capture_id;
        }
        if let Some((event_id, candidate)) = &device.candidate_event
            && *event_id > worker.last_candidate_event_id
        {
            state.snapshot.previews.reference_candidate = candidate.clone();
            worker.last_candidate_event_id = *event_id;
        }

        let first = app.recent_activity_first_id;
        if worker.last_activity_id.saturating_add(1) < first {
            state.record("Device activity entries were skipped");
            worker.last_activity_id = first.saturating_sub(1);
        }
        for (index, activity) in app.recent_activity.iter().enumerate() {
            let id = first.saturating_add(index as u64);
            if id > worker.last_activity_id {
                state.record(activity.clone());
                worker.last_activity_id = id;
            }
        }
    }

    fn publish(&self, state: &mut RuntimeState, now: Instant) {
        let Some(worker) = &self.worker else {
            return;
        };
        let device = worker.snapshot();
        let Some(frame) = state.snapshot.previews.final_output.as_deref() else {
            return;
        };
        if let Some(publisher) = &device.publisher {
            match publisher.publish(frame) {
                Ok(()) => {
                    state.snapshot.publisher_component.mark_ready(now);
                    state.clear_warning(WarningSource::PublisherSink);
                }
                Err(error) => {
                    state
                        .snapshot
                        .publisher_component
                        .mark_failed(now, error.clone());
                    let already_reported =
                        state.warnings.entries[WarningSource::PublisherSink as usize].as_deref()
                            == Some(error.as_str());
                    state.set_warning(WarningSource::PublisherSink, error.clone());
                    if !already_reported {
                        state.record(format!("Frame publish failed: {error}"));
                    }
                }
            }
            state.record_publisher_diagnostics(publisher.diagnostics(), now);
        }
    }

    fn reference_updated(&mut self, state: &mut RuntimeState) {
        if let Some(worker) = &self.worker {
            worker.update_reference(state.reference.clone());
        }
    }

    fn signal_shutdown(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.signal_shutdown();
        }
    }

    fn finish_shutdown(&mut self, timeout: Duration) -> bool {
        self.worker
            .as_mut()
            .is_none_or(|worker| worker.finish_shutdown(timeout))
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
    fn reference_updated(&mut self, _state: &mut RuntimeState) {}
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

    fn reference_updated(&mut self, state: &mut RuntimeState) {
        Platform::reference_updated(self, state);
    }

    fn signal_shutdown(&mut self) {
        #[cfg(windows)]
        Platform::signal_shutdown(self);
    }

    fn finish_shutdown(&mut self, timeout: Duration) -> bool {
        #[cfg(windows)]
        {
            Platform::finish_shutdown(self, timeout)
        }
        #[cfg(not(windows))]
        {
            let _ = timeout;
            true
        }
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
        assert!(engine.state.snapshot.output_deadline_misses > 0);
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
    fn shutdown_timeout_detaches_worker_and_allows_controlled_release() {
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);
        let (done_sender, done_receiver) = std_mpsc::sync_channel(1);
        let mut worker = Some(thread::spawn(move || {
            worker_barrier.wait();
            let _ = done_sender.send(());
        }));

        assert!(!finish_worker_shutdown(
            &done_receiver,
            &mut worker,
            Duration::from_millis(1)
        ));
        assert!(worker.is_none());
        barrier.wait();
        assert!(done_receiver.recv_timeout(Duration::from_secs(1)).is_ok());
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
    fn an_old_but_visible_session_frame_remains_usable() {
        let mut visible = frame_with_bright_pixels(200);
        visible.received_at = Instant::now() - FRAME_STALE_AFTER - Duration::from_secs(1);
        let mut recovery = ScreenCaptureRecovery::default();

        assert_eq!(
            recovery.observe(Some(&visible)),
            ScreenCaptureRecoveryObservation::Clear
        );
        assert_eq!(recovery.consecutive_failures, 0);
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
        assert!(!directory.path().join("reference.pending.png").exists());
    }

    #[test]
    fn reference_decode_enforces_dimension_limit_and_caps_retained_preview() {
        let directory = tempfile::tempdir().unwrap();
        let boundary = directory.path().join("boundary.png");
        image::RgbaImage::new(REFERENCE_MAX_DIMENSION, 1)
            .save(&boundary)
            .unwrap();
        assert!(decode_reference(&boundary).is_ok());

        let oversized = directory.path().join("oversized.png");
        image::RgbaImage::new(REFERENCE_MAX_DIMENSION + 1, 1)
            .save(&oversized)
            .unwrap();
        assert!(decode_reference(&oversized).is_err());

        let large = image::RgbaImage::new(2000, 1000);
        let data = reference_data_from_rgba(&large).unwrap();
        assert_eq!(data.preview.size, Size::new(1280, 640));
        assert_eq!(data.detector.size, Size::new(160, 90));
    }

    #[test]
    fn atomic_reference_replace_preserves_exact_pixels_and_cleans_pending_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("reference.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .save(&destination)
            .unwrap();
        let replacement = image::RgbaImage::from_pixel(3, 1, image::Rgba([9, 8, 7, 255]));

        persist_rgba_atomic(&replacement, &destination).unwrap();

        assert_eq!(image::open(&destination).unwrap().to_rgba8(), replacement);
        assert!(!directory.path().join("reference.pending.png").exists());
    }

    #[test]
    fn warning_registry_preserves_source_ownership_and_priority() {
        let mut state = RuntimeState::new(AppConfig::default());
        state.set_warning(WarningSource::Hdr, "hdr");
        state.set_warning(WarningSource::VirtualCamera, "camera");
        state.set_warning(WarningSource::PublisherSink, "publisher");
        assert_eq!(state.snapshot.warning.as_deref(), Some("publisher"));

        state.clear_warning(WarningSource::VirtualCamera);
        assert_eq!(state.snapshot.warning.as_deref(), Some("publisher"));
        state.clear_warning(WarningSource::PublisherSink);
        assert_eq!(state.snapshot.warning.as_deref(), Some("hdr"));
        state.clear_warning(WarningSource::Hdr);
        assert!(state.snapshot.warning.is_none());
    }

    #[test]
    fn activity_ids_keep_duplicate_messages_and_track_ring_rollover() {
        let mut state = RuntimeState::new(AppConfig::default());
        state.record("duplicate");
        state.record("duplicate");
        assert_eq!(state.snapshot.recent_activity_first_id, 1);
        assert_eq!(state.snapshot.recent_activity.len(), 2);
        for index in 0..ACTIVITY_LIMIT {
            state.record(format!("event-{index}"));
        }
        assert_eq!(state.snapshot.recent_activity.len(), ACTIVITY_LIMIT);
        assert_eq!(state.snapshot.recent_activity_first_id, 3);
        assert_eq!(state.snapshot.recent_activity[0], "event-0");
    }

    #[test]
    fn device_command_application_does_not_replay_user_activity() {
        let mut state = RuntimeState::new(AppConfig::default());
        assert!(state.apply_command(Command::Start, false));
        assert!(state.snapshot.recent_activity.is_empty());

        assert!(state.command(Command::Start));
        assert_eq!(state.snapshot.recent_activity.len(), 1);
        assert_eq!(state.snapshot.recent_activity[0], "Automation started");
    }

    #[test]
    fn duplicate_device_refresh_and_rescan_activity_is_coalesced() {
        let mut state = RuntimeState::new(AppConfig::default());

        assert!(state.command(Command::RefreshVideoDevices));
        assert!(state.command(Command::RefreshVideoDevices));
        assert!(state.command(Command::Rescan));
        assert!(state.command(Command::Rescan));

        assert_eq!(state.snapshot.recent_activity.len(), 2);
        assert_eq!(
            state.snapshot.recent_activity[0],
            "Video device list refreshed"
        );
        assert_eq!(
            state.snapshot.recent_activity[1],
            "Monitor rescan requested"
        );
    }

    #[test]
    fn settings_only_reload_reference_when_required() {
        let now = Instant::now();
        let config = AppConfig::default();
        let state = RuntimeState::new_at(config.clone(), now);
        let mut engine = RuntimeEngine::from_parts(state, FakeRuntimePorts::default(), now);
        assert_eq!(engine.reference_generation, 1);

        assert!(engine.command(Command::UpdateSettings(Box::new(config.clone()))));
        assert_eq!(engine.reference_generation, 1);
        assert!(engine.command(Command::ReloadSettings(Box::new(config))));
        assert_eq!(engine.reference_generation, 2);
    }

    #[test]
    fn device_commands_are_bounded_coalesced_and_restart_all_subsumes_individuals() {
        let mut pending = PendingDeviceCommands::default();
        let first = AppConfig {
            cursor_visible: false,
            ..AppConfig::default()
        };
        let latest = AppConfig {
            cursor_visible: true,
            ..first.clone()
        };
        pending.push(&Command::UpdateSettings(Box::new(first)));
        pending.push(&Command::UpdateSettings(Box::new(latest.clone())));
        pending.push(&Command::RefreshVideoDevices);
        pending.push(&Command::RefreshVideoDevices);
        pending.push(&Command::Restart(RestartTarget::Webcam));
        pending.push(&Command::Restart(RestartTarget::All));
        pending.push(&Command::Restart(RestartTarget::ScreenCapture));

        let (commands, reference) = pending.take();

        assert!(reference.is_none());
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, Command::UpdateSettings(_)))
                .count(),
            1
        );
        assert!(commands.contains(&Command::UpdateSettings(Box::new(latest))));
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, Command::RefreshVideoDevices))
                .count(),
            1
        );
        assert!(commands.contains(&Command::Restart(RestartTarget::All)));
        assert!(!commands.contains(&Command::Restart(RestartTarget::Webcam)));
        assert!(!commands.contains(&Command::Restart(RestartTarget::ScreenCapture)));
        assert!(pending.take().0.is_empty());
    }

    #[test]
    fn discovery_request_slots_keep_only_the_latest_pending_work() {
        let slot = CoalescingSlot::new();
        assert!(slot.replace(1_u64));
        assert!(slot.replace(2_u64));
        assert_eq!(slot.take(), Some(2));
        assert_eq!(slot.take(), None);
        assert!(slot.replace(3_u64));
        slot.clear();
        assert_eq!(slot.take(), None);
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
