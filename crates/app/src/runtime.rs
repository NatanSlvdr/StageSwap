use asc_core::{
    AppConfig, AppSnapshot, Command, DebouncedDetector, DetectorSettings, DeviceState, Frame,
    FrameMetadata, GrayImage, PIPELINE_FPS, PIPELINE_SIZE, RunState, Size, Source,
    SourceAvailability, TransitionController, bgra_to_gray, decide, image_similarity,
    resize_bilinear,
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

const COMMAND_CAPACITY: usize = 32;
const ACTIVITY_LIMIT: usize = 20;

pub struct RuntimeHandle {
    commands: SyncSender<Command>,
    snapshot: Arc<RwLock<AppSnapshot>>,
    worker: Option<JoinHandle<()>>,
}

impl RuntimeHandle {
    pub fn spawn(config: AppConfig) -> Self {
        let (commands, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let snapshot = Arc::new(RwLock::new(AppSnapshot {
            mode: config.output_mode,
            ..AppSnapshot::default()
        }));
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

struct RuntimeState {
    config: AppConfig,
    snapshot: AppSnapshot,
    transition: TransitionController,
    activity: VecDeque<String>,
    sequence: u64,
    started_at: Instant,
    reference: Option<GrayImage>,
    detector: DebouncedDetector,
    last_detection: Instant,
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
        let mut snapshot = AppSnapshot {
            mode,
            selected_video_device_id: config.selected_video_device_id.clone(),
            ..AppSnapshot::default()
        };
        snapshot.previews.reference = reference_preview;
        Self {
            snapshot,
            config,
            transition: TransitionController::default(),
            activity: VecDeque::with_capacity(ACTIVITY_LIMIT),
            sequence: 0,
            started_at: now,
            reference,
            detector,
            last_detection: now - Duration::from_millis(250),
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

    fn tick(&mut self, now: Instant) {
        if self.snapshot.run_state != RunState::Running {
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
        let output = Frame::blend(
            self.snapshot.previews.webcam.as_deref(),
            self.snapshot.previews.screen.as_deref(),
            self.snapshot.transition.screen_mix,
            self.config.placeholder_color_bgra,
            PIPELINE_SIZE,
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
}

fn gray_thumbnail(frame: &Frame) -> Option<GrayImage> {
    bgra_to_gray(frame.pixels(), frame.size, frame.stride as usize)
        .ok()
        .and_then(|image| resize_bilinear(&image, Size::new(160, 90)).ok())
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
    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(PIPELINE_FPS));
    loop {
        match commands.recv_timeout(frame_interval) {
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
        platform.refresh_inputs(&mut state);
        state.detect(Instant::now());
        state.tick(Instant::now());
        platform.publish(&mut state);
        *shared
            .write()
            .expect("runtime snapshot lock is not poisoned") = state.snapshot.clone();
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
                match VirtualCameraController::start(
                    pipe_name.clone(),
                    state.config.placeholder_color_bgra,
                ) {
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
        let mut platform = Self {
            publisher,
            camera,
            pipe_name,
            webcam,
            screen,
            selected_monitor,
            monitor_tracker,
            last_monitor_scan: Instant::now() - Duration::from_secs(30),
        };
        platform.rescan_monitors(state);
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
                }
                if config.placeholder_color_bgra != state.config.placeholder_color_bgra
                    && let Some(camera) = &mut self.camera
                    && let Err(error) = camera.update_placeholder(config.placeholder_color_bgra)
                {
                    state.snapshot.virtual_camera_state = DeviceState::Failed;
                    state.snapshot.warning = Some(error.clone());
                    state.record(format!("Virtual camera placeholder update failed: {error}"));
                }
                if config.similarity_threshold != state.config.similarity_threshold {
                    let mut tracker = MonitorTracker::new(MonitorTrackerSettings {
                        match_threshold: config.similarity_threshold,
                    });
                    if let Some(monitor) = self.selected_monitor.clone() {
                        tracker.select(monitor);
                    }
                    self.monitor_tracker = tracker;
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
                        self.rescan_monitors(state);
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
            Command::Rescan => self.rescan_monitors(state),
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
            VirtualCameraController::start(pipe_name.clone(), state.config.placeholder_color_bgra)
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
        if now.duration_since(self.last_monitor_scan) >= Duration::from_secs(30) {
            self.rescan_monitors(state);
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
        if webcam.is_some() {
            state.snapshot.previews.webcam = webcam;
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

    fn rescan_monitors(&mut self, state: &mut RuntimeState) {
        self.last_monitor_scan = Instant::now();
        if let Ok(monitors) = self.screen.enumerate() {
            state.snapshot.monitors = monitors.into();
        }
        let Some(reference) = state.reference.clone() else {
            return;
        };
        let scores = self.scan_scores(&reference, state.config.cursor_visible);
        let result = self.monitor_tracker.apply_scan(&scores);
        let result = if result.confirmation_pending {
            let confirmation = self.scan_scores(&reference, state.config.cursor_visible);
            self.monitor_tracker.apply_scan(&confirmation)
        } else {
            result
        };
        if let Some(monitor) = result.tracked
            && self.selected_monitor.as_ref() != Some(&monitor)
        {
            self.selected_monitor = Some(monitor);
            state.snapshot.selected_monitor = self.selected_monitor.clone();
            self.restart_screen(state);
            state.record("Reference monitor changed after two scans");
        }
    }

    fn scan_scores(&self, reference: &GrayImage, cursor_visible: bool) -> Vec<MonitorScore> {
        let Ok(monitors) = self.screen.enumerate() else {
            return Vec::new();
        };
        monitors
            .into_iter()
            .map(|monitor| {
                let mut capture = WindowsGraphicsScreenInput::default();
                let valid = capture.start(&monitor, cursor_visible).is_ok();
                let deadline = Instant::now() + Duration::from_millis(750);
                let frame = loop {
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
    }

    fn publish(&self, state: &mut RuntimeState) {
        if state.snapshot.run_state != RunState::Running {
            return;
        }
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
