use asc_core::{
    AppConfig, AppSnapshot, Command, DebouncedDetector, DetectorSettings, DeviceState, Frame,
    FrameCompositor, FrameMetadata, FramePacer, GrayImage, PIPELINE_FPS, RunState, Size, Source,
    SourceAvailability, TransitionController, bgra_to_gray, decide, image_similarity, off_frame,
    resize_bgra_to_gray, resize_bilinear,
};
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(windows)]
use asc_core::{MonitorScore, MonitorTracker, MonitorTrackerSettings, RestartTarget};
#[cfg(windows)]
use asc_windows::{
    FramePublisher, MediaFoundationVideoInput, ScreenInput, VideoInput, VirtualCameraController,
    WindowsGraphicsScreenInput, choose_video_device, configure_startup, frame_pipe_name,
};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

const COMMAND_CAPACITY: usize = 32;
const ACTIVITY_LIMIT: usize = 20;
const FPS_TRACKING_WINDOW: Duration = Duration::from_secs(1);
#[cfg(any(windows, test))]
const WEBCAM_CROP_ZOOM: f64 = 4.0 / 3.0;

pub struct RuntimeHandle {
    commands: SyncSender<Command>,
    snapshot: Arc<RwLock<AppSnapshot>>,
    worker: Option<JoinHandle<()>>,
}

impl RuntimeHandle {
    pub fn spawn(config: AppConfig) -> Self {
        let (commands, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let now = Instant::now();
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
            .name("asc-runtime".into())
            .spawn(move || run(config, receiver, worker_snapshot))
            .expect("runtime thread can be created");
        Self {
            commands,
            snapshot,
            worker: Some(worker),
        }
    }

    pub fn send(&self, command: Command) -> Result<(), mpsc::SendError<Command>> {
        self.commands.send(command)
    }

    pub fn try_send(&self, command: Command) -> Result<(), mpsc::TrySendError<Command>> {
        self.commands.try_send(command)
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
        let _ = self.commands.send(Command::Exit);
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
    format: Option<(Size, u32)>,
    source_x: Vec<usize>,
    source_rows: Vec<usize>,
}

#[cfg(any(windows, test))]
impl WebcamCropCache {
    fn apply(&mut self, source: Arc<Frame>, enabled: bool) -> Arc<Frame> {
        if !enabled {
            return source;
        }
        if self
            .source
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, &source))
            && let Some(cropped) = &self.cropped
        {
            return Arc::clone(cropped);
        }
        if self.format != Some((source.size, source.stride)) {
            let crop_width = (source.size.width as f64 / WEBCAM_CROP_ZOOM)
                .round()
                .max(1.0) as u32;
            let crop_height = (source.size.height as f64 / WEBCAM_CROP_ZOOM)
                .round()
                .max(1.0) as u32;
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
            self.format = Some((source.size, source.stride));
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

struct RuntimeState {
    config: AppConfig,
    snapshot: AppSnapshot,
    transition: TransitionController,
    compositor: FrameCompositor,
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
    fn new(config: AppConfig) -> Self {
        let mode = config.output_mode;
        let (reference, reference_preview) = load_reference(&config.reference_image_path)
            .map_or((None, None), |(reference, preview)| {
                (Some(reference), Some(preview))
            });
        let detector = DebouncedDetector::new(DetectorSettings {
            threshold: config.similarity_threshold,
            ..DetectorSettings::default()
        });
        let now = Instant::now();
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
            Command::SetMode(mode) => {
                self.snapshot.mode = mode;
                self.config.output_mode = mode;
                self.record(format!("Output mode changed to {mode:?}"));
            }
            Command::UpdateSettings(config) => {
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
                self.detector = DebouncedDetector::new(DetectorSettings {
                    threshold: self.config.similarity_threshold,
                    ..DetectorSettings::default()
                });
                self.record("Settings updated");
            }
            Command::CaptureReference => self.record("Reference capture requested"),
            Command::ImportReference(path) => {
                let _ = path;
                self.record("Reference import requested");
            }
            Command::SelectMonitor(monitor) => {
                self.snapshot.selected_monitor = Some(monitor);
                self.record("Tracked monitor selected");
            }
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
        self.snapshot.previews.final_output = Some(Arc::new(output));
    }

    #[cfg(windows)]
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
        self.snapshot.detection = asc_core::DetectionState::Unknown;
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

#[cfg(windows)]
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

fn run(config: AppConfig, commands: Receiver<Command>, shared: Arc<RwLock<AppSnapshot>>) {
    let start_automatically = config.start_automatically;
    let mut state = RuntimeState::new(config);
    state.snapshot.availability = SourceAvailability::default();
    state.snapshot.webcam_state = DeviceState::Unavailable;
    state.snapshot.screen_state = DeviceState::Unavailable;
    state.snapshot.virtual_camera_state = DeviceState::Unavailable;
    state.snapshot.actual_output = Source::Placeholder;
    let mut platform = Platform::new(&mut state);
    if start_automatically {
        state.command(Command::Start);
    }
    let frame_interval = Duration::from_nanos(1_000_000_000 / u64::from(PIPELINE_FPS));
    let mut pacer = FramePacer::new(Instant::now(), frame_interval);
    loop {
        match commands.recv_timeout(pacer.wait_duration(Instant::now())) {
            Ok(command) => {
                platform.command(&command, &mut state);
                if !state.command(command) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(command) = commands.try_recv() {
            platform.command(&command, &mut state);
            if !state.command(command) {
                return;
            }
        }
        let now = Instant::now();
        if !pacer.is_due(now, Duration::ZERO) {
            continue;
        }
        pacer.advance(now);
        platform.refresh_inputs(&mut state);
        state.refresh_input_fps(now);
        state.detect(now);
        state.tick(now);
        state.snapshot.output_fps = state.output_fps.observe(now);
        platform.publish(&mut state);
        *shared
            .write()
            .expect("runtime snapshot lock is not poisoned") = state.snapshot.clone();
    }
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
    monitors: Vec<asc_core::MonitorDescriptor>,
    scores: Vec<MonitorScore>,
}

#[cfg(windows)]
struct MonitorScanWorker {
    requests: Option<SyncSender<MonitorScanRequest>>,
    results: Option<Receiver<MonitorScanResult>>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    in_flight: bool,
}

#[cfg(windows)]
impl MonitorScanWorker {
    fn start() -> Result<Self, String> {
        let (request_sender, request_receiver) = mpsc::sync_channel::<MonitorScanRequest>(1);
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("asc-monitor-scan".into())
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
        })
    }

    fn request(&mut self, request: MonitorScanRequest) -> bool {
        if self.in_flight {
            return false;
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
        Some(result)
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
    selected_monitor: Option<asc_core::MonitorDescriptor>,
    monitor_tracker: MonitorTracker,
    last_monitor_scan: Instant,
    monitor_scan_generation: u64,
    monitor_scan_worker: Option<MonitorScanWorker>,
}

#[cfg(windows)]
impl Platform {
    fn new(state: &mut RuntimeState) -> Self {
        if let Err(error) = configure_startup(state.config.start_with_windows) {
            state.record(format!("Windows startup preference failed: {error}"));
        }
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
                        state.record("Virtual camera initialized");
                        Some(camera)
                    }
                    Err(error) => {
                        state.snapshot.virtual_camera_state = DeviceState::Failed;
                        state.snapshot.warning = Some(error.clone());
                        state.record(format!("Virtual camera failed: {error}"));
                        None
                    }
                }
            })
        });
        if publisher.is_none() {
            state.snapshot.virtual_camera_state = DeviceState::Failed;
        }
        let mut webcam = MediaFoundationVideoInput::default();
        match webcam.enumerate() {
            Ok(devices) => {
                state.snapshot.video_devices = devices
                    .iter()
                    .map(|device| asc_core::VideoDeviceChoice {
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
                    state.snapshot.webcam_state = DeviceState::Ready;
                    state.record("Webcam initialized");
                } else if devices.is_empty() {
                    state.record("No physical webcam found");
                } else {
                    state.record("Webcam selection required");
                }
            }
            Err(error) => {
                state.snapshot.webcam_state = DeviceState::Failed;
                state.record(format!("Webcam enumeration failed: {error}"));
            }
        }
        let mut screen = WindowsGraphicsScreenInput::default();
        let selected_monitor = match screen.enumerate() {
            Ok(monitors) => {
                state.snapshot.monitors = monitors.clone().into();
                monitors.into_iter().next().and_then(|monitor| {
                    match screen.start(&monitor, state.config.cursor_visible) {
                        Ok(()) => {
                            state.snapshot.screen_state = DeviceState::Ready;
                            state.record(format!("Screen capture initialized: {}", monitor.label));
                            Some(monitor)
                        }
                        Err(error) => {
                            state.snapshot.screen_state = DeviceState::Failed;
                            state.record(format!("Screen capture failed: {error}"));
                            None
                        }
                    }
                })
            }
            Err(error) => {
                state.snapshot.screen_state = DeviceState::Failed;
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
        state.snapshot.selected_monitor = selected_monitor.clone();
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
            monitor_tracker,
            last_monitor_scan: Instant::now() - Duration::from_secs(30),
            monitor_scan_generation: 1,
            monitor_scan_worker,
        };
        platform.request_monitor_scan(state);
        platform
    }

    fn command(&mut self, command: &Command, state: &mut RuntimeState) {
        match command {
            Command::UpdateSettings(config) => {
                if let Err(error) = configure_startup(config.start_with_windows) {
                    state.record(format!("Windows startup preference failed: {error}"));
                }
                if config.selected_video_device_id != state.config.selected_video_device_id {
                    self.webcam.stop();
                    if config.selected_video_device_id.is_empty() {
                        state.snapshot.webcam_state = DeviceState::Unavailable;
                    } else {
                        match self.webcam.start(&config.selected_video_device_id) {
                            Ok(()) => state.snapshot.webcam_state = DeviceState::Ready,
                            Err(error) => {
                                state.snapshot.webcam_state = DeviceState::Failed;
                                state.record(format!("Webcam selection failed: {error}"));
                            }
                        }
                    }
                }
                if config.cursor_visible != state.config.cursor_visible {
                    let old = state.config.cursor_visible;
                    state.config.cursor_visible = config.cursor_visible;
                    self.restart_screen(state);
                    state.config.cursor_visible = old;
                    self.monitor_scan_generation = self.monitor_scan_generation.wrapping_add(1);
                    self.last_monitor_scan = Instant::now() - Duration::from_secs(30);
                }
                if config.similarity_threshold != state.config.similarity_threshold {
                    let mut tracker = MonitorTracker::new(MonitorTrackerSettings {
                        match_threshold: config.similarity_threshold,
                    });
                    if let Some(monitor) = self.selected_monitor.clone() {
                        tracker.select(monitor);
                    }
                    self.monitor_tracker = tracker;
                    self.monitor_scan_generation = self.monitor_scan_generation.wrapping_add(1);
                    self.last_monitor_scan = Instant::now() - Duration::from_secs(30);
                }
            }
            Command::CaptureReference => {
                if let Some(frame) = self.screen.latest_frame() {
                    if let Some(reference) = gray_thumbnail(&frame) {
                        state.install_reference(reference, &frame, Instant::now());
                        match save_reference(&frame, &state.config.reference_image_path) {
                            Ok(()) => state.record("Reference captured"),
                            Err(error) => state.record(format!(
                                "Reference captured for this session but could not be saved: {error}"
                            )),
                        }
                        self.monitor_scan_generation = self.monitor_scan_generation.wrapping_add(1);
                        self.request_monitor_scan(state);
                    } else {
                        state.record("Reference capture failed: invalid screen frame");
                    }
                } else {
                    state.record("Reference capture failed: no screen frame");
                }
            }
            Command::ImportReference(path) => {
                match import_reference(path, &state.config.reference_image_path) {
                    Ok((reference, preview)) => {
                        state.install_reference(reference, &preview, Instant::now());
                        state.record("Reference imported into local storage");
                        self.monitor_scan_generation = self.monitor_scan_generation.wrapping_add(1);
                        self.request_monitor_scan(state);
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
                    self.monitor_tracker.select(monitor.clone());
                    self.monitor_scan_generation = self.monitor_scan_generation.wrapping_add(1);
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
                        .map(|device| asc_core::VideoDeviceChoice {
                            id: device.id,
                            name: device.name,
                        })
                        .collect::<Vec<_>>()
                        .into();
                    state.record("Video device list refreshed from settings");
                }
                Err(error) => state.record(format!("Video device refresh failed: {error}")),
            },
            Command::Rescan => self.request_monitor_scan(state),
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

    fn restart_webcam(&mut self, state: &mut RuntimeState) {
        let id = state.config.selected_video_device_id.clone();
        self.webcam.stop();
        if id.is_empty() {
            state.snapshot.webcam_state = DeviceState::Unavailable;
            state.record("Webcam restart skipped: no video input selected");
            return;
        }
        match self.webcam.start(&id) {
            Ok(()) => {
                state.snapshot.webcam_state = DeviceState::Ready;
                state.record("Webcam restarted");
            }
            Err(error) => {
                state.snapshot.webcam_state = DeviceState::Failed;
                state.record(format!("Webcam restart failed: {error}"));
            }
        }
    }

    fn restart_virtual_camera(&mut self, state: &mut RuntimeState) {
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
                state.snapshot.warning = None;
                state.record("Virtual camera restarted");
            }
            Err(error) => {
                state.snapshot.virtual_camera_state = DeviceState::Failed;
                state.snapshot.warning = Some(error.clone());
                state.record(format!("Virtual camera restart failed: {error}"));
            }
        }
    }

    fn refresh_inputs(&mut self, state: &mut RuntimeState) {
        let now = Instant::now();
        self.refresh_monitor_scan(state);
        if now.duration_since(self.last_monitor_scan) >= Duration::from_secs(30) {
            self.request_monitor_scan(state);
        }
        let webcam = self.webcam.latest_frame();
        state.snapshot.availability.camera_ready = webcam.is_some();
        if let Some(error) = self.webcam.last_error() {
            if state.snapshot.webcam_state != DeviceState::Failed {
                state.record(error);
            }
            state.snapshot.webcam_state = DeviceState::Failed;
        } else if webcam.is_some() && state.snapshot.webcam_state == DeviceState::Failed {
            state.snapshot.webcam_state = DeviceState::Ready;
            state.record("Webcam capture recovered");
        }
        if let Some(webcam) = webcam {
            state.snapshot.previews.webcam = Some(
                state
                    .webcam_crop
                    .apply(webcam, state.config.crop_webcam_to_16_9),
            );
        }
        let screen = self.screen.latest_frame();
        state.snapshot.availability.screen_ready = screen.is_some();
        if screen.is_some() {
            state.snapshot.previews.screen = screen;
        }
    }

    fn restart_screen(&mut self, state: &mut RuntimeState) {
        self.screen.stop();
        let Some(monitor) = self.selected_monitor.as_ref() else {
            state.snapshot.screen_state = DeviceState::Unavailable;
            return;
        };
        match self.screen.start(monitor, state.config.cursor_visible) {
            Ok(()) => {
                state.snapshot.screen_state = DeviceState::Ready;
                state.record("Screen capture restarted");
            }
            Err(error) => {
                state.snapshot.screen_state = DeviceState::Failed;
                state.record(format!("Screen capture restart failed: {error}"));
            }
        }
    }

    fn request_monitor_scan(&mut self, state: &RuntimeState) {
        let Some(worker) = &mut self.monitor_scan_worker else {
            return;
        };
        if worker.request(MonitorScanRequest {
            generation: self.monitor_scan_generation,
            reference: state.reference.clone(),
            cursor_visible: state.config.cursor_visible,
        }) {
            self.last_monitor_scan = Instant::now();
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
            self.request_monitor_scan(state);
            return;
        }
        state.snapshot.monitors = result.monitors.into();
        if result.scores.is_empty() {
            return;
        }
        let tracking = self.monitor_tracker.apply_scan(&result.scores);
        if tracking.confirmation_pending {
            self.request_monitor_scan(state);
            return;
        }
        if let Some(monitor) = tracking.tracked
            && self.selected_monitor.as_ref() != Some(&monitor)
        {
            self.selected_monitor = Some(monitor);
            state.snapshot.selected_monitor = self.selected_monitor.clone();
            self.restart_screen(state);
            state.record("Reference monitor changed after two scans");
        }
    }

    fn publish(&self, state: &mut RuntimeState) {
        let Some(frame) = state.snapshot.previews.final_output.as_deref() else {
            return;
        };
        if let Some(publisher) = &self.publisher
            && let Err(error) = publisher.publish(frame)
            && state.snapshot.warning.as_deref() != Some(error.as_str())
        {
            state.snapshot.warning = Some(error.clone());
            state.record(format!("Frame publish failed: {error}"));
        }
        if self
            .camera
            .as_ref()
            .is_some_and(|camera| !camera.is_running())
        {
            state.snapshot.virtual_camera_state = DeviceState::Failed;
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
    fn refresh_inputs(&mut self, _state: &mut RuntimeState) {}
    fn publish(&self, _state: &mut RuntimeState) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use asc_core::OutputMode;

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
    fn webcam_crop_cache_shares_processed_preview_with_camera_output() {
        let now = Instant::now();
        let size = Size::new(1280, 720);
        let mut pixels = vec![0; size.width as usize * size.height as usize * 4];
        for y in 0..size.height as usize {
            for x in 0..size.width as usize {
                let offset = (y * size.width as usize + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(
                    if x < 160 || x >= 1120 || y < 90 || y >= 630 {
                        &[0, 0, 255, 255]
                    } else {
                        &[0, 255, 0, 255]
                    },
                );
            }
        }
        let raw = Arc::new(Frame::new(pixels.into(), size, size.width * 4, 9, 321, now).unwrap());
        let mut state = RuntimeState::new(AppConfig::default());
        let processed = state.webcam_crop.apply(Arc::clone(&raw), true);
        let cached = state.webcam_crop.apply(Arc::clone(&raw), true);
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
            &state.webcam_crop.apply(Arc::clone(&raw), false),
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
    fn commands_publish_immutable_snapshots() {
        let runtime = RuntimeHandle::spawn(AppConfig {
            start_automatically: false,
            ..AppConfig::default()
        });
        runtime.send(Command::Start).unwrap();
        runtime
            .send(Command::SetMode(OutputMode::ForceScreen))
            .unwrap();
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
            asc_core::off_frame_pixels().as_ref()
        );

        runtime.send(Command::Start).unwrap();
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

        runtime.send(Command::Stop).unwrap();
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
            asc_core::off_frame_pixels().as_ref()
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
        assert_eq!(later.pixels(), asc_core::off_frame_pixels().as_ref());
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
            runtime.send(Command::SetMode(mode)).unwrap();
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
            runtime.send(Command::Restart(target)).unwrap();
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
