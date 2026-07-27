#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2,
};
use stageswap_app::RuntimeHandle;
use stageswap_core::{
    AdminProfileStatus, AdminProfileStore, AdminRestoreOutcome, AppConfig, AppSnapshot, Command,
    ConfigLoad, ConfigStore, DetectionState, DeviceState, Frame, OutputMode, RestartTarget,
    RunState, Source,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const WINDOW_ASPECT_RATIO: f32 = 16.0 / 9.0;
const MIN_WINDOW_HEIGHT: f32 = 600.0;
const WINDOW_TITLE: &str = concat!("StageSwap - v", env!("CARGO_PKG_VERSION"));
const LIVE_RED: Color32 = Color32::from_rgb(235, 90, 90);
const ACTIVE_GREEN: Color32 = Color32::from_rgb(76, 205, 132);
const TRANSITION_AMBER: Color32 = Color32::from_rgb(245, 190, 75);
const PREVIEW_NEUTRAL: Color32 = Color32::from_rgb(42, 47, 55);
const SETTINGS_BLUE: Color32 = Color32::from_rgb(64, 118, 216);
const SETTINGS_SWITCH_OFF: Color32 = Color32::from_rgb(49, 56, 68);
const VISIBLE_REFRESH: Duration = Duration::from_nanos(1_000_000_000 / 30);
const HIDDEN_REFRESH: Duration = Duration::from_millis(250);
const MAX_PREVIEW_TEXTURE_WIDTH: u32 = 480;
const MAX_PREVIEW_TEXTURE_HEIGHT: u32 = 270;
const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
const SETTINGS_ENTRANCE_DURATION: Duration = Duration::from_millis(160);
const SETTINGS_SECTION_DURATION: Duration = Duration::from_millis(120);
const DIALOG_ENTRANCE_DURATION: Duration = Duration::from_millis(150);
const DISCO_GESTURE_WINDOW: Duration = Duration::from_secs(3);
const SETTINGS_SIDEBAR_WIDTH: f32 = 196.0;
const SETTINGS_CONTENT_WIDTH: f32 = 960.0;
const SETTINGS_PREVIEW_WIDTH: f32 = 480.0;
const SETTINGS_PREVIEW_HEIGHT: f32 = 270.0;
const SETTINGS_PREVIEW_COLUMNS_BREAKPOINT: f32 = 700.0;
const SETTINGS_SIDEBAR_FILL: Color32 = Color32::from_rgb(20, 22, 27);
const SETTINGS_NAV_HOVERED: Color32 = Color32::from_rgb(32, 35, 41);
const SETTINGS_NAV_SELECTED: Color32 = Color32::from_rgb(45, 48, 55);
const SETTINGS_NAV_INDICATOR: Color32 = Color32::from_rgb(151, 157, 168);

mod app_icon;
mod deployment_payload;
mod local_log;
#[cfg(windows)]
mod tray;
use local_log::LocalLog;

fn main() -> eframe::Result {
    let _embedded_payload = deployment_payload::bytes();
    #[cfg(not(windows))]
    let ui_preview_request = match parse_ui_preview_request(&std::env::args().collect::<Vec<_>>()) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("StageSwap UI preview: {error}");
            eprintln!(
                "Usage: StageSwap --ui-preview [general|webcam|screen|matching|diagnostics|dialog-*]"
            );
            return Ok(());
        }
    };
    #[cfg(windows)]
    let launch_context = match stageswap_windows::portable_bootstrap(_embedded_payload) {
        Ok(stageswap_windows::BootstrapResult::Continue(context)) => context,
        Ok(stageswap_windows::BootstrapResult::Exit) => return Ok(()),
        Err(error) => {
            stageswap_windows::show_error_dialog("StageSwap installation failed", &error);
            return Ok(());
        }
    };
    #[cfg(windows)]
    let deployment_command = matches!(
        std::env::args().nth(1).as_deref(),
        Some("--register-elevated" | "--cleanup-elevated" | "--uninstall-elevated" | "--cleanup")
    );
    #[cfg(windows)]
    if deployment_command {
        match stageswap_windows::deployment_startup(_embedded_payload) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => deployment_failure(&error),
        }
    }
    #[cfg(windows)]
    let _single_instance = match stageswap_windows::SingleInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            if std::env::args().nth(1).as_deref() != Some("--startup")
                && let Err(error) = stageswap_windows::send_instance_command(
                    stageswap_windows::InstanceCommand::Show,
                )
            {
                stageswap_windows::show_error_dialog(
                    "StageSwap is already running",
                    &format!(
                        "Its window could not be opened. Exit the legacy tray instance and try again.\n\n{error}"
                    ),
                );
            }
            return Ok(());
        }
        Err(error) => {
            stageswap_windows::show_error_dialog("StageSwap could not start", &error);
            return Ok(());
        }
    };
    #[cfg(windows)]
    if !deployment_command {
        match stageswap_windows::deployment_startup(_embedded_payload) {
            Ok(false) => {}
            Ok(true) => return Ok(()),
            Err(error) => deployment_failure(&error),
        }
    }
    #[cfg(windows)]
    let (instance_sender, instance_receiver) = std::sync::mpsc::channel();
    #[cfg(windows)]
    let _instance_control = match stageswap_windows::InstanceControl::start(instance_sender) {
        Ok(control) => control,
        Err(error) => {
            stageswap_windows::show_error_dialog(
                "StageSwap could not start its local control service",
                &error,
            );
            return Ok(());
        }
    };
    #[cfg(windows)]
    let instance_readiness = _instance_control.readiness();
    #[cfg(windows)]
    let store = ConfigStore::new(local_data_directory());
    #[cfg(not(windows))]
    let store = ConfigStore::new(if ui_preview_request.is_some() {
        std::env::temp_dir().join(format!("StageSwap-ui-preview-{}", std::process::id()))
    } else {
        local_data_directory()
    });
    #[cfg(windows)]
    let loaded = load_config_with_admin_restore(&store);
    #[cfg(not(windows))]
    let loaded = if ui_preview_request.is_some() {
        ConfigLoad {
            config: ui_preview_config(),
            ..ConfigLoad::default()
        }
    } else {
        load_config_with_admin_restore(&store)
    };
    #[cfg(windows)]
    let mut loaded = loaded;
    #[cfg(windows)]
    if launch_context.mode == stageswap_windows::PortableMode::RunOnce
        && loaded.config.start_with_windows
    {
        loaded.config.start_with_windows = false;
        let _ = stageswap_windows::save_config_atomic(&store, &loaded.config);
    }
    #[cfg(windows)]
    let start_visible = launch_context.force_visible || !loaded.config.start_minimized;
    #[cfg(not(windows))]
    let start_visible = ui_preview_request.is_some() || !loaded.config.start_minimized;
    let app_icon = app_icon::load(None).expect("embedded app icon should decode");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([MIN_WINDOW_HEIGHT * WINDOW_ASPECT_RATIO, MIN_WINDOW_HEIGHT])
            .with_visible(start_visible)
            .with_icon(egui::IconData {
                rgba: app_icon.rgba,
                width: app_icon.width,
                height: app_icon.height,
            }),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |context| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = Color32::from_rgb(18, 20, 24);
            visuals.window_fill = Color32::from_rgb(23, 25, 30);
            visuals.selection.bg_fill = Color32::from_rgb(55, 115, 245);
            visuals.widgets.inactive.corner_radius = 6.into();
            visuals.widgets.hovered.corner_radius = 6.into();
            visuals.widgets.active.corner_radius = 6.into();
            visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 53, 63);
            visuals.widgets.active.bg_fill = Color32::from_rgb(59, 67, 82);
            context.egui_ctx.set_visuals(visuals);
            context.egui_ctx.style_mut_of(egui::Theme::Dark, |style| {
                style.spacing.item_spacing = egui::vec2(10.0, 10.0);
                style.spacing.button_padding = egui::vec2(14.0, 8.0);
            });
            let app = SwitcherApp::new(loaded.config, loaded.warnings, store);
            #[cfg(not(windows))]
            let app = if let Some(request) = ui_preview_request {
                app.with_ui_preview(request)
            } else {
                app
            };
            #[cfg(windows)]
            let app =
                app.with_launch_context(launch_context.mode, instance_receiver, instance_readiness);
            #[cfg(windows)]
            if let Some(visible) = initial_visibility_override(start_visible, app.tray.is_some()) {
                // eframe shows the root window after its first render even when the
                // viewport builder starts hidden. A viewport command is applied
                // afterwards, so reassert tray-hidden startup here.
                context
                    .egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Visible(visible));
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(windows)]
fn deployment_failure(error: &str) -> ! {
    stageswap_windows::show_error_dialog("StageSwap deployment failed", error);
    eprintln!("StageSwap deployment failed: {error}");
    std::process::exit(1);
}

#[cfg(any(windows, test))]
fn initial_visibility_override(start_visible: bool, tray_available: bool) -> Option<bool> {
    (!start_visible).then_some(!tray_available)
}

fn local_data_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
        .join("StageSwap")
}

fn load_config_with_admin_restore(store: &ConfigStore) -> ConfigLoad {
    let mut restore_warnings = Vec::new();
    let admin_store = AdminProfileStore::new(store.directory());
    if let Err(error) = restore_admin_profile(&admin_store) {
        restore_warnings.push(format!(
            "Could not restore the admin configuration: {error}"
        ));
    }
    let mut loaded = store.load();
    restore_warnings.append(&mut loaded.warnings);
    loaded.warnings = restore_warnings;
    loaded
}

#[cfg(windows)]
fn restore_admin_profile(store: &AdminProfileStore) -> std::io::Result<AdminRestoreOutcome> {
    store.restore_on_launch_with_replace(stageswap_windows::replace_file_atomic)
}

#[cfg(not(windows))]
fn restore_admin_profile(store: &AdminProfileStore) -> std::io::Result<AdminRestoreOutcome> {
    store.restore_on_launch()
}

#[cfg(windows)]
fn restore_admin_profile_now(store: &AdminProfileStore) -> std::io::Result<AdminRestoreOutcome> {
    store.restore_now_with_replace(stageswap_windows::replace_file_atomic)
}

#[cfg(not(windows))]
fn restore_admin_profile_now(store: &AdminProfileStore) -> std::io::Result<AdminRestoreOutcome> {
    store.restore_now()
}

fn aspect_locked_window_size(current: Vec2, previous: Option<Vec2>) -> Vec2 {
    let preserve_width = previous
        .is_none_or(|previous| (current.x - previous.x).abs() >= (current.y - previous.y).abs());
    let mut desired = if preserve_width {
        egui::vec2(current.x, current.x / WINDOW_ASPECT_RATIO)
    } else {
        egui::vec2(current.y * WINDOW_ASPECT_RATIO, current.y)
    };
    if desired.y < MIN_WINDOW_HEIGHT {
        desired = egui::vec2(MIN_WINDOW_HEIGHT * WINDOW_ASPECT_RATIO, MIN_WINDOW_HEIGHT);
    }
    desired
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SettingsTab {
    General,
    Webcam,
    Screen,
    Matching,
    Diagnostics,
}

impl SettingsTab {
    #[cfg(test)]
    const ALL: [Self; 5] = [
        Self::General,
        Self::Webcam,
        Self::Screen,
        Self::Matching,
        Self::Diagnostics,
    ];

    const PRIMARY: [(Self, UiIcon, &'static str); 4] = [
        (Self::General, UiIcon::Settings, "General"),
        (Self::Webcam, UiIcon::Camera, "Webcam"),
        (Self::Screen, UiIcon::Monitor, "Screen"),
        (Self::Matching, UiIcon::Target, "Matching"),
    ];

    const DIAGNOSTICS: (Self, UiIcon, &'static str) =
        (Self::Diagnostics, UiIcon::Layers, "Diagnostics");

    const fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Webcam => "Webcam",
            Self::Screen => "Screen",
            Self::Matching => "Matching",
            Self::Diagnostics => "Diagnostics",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::General => "Choose how the app launches, stays available, and reports problems.",
            Self::Webcam => "Select, verify, and recover the camera used for webcam output.",
            Self::Screen => "Choose the display Automatic mode watches and how it is captured.",
            Self::Matching => "Teach Automatic mode when the screen should show the webcam.",
            Self::Diagnostics => {
                "Inspect component health, technical details, logs, and recovery tools."
            }
        }
    }
}

#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UiPreviewRequest {
    target: UiPreviewTarget,
}

#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiPreviewTarget {
    Settings(SettingsTab),
    Dialog(AppDialogKind),
}

#[cfg(not(windows))]
impl SettingsTab {
    #[cfg(test)]
    const fn preview_name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Webcam => "webcam",
            Self::Screen => "screen",
            Self::Matching => "matching",
            Self::Diagnostics => "diagnostics",
        }
    }

    fn from_preview_name(value: &str) -> Option<Self> {
        match value {
            "general" => Some(Self::General),
            "webcam" => Some(Self::Webcam),
            "screen" => Some(Self::Screen),
            "matching" => Some(Self::Matching),
            "diagnostics" => Some(Self::Diagnostics),
            _ => None,
        }
    }
}

#[cfg(not(windows))]
fn parse_ui_preview_request(args: &[String]) -> Result<Option<UiPreviewRequest>, String> {
    let Some(preview_index) = args.iter().position(|argument| argument == "--ui-preview") else {
        return Ok(None);
    };
    let target = match args.get(preview_index + 1).map(String::as_str) {
        Some("dialog-exit") => UiPreviewTarget::Dialog(AppDialogKind::Exit),
        Some("dialog-clear-logs") => UiPreviewTarget::Dialog(AppDialogKind::ClearLogs),
        Some("dialog-admin") => UiPreviewTarget::Dialog(AppDialogKind::Admin),
        Some("dialog-replace-baseline") => {
            UiPreviewTarget::Dialog(AppDialogKind::ReplaceAdminBaseline)
        }
        Some("dialog-load-admin-config") => UiPreviewTarget::Dialog(AppDialogKind::LoadAdminConfig),
        Some("dialog-remove-baseline") => {
            UiPreviewTarget::Dialog(AppDialogKind::RemoveAdminBaseline)
        }
        Some(value) if !value.starts_with("--") => SettingsTab::from_preview_name(value)
            .map(UiPreviewTarget::Settings)
            .ok_or_else(|| {
                format!(
                    "unknown UI preview '{value}'; expected a Settings page or dialog-* preview"
                )
            })?,
        _ => UiPreviewTarget::Settings(SettingsTab::General),
    };
    Ok(Some(UiPreviewRequest { target }))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AppView {
    #[default]
    Dashboard,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum AppDialogKind {
    Exit,
    ClearLogs,
    Admin,
    ReplaceAdminBaseline,
    LoadAdminConfig,
    RemoveAdminBaseline,
}

#[derive(Clone, Copy, Debug)]
struct ActiveDialog {
    kind: AppDialogKind,
    opened_at: Instant,
    focus_safe_action: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DialogAction {
    Dismiss,
    Exit,
    ClearLogs,
    SaveAdminBaseline,
    ReplaceAdminBaseline,
    LoadAdminConfig,
    RemoveAdminBaseline,
    SetAdminAutoRestore(bool),
}

#[derive(Clone, Debug, Default)]
enum SettingsSaveState {
    #[default]
    Saved,
    Pending(Instant),
    Failed(String),
}

#[derive(Default)]
struct DiscoDiagnosticsGesture {
    first_click_at: Option<Instant>,
    click_count: u8,
}

impl DiscoDiagnosticsGesture {
    fn register_primary_click(&mut self, now: Instant) -> bool {
        if self
            .first_click_at
            .is_none_or(|started| now.saturating_duration_since(started) > DISCO_GESTURE_WINDOW)
        {
            self.first_click_at = Some(now);
            self.click_count = 1;
            return false;
        }
        self.click_count += 1;
        if self.click_count < 5 {
            return false;
        }
        self.reset();
        true
    }

    fn reset(&mut self) {
        self.first_click_at = None;
        self.click_count = 0;
    }
}

struct PreviewTexture {
    source: Arc<Frame>,
    size: [usize; 2],
    texture: TextureHandle,
}

struct PreviewJob {
    frame: Arc<Frame>,
    size: [usize; 2],
}

struct PreparedPreview {
    frame: Arc<Frame>,
    size: [usize; 2],
    image: egui::ColorImage,
}

#[derive(Default)]
struct PreviewConverterState {
    latest_request: Option<(Arc<Frame>, [usize; 2])>,
    pending: Option<PreviewJob>,
    ready: Option<PreparedPreview>,
    stopping: bool,
}

struct PreviewConverter {
    shared: Arc<(Mutex<PreviewConverterState>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl PreviewConverter {
    fn new(kind: PreviewKind) -> Self {
        let shared = Arc::new((Mutex::new(PreviewConverterState::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name(format!("stageswap-preview-{}", kind.key()))
            .spawn(move || preview_converter_loop(&worker_shared))
            .expect("preview conversion worker can be created");
        Self {
            shared,
            worker: Some(worker),
        }
    }

    fn submit(&self, frame: Arc<Frame>, size: [usize; 2]) {
        let (state, wake) = &*self.shared;
        let mut state = state
            .lock()
            .expect("preview converter state is not poisoned");
        if state
            .latest_request
            .as_ref()
            .is_some_and(|(requested, requested_size)| {
                Arc::ptr_eq(requested, &frame) && *requested_size == size
            })
        {
            return;
        }
        state.latest_request = Some((Arc::clone(&frame), size));
        state.pending = Some(PreviewJob { frame, size });
        wake.notify_one();
    }

    fn take_ready(&self) -> Option<PreparedPreview> {
        self.shared
            .0
            .lock()
            .expect("preview converter state is not poisoned")
            .ready
            .take()
    }
}

impl Drop for PreviewConverter {
    fn drop(&mut self) {
        let (state, wake) = &*self.shared;
        if let Ok(mut state) = state.lock() {
            state.stopping = true;
            state.pending = None;
            wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn preview_converter_loop(shared: &Arc<(Mutex<PreviewConverterState>, Condvar)>) {
    loop {
        let job = {
            let (state, wake) = &**shared;
            let mut state = state
                .lock()
                .expect("preview converter state is not poisoned");
            while state.pending.is_none() && !state.stopping {
                state = wake
                    .wait(state)
                    .expect("preview converter state is not poisoned");
            }
            if state.stopping {
                return;
            }
            state
                .pending
                .take()
                .expect("pending preview job is present")
        };
        let prepared = PreparedPreview {
            image: frame_image(&job.frame, job.size),
            frame: Arc::clone(&job.frame),
            size: job.size,
        };
        let mut state = shared
            .0
            .lock()
            .expect("preview converter state is not poisoned");
        store_completed_preview(&mut state, prepared);
    }
}

fn store_completed_preview(state: &mut PreviewConverterState, prepared: PreparedPreview) {
    state.ready = Some(prepared);
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PreviewKind {
    Webcam,
    Screen,
    Reference,
    Output,
}

impl PreviewKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Webcam => "webcam",
            Self::Screen => "screen",
            Self::Reference => "reference",
            Self::Output => "output",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Webcam => "WEBCAM",
            Self::Screen => "SCREEN",
            Self::Reference => "REFERENCE",
            Self::Output => "OUTPUT",
        }
    }

    const fn icon(self) -> UiIcon {
        match self {
            Self::Webcam => UiIcon::Camera,
            Self::Screen => UiIcon::Monitor,
            Self::Reference => UiIcon::Image,
            Self::Output => UiIcon::Broadcast,
        }
    }

    const fn shows_fps(self) -> bool {
        !matches!(self, Self::Reference)
    }

    fn pipeline_fps(self, snapshot: &AppSnapshot) -> Option<u32> {
        match self {
            Self::Webcam => snapshot.webcam_fps,
            Self::Screen => snapshot.screen_fps,
            Self::Reference => None,
            Self::Output => snapshot.output_fps,
        }
    }

    const fn empty_message(self) -> &'static str {
        match self {
            Self::Webcam => "No webcam frame",
            Self::Screen => "No screen frame",
            Self::Reference => "No reference image",
            Self::Output => "No output frame",
        }
    }
}

#[cfg(not(windows))]
fn ui_preview_config() -> AppConfig {
    AppConfig {
        selected_video_device_id: "preview-camera".into(),
        selected_monitor_label: "Stage display".into(),
        start_with_windows: true,
        start_minimized: true,
        start_automatically: true,
        output_mode: OutputMode::Automatic,
        ..AppConfig::default()
    }
}

#[cfg(not(windows))]
fn ui_preview_snapshot() -> AppSnapshot {
    let webcam = ui_preview_frame(1, [46, 30, 18], [139, 83, 37]);
    let screen = ui_preview_frame(2, [39, 23, 15], [216, 118, 63]);
    AppSnapshot {
        run_state: RunState::Running,
        mode: OutputMode::Automatic,
        detection: DetectionState::Matching,
        automatic_target: Source::Camera,
        actual_output: Source::Camera,
        availability: stageswap_core::SourceAvailability {
            camera_ready: true,
            screen_ready: true,
        },
        webcam_state: DeviceState::Ready,
        screen_state: DeviceState::Ready,
        virtual_camera_state: DeviceState::Ready,
        webcam_fps: Some(30),
        screen_fps: Some(30),
        output_fps: Some(30),
        recent_activity: vec![
            "Reference display confirmed".into(),
            "Automatic output selected Camera".into(),
        ]
        .into(),
        previews: stageswap_core::PreviewFrames {
            final_output: Some(Arc::clone(&webcam)),
            webcam: Some(webcam),
            screen: Some(Arc::clone(&screen)),
            reference: Some(screen),
        },
        video_devices: vec![
            stageswap_core::VideoDeviceChoice {
                id: "preview-camera".into(),
                name: "Studio Camera".into(),
            },
            stageswap_core::VideoDeviceChoice {
                id: "preview-camera-secondary".into(),
                name: "Wide Camera".into(),
            },
        ]
        .into(),
        selected_video_device_id: "preview-camera".into(),
        monitors: vec![
            stageswap_core::MonitorDescriptor {
                display_name: "preview-display-1".into(),
                label: "Control display".into(),
                width: 1920,
                height: 1080,
                ..stageswap_core::MonitorDescriptor::default()
            },
            stageswap_core::MonitorDescriptor {
                display_name: "preview-display-2".into(),
                label: "Stage display".into(),
                x: 1920,
                width: 1920,
                height: 1080,
                ..stageswap_core::MonitorDescriptor::default()
            },
        ]
        .into(),
        selected_monitor: Some(stageswap_core::MonitorDescriptor {
            display_name: "preview-display-2".into(),
            label: "Stage display".into(),
            x: 1920,
            width: 1920,
            height: 1080,
            ..stageswap_core::MonitorDescriptor::default()
        }),
        ..AppSnapshot::default()
    }
}

#[cfg(not(windows))]
fn ui_preview_frame(sequence: u64, background: [u8; 3], accent: [u8; 3]) -> Arc<Frame> {
    let size = stageswap_core::Size::new(640, 360);
    let mut pixels = vec![0; size.width as usize * size.height as usize * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[background[2], background[1], background[0], 255]);
    }
    for y in 64..296 {
        for x in 72..568 {
            let offset = (y * size.width as usize + x) * 4;
            pixels[offset..offset + 4].copy_from_slice(&[accent[2], accent[1], accent[0], 255]);
        }
    }
    Arc::new(
        Frame::new(
            pixels.into(),
            size,
            size.width * 4,
            sequence,
            0,
            Instant::now(),
        )
        .expect("UI preview frame is valid"),
    )
}

#[derive(Clone, Copy)]
struct PreviewOptions {
    show_fps: bool,
    fps: Option<u32>,
    empty_message: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct SettingsPreviewControls {
    heading: Rect,
    preview: Rect,
    controls: Rect,
    side_by_side: bool,
}

#[derive(Debug)]
struct SettingsSidebarLayout {
    brand_icon: Rect,
    brand_title: Rect,
    brand_separator: Rect,
    back: Rect,
    primary_navigation: Vec<Rect>,
    diagnostics: Rect,
    save_status: Rect,
    go_back: bool,
}

#[derive(Debug)]
#[allow(dead_code)]
struct DashboardControlsLayout {
    sections: [Rect; 3],
    section_headings: [Rect; 3],
    health_indicators: [Rect; 5],
    section_dividers: [Rect; 2],
    other_actions: [Rect; 2],
}

#[derive(Debug)]
struct ControlsWorkspaceLayout {
    body: DashboardControlsLayout,
    footer: Rect,
}

#[derive(Clone, Copy)]
struct SettingsSection {
    icon: UiIcon,
    title: &'static str,
    description: &'static str,
}

#[derive(Clone, Copy)]
struct SettingsPreview<'a> {
    kind: PreviewKind,
    frame: Option<&'a Arc<Frame>>,
    label: &'a str,
    empty_message: &'static str,
    actual_output: Source,
}

impl PreviewOptions {
    const fn dashboard(kind: PreviewKind, fps: Option<u32>) -> Self {
        Self {
            show_fps: kind.shows_fps(),
            fps,
            empty_message: kind.empty_message(),
        }
    }

    const fn settings(empty_message: &'static str) -> Self {
        Self {
            show_fps: false,
            fps: None,
            empty_message,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewContour {
    Neutral,
    Active,
    Live,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiIcon {
    Back,
    Bell,
    Broadcast,
    Camera,
    Capture,
    Check,
    Error,
    Folder,
    Image,
    Info,
    Layers,
    Loader,
    Monitor,
    Play,
    Question,
    Refresh,
    Robot,
    Route,
    Settings,
    Stop,
    Target,
    Unavailable,
    Window,
    Wrench,
}

const SETTINGS_RECOVERY_TARGETS: [(UiIcon, &str, f32, RestartTarget); 4] = [
    (
        UiIcon::Camera,
        "Restart webcam",
        148.0,
        RestartTarget::Webcam,
    ),
    (
        UiIcon::Monitor,
        "Restart screen capture",
        184.0,
        RestartTarget::ScreenCapture,
    ),
    (
        UiIcon::Broadcast,
        "Restart virtual camera",
        188.0,
        RestartTarget::VirtualCamera,
    ),
    (UiIcon::Layers, "Restart all", 126.0, RestartTarget::All),
];

#[cfg(not(windows))]
struct UiPreviewSession {
    snapshot: AppSnapshot,
}

struct SwitcherApp {
    config: AppConfig,
    runtime: RuntimeHandle,
    store: ConfigStore,
    load_warnings: Vec<String>,
    view: AppView,
    settings_tab: SettingsTab,
    settings_save_state: SettingsSaveState,
    admin_profile_status: Option<AdminProfileStatus>,
    active_dialog: Option<ActiveDialog>,
    awaiting_video_device_id: Option<String>,
    awaiting_monitor_label: Option<String>,
    settings_opened_at: Option<Instant>,
    settings_section_changed_at: Option<Instant>,
    app_icon_texture: Option<TextureHandle>,
    textures: HashMap<&'static str, PreviewTexture>,
    preview_converters: HashMap<PreviewKind, PreviewConverter>,
    log: LocalLog,
    last_activity: Option<String>,
    #[cfg(windows)]
    last_notified_warning: Option<String>,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    #[cfg(windows)]
    portable_mode: stageswap_windows::PortableMode,
    #[cfg(windows)]
    instance_commands: Option<std::sync::mpsc::Receiver<stageswap_windows::InstanceCommand>>,
    #[cfg(windows)]
    instance_readiness: Option<stageswap_windows::InstanceReadiness>,
    #[cfg(not(windows))]
    ui_preview: Option<UiPreviewSession>,
    exit_requested: bool,
    last_window_size: Option<Vec2>,
    disco_diagnostics_gesture: DiscoDiagnosticsGesture,
    disco_ui_activated_at: Option<Instant>,
    ui_animation_started_at: Instant,
}

impl SwitcherApp {
    fn new(mut config: AppConfig, mut load_warnings: Vec<String>, store: ConfigStore) -> Self {
        if config.reference_image_path.is_empty() {
            config.reference_image_path = store.reference_path().display().to_string();
        }
        let admin_profile_status = match AdminProfileStore::new(store.directory()).status() {
            Ok(status) => status,
            Err(error) => {
                let warning = format!("Could not read the admin configuration: {error}");
                if !load_warnings
                    .iter()
                    .any(|warning| warning.contains("admin configuration"))
                {
                    load_warnings.push(warning);
                }
                None
            }
        };
        let log = LocalLog::new(store.logs_path(), 14);
        for warning in &load_warnings {
            log.write("warning", "configuration", "LOAD_WARNING", warning);
        }
        Self {
            runtime: RuntimeHandle::spawn(config.clone()),
            config,
            store,
            load_warnings,
            view: AppView::Dashboard,
            settings_tab: SettingsTab::General,
            settings_save_state: SettingsSaveState::Saved,
            admin_profile_status,
            active_dialog: None,
            awaiting_video_device_id: None,
            awaiting_monitor_label: None,
            settings_opened_at: None,
            settings_section_changed_at: None,
            app_icon_texture: None,
            textures: HashMap::new(),
            preview_converters: HashMap::new(),
            log,
            last_activity: None,
            #[cfg(windows)]
            last_notified_warning: None,
            #[cfg(windows)]
            tray: tray::Tray::new().ok(),
            #[cfg(windows)]
            portable_mode: stageswap_windows::PortableMode::Managed,
            #[cfg(windows)]
            instance_commands: None,
            #[cfg(windows)]
            instance_readiness: None,
            #[cfg(not(windows))]
            ui_preview: None,
            exit_requested: false,
            last_window_size: None,
            disco_diagnostics_gesture: DiscoDiagnosticsGesture::default(),
            disco_ui_activated_at: None,
            ui_animation_started_at: Instant::now(),
        }
    }

    #[cfg(windows)]
    fn with_launch_context(
        mut self,
        portable_mode: stageswap_windows::PortableMode,
        instance_commands: std::sync::mpsc::Receiver<stageswap_windows::InstanceCommand>,
        instance_readiness: stageswap_windows::InstanceReadiness,
    ) -> Self {
        self.portable_mode = portable_mode;
        self.instance_commands = Some(instance_commands);
        self.instance_readiness = Some(instance_readiness);
        self
    }

    #[cfg(not(windows))]
    fn with_ui_preview(mut self, request: UiPreviewRequest) -> Self {
        self.view = AppView::Settings;
        match request.target {
            UiPreviewTarget::Settings(tab) => self.settings_tab = tab,
            UiPreviewTarget::Dialog(kind) => {
                self.settings_tab = if kind == AppDialogKind::ClearLogs {
                    SettingsTab::Diagnostics
                } else {
                    SettingsTab::General
                };
                if matches!(
                    kind,
                    AppDialogKind::Admin
                        | AppDialogKind::ReplaceAdminBaseline
                        | AppDialogKind::LoadAdminConfig
                        | AppDialogKind::RemoveAdminBaseline
                ) {
                    self.admin_profile_status = Some(AdminProfileStatus {
                        auto_restore_on_launch: true,
                        reference_included: true,
                    });
                }
                self.open_dialog(kind);
            }
        }
        self.settings_opened_at = None;
        self.settings_section_changed_at = None;
        self.ui_preview = Some(UiPreviewSession {
            snapshot: ui_preview_snapshot(),
        });
        self
    }

    fn snapshot(&self) -> AppSnapshot {
        #[cfg(not(windows))]
        if let Some(preview) = &self.ui_preview {
            return preview.snapshot.clone();
        }
        self.runtime.snapshot()
    }

    fn send(&self, command: Command) {
        let _ = self.runtime.send(command);
    }

    fn toggle_disco(&mut self) {
        #[cfg(not(windows))]
        if let Some(preview) = self.ui_preview.as_mut() {
            let enabled = !preview.snapshot.disco_enabled;
            preview.snapshot.disco_enabled = enabled;
            let now = Instant::now();
            self.disco_ui_activated_at = enabled.then_some(now);
            return;
        }

        let enabled = !self.snapshot().disco_enabled;
        if self.runtime.send(Command::ToggleDisco).is_ok() {
            let now = Instant::now();
            self.disco_ui_activated_at = enabled.then_some(now);
        }
    }

    fn set_mode(&mut self, mode: OutputMode) {
        if self.config.output_mode == mode {
            return;
        }
        self.config.output_mode = mode;
        self.send(Command::SetMode(mode));
        self.queue_settings_save();
    }

    fn queue_settings_save(&mut self) {
        self.settings_save_state = SettingsSaveState::Pending(Instant::now());
    }

    fn settings_save_due(&self, now: Instant) -> bool {
        matches!(
            &self.settings_save_state,
            SettingsSaveState::Pending(started_at)
                if now.saturating_duration_since(*started_at) >= SETTINGS_SAVE_DEBOUNCE
        )
    }

    fn flush_settings(&mut self) {
        if !matches!(self.settings_save_state, SettingsSaveState::Pending(_)) {
            return;
        }
        self.send(Command::UpdateSettings(Box::new(self.config.clone())));
        match save_config(&self.store, &self.config) {
            Ok(()) => self.settings_save_state = SettingsSaveState::Saved,
            Err(error) => {
                let message = format!("Could not save settings: {error}");
                self.load_warnings.push(message.clone());
                self.settings_save_state = SettingsSaveState::Failed(message);
            }
        }
    }

    fn sync_selected_monitor_preference(&mut self, snapshot: &AppSnapshot) {
        let Some(monitor) = snapshot.selected_monitor.as_ref() else {
            return;
        };
        if self.config.selected_monitor_label == monitor.label {
            return;
        }
        self.config
            .selected_monitor_label
            .clone_from(&monitor.label);
        if let Err(error) = save_config(&self.store, &self.config) {
            self.load_warnings
                .push(format!("Could not save monitor selection: {error}"));
        }
    }

    fn open_settings(&mut self) {
        self.view = AppView::Settings;
        self.disco_diagnostics_gesture.reset();
        self.settings_opened_at = Some(Instant::now());
        self.settings_section_changed_at = Some(Instant::now());
        self.send(Command::RefreshVideoDevices);
        if self.config.automatic_monitor_rescans {
            self.send(Command::Rescan);
        }
    }

    fn close_settings(&mut self) {
        self.flush_settings();
        self.view = AppView::Dashboard;
        self.disco_diagnostics_gesture.reset();
        self.active_dialog = None;
        self.settings_opened_at = None;
        self.settings_section_changed_at = None;
    }

    fn open_dialog(&mut self, kind: AppDialogKind) {
        self.active_dialog = Some(ActiveDialog {
            kind,
            opened_at: Instant::now(),
            focus_safe_action: true,
        });
    }

    fn dismiss_dialog(&mut self) {
        self.active_dialog = None;
    }

    fn dialog_is(&self, kind: AppDialogKind) -> bool {
        self.active_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.kind == kind)
    }

    fn open_admin_configuration(&mut self) {
        match AdminProfileStore::new(self.store.directory()).status() {
            Ok(status) => self.admin_profile_status = status,
            Err(error) => {
                self.admin_profile_status = None;
                self.record_admin_failure(format!(
                    "Could not read the existing admin config: {error}"
                ));
            }
        }
        self.open_dialog(AppDialogKind::Admin);
    }

    fn save_admin_baseline(&mut self) {
        let admin_store = AdminProfileStore::new(self.store.directory());
        match save_admin_profile(&admin_store, &self.config) {
            Ok(status) => {
                self.admin_profile_status = Some(status);
                self.open_dialog(AppDialogKind::Admin);
                self.log.write(
                    "info",
                    "configuration",
                    "ADMIN_BASELINE_SAVED",
                    "Admin config saved",
                );
            }
            Err(error) => {
                self.record_admin_failure(format!("Could not save the admin config: {error}"));
            }
        }
    }

    fn load_admin_config(&mut self) {
        let admin_store = AdminProfileStore::new(self.store.directory());
        match restore_admin_profile_now(&admin_store) {
            Ok(AdminRestoreOutcome::Restored) => {
                let mut loaded = self.store.load();
                for warning in loaded.warnings.drain(..) {
                    self.log
                        .write("warning", "configuration", "LOAD_WARNING", &warning);
                    self.load_warnings.push(warning);
                }

                let selected_monitor = {
                    let snapshot = self.snapshot();
                    snapshot
                        .monitors
                        .iter()
                        .find(|monitor| monitor.label == loaded.config.selected_monitor_label)
                        .or_else(|| snapshot.monitors.get(1))
                        .or_else(|| snapshot.monitors.first())
                        .cloned()
                };
                self.awaiting_video_device_id =
                    Some(loaded.config.selected_video_device_id.clone());
                self.awaiting_monitor_label = selected_monitor
                    .as_ref()
                    .map(|monitor| monitor.label.clone());
                self.config = loaded.config;
                self.settings_save_state = SettingsSaveState::Saved;
                self.send(Command::ReloadSettings(Box::new(self.config.clone())));
                if let Some(monitor) = selected_monitor {
                    self.send(Command::SelectMonitor(monitor));
                }
                if self.config.automatic_monitor_rescans {
                    self.send(Command::Rescan);
                }
                self.open_dialog(AppDialogKind::Admin);
                self.log.write(
                    "info",
                    "configuration",
                    "ADMIN_CONFIG_LOADED",
                    "Admin config loaded",
                );
            }
            Ok(AdminRestoreOutcome::Missing) => {
                self.admin_profile_status = None;
                self.record_admin_failure("No admin config is saved.".into());
            }
            Ok(AdminRestoreOutcome::Disabled) => {
                self.record_admin_failure("Could not load the admin config.".into());
            }
            Err(error) => {
                self.record_admin_failure(format!("Could not load the admin config: {error}"));
            }
        }
    }

    fn set_admin_auto_restore(&mut self, enabled: bool) {
        let admin_store = AdminProfileStore::new(self.store.directory());
        match set_admin_auto_restore(&admin_store, enabled) {
            Ok(status) => {
                self.admin_profile_status = Some(status);
                self.log.write(
                    "info",
                    "configuration",
                    "ADMIN_AUTO_RESTORE_CHANGED",
                    if enabled {
                        "Admin auto-restore enabled"
                    } else {
                        "Admin auto-restore disabled"
                    },
                );
            }
            Err(error) => {
                self.record_admin_failure(format!("Could not update auto-restore: {error}"));
            }
        }
    }

    fn remove_admin_baseline(&mut self) {
        let admin_store = AdminProfileStore::new(self.store.directory());
        match admin_store.remove() {
            Ok(_) => {
                self.admin_profile_status = None;
                self.open_dialog(AppDialogKind::Admin);
                self.log.write(
                    "info",
                    "configuration",
                    "ADMIN_BASELINE_REMOVED",
                    "Admin config deleted",
                );
            }
            Err(error) => {
                self.record_admin_failure(format!("Could not delete the admin config: {error}"));
            }
        }
    }

    fn record_admin_failure(&mut self, message: String) {
        self.log.write(
            "warning",
            "configuration",
            "ADMIN_CONFIGURATION_FAILED",
            &message,
        );
        self.load_warnings.push(message);
    }

    fn import_reference_dialog(&mut self) {
        #[cfg(windows)]
        if let Some(path) = stageswap_windows::pick_reference_image() {
            self.send(Command::ImportReference(path));
        }
        #[cfg(not(windows))]
        self.load_warnings
            .push("Reference file dialogs are available in the Windows application".into());
    }

    fn open_log_directory(&mut self) {
        #[cfg(windows)]
        if let Err(error) = stageswap_windows::open_directory(self.log.directory()) {
            self.load_warnings.push(error);
        }
        #[cfg(not(windows))]
        self.load_warnings
            .push(format!("Log directory: {}", self.log.directory().display()));
    }

    fn export_logs(&mut self) {
        #[cfg(windows)]
        if let Some(path) = stageswap_windows::pick_log_export_path()
            && let Err(error) = self.log.export_to(&path)
        {
            self.load_warnings
                .push(format!("Could not export logs: {error}"));
        }
        #[cfg(not(windows))]
        self.load_warnings
            .push("Log export dialog is available in the Windows application".into());
    }

    fn clear_logs(&mut self) {
        match self.log.clear() {
            Ok(()) => self
                .log
                .write("info", "logging", "LOGS_CLEARED", "Diagnostic logs cleared"),
            Err(error) => self
                .load_warnings
                .push(format!("Could not clear logs: {error}")),
        }
    }

    fn request_log_clear(&mut self) {
        self.open_dialog(AppDialogKind::ClearLogs);
    }

    fn confirm_log_clear(&mut self) {
        if self.dialog_is(AppDialogKind::ClearLogs) {
            self.clear_logs();
            self.dismiss_dialog();
        }
    }

    fn root_ui(&mut self, ui: &mut egui::Ui) -> Rect {
        let context = ui.ctx().clone();
        let disco_enabled = self.snapshot().disco_enabled;
        let content_rect = ui
            .scope(|ui| {
                if disco_enabled {
                    let accent = disco_ui_color(
                        Instant::now().saturating_duration_since(self.ui_animation_started_at),
                        0.0,
                    );
                    ui.visuals_mut().selection.bg_fill = accent;
                    ui.visuals_mut().widgets.hovered.bg_fill =
                        mix_color(Color32::from_rgb(48, 53, 63), accent, 0.22);
                    ui.visuals_mut().widgets.active.bg_fill =
                        mix_color(Color32::from_rgb(59, 67, 82), accent, 0.32);
                    ui.visuals_mut().panel_fill =
                        mix_color(Color32::from_rgb(18, 20, 24), accent, 0.08);
                }
                egui::CentralPanel::default()
                    .show(ui, |ui| {
                        let content_rect = ui.max_rect();
                        match self.view {
                            AppView::Dashboard => self.dashboard(ui),
                            AppView::Settings => self.settings_view(&context, ui),
                        }
                        content_rect
                    })
                    .inner
            })
            .inner;
        self.dialog(&context);
        self.paint_disco_interface(&context, content_rect, disco_enabled);
        content_rect
    }

    fn paint_disco_interface(
        &mut self,
        context: &egui::Context,
        content_rect: Rect,
        disco_enabled: bool,
    ) {
        let now = Instant::now();
        if !disco_enabled {
            return;
        }
        let elapsed = now.saturating_duration_since(self.ui_animation_started_at);
        let activation_elapsed = self
            .disco_ui_activated_at
            .map(|activated_at| now.saturating_duration_since(activated_at))
            .unwrap_or(Duration::from_secs(2));
        paint_disco_interface(
            context,
            content_rect,
            elapsed,
            activation_elapsed,
            disco_enabled,
        );
    }

    fn dashboard(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot();
        for warning in self
            .load_warnings
            .iter()
            .map(String::as_str)
            .chain(snapshot.warning.as_deref())
        {
            egui::Frame::new()
                .fill(Color32::from_rgb(74, 48, 22))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.label(RichText::new(warning).color(Color32::from_rgb(255, 210, 130)));
                });
        }
        ui.add_space(8.0);
        let available_width = ui.available_width();
        let preview_width = available_width * 0.70;
        let workspace_height = ui.available_height().max(0.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(preview_width, workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.preview_workspace(ui, &snapshot, preview_width, workspace_height);
                },
            );

            ui.separator();
            let controls_width = ui.available_width();
            let controls = ui.allocate_ui_with_layout(
                egui::vec2(controls_width, workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.controls_workspace(ui, &snapshot, controls_width, workspace_height),
            );
            debug_assert!(controls.inner.footer.is_positive());
            debug_assert!(controls.inner.body.sections.iter().all(Rect::is_positive));
        });
    }

    fn preview_workspace(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
        width: f32,
        height: f32,
    ) {
        const FOOTER_HEIGHT: f32 = 40.0;
        const FOOTER_GAP: f32 = 10.0;
        let app_icon_texture = self.app_icon_texture(ui.ctx());
        ui.allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::bottom_up(egui::Align::Center),
            |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, FOOTER_HEIGHT),
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        app_title(ui, app_icon_texture);
                    },
                );
                ui.add_space(FOOTER_GAP);
                let body_height = ui.available_height().max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, body_height),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        self.preview_grid(ui, snapshot, width, body_height);
                    },
                );
            },
        );
    }

    fn app_icon_texture(&mut self, context: &egui::Context) -> egui::TextureId {
        self.app_icon_texture
            .get_or_insert_with(|| {
                let icon = app_icon::load(Some(512)).expect("embedded app icon should decode");
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [icon.width as usize, icon.height as usize],
                    &icon.rgba,
                );
                context.load_texture("app-icon", image, TextureOptions::LINEAR)
            })
            .id()
    }

    fn preview_grid(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot, width: f32, height: f32) {
        let column_gap = ui.spacing().item_spacing.x;
        let row_gap = ui.spacing().item_spacing.y;
        let label_height = ui.text_style_height(&egui::TextStyle::Small);
        let cell_width = ((width - column_gap) / 2.0).max(48.0);
        let row_budget = ((height - row_gap) / 2.0).max(48.0);
        let maximum_preview_height = (row_budget - label_height - 24.0).max(24.0);
        let rendered_width = cell_width
            .min(maximum_preview_height * WINDOW_ASPECT_RATIO)
            .max(24.0 * WINDOW_ASPECT_RATIO);
        let rendered_height = rendered_width / WINDOW_ASPECT_RATIO;
        let cell_height = rendered_height + 8.0 + label_height + 16.0;
        let grid_height = cell_height * 2.0 + row_gap;
        ui.add_space(((height - grid_height) / 2.0).max(0.0));

        for row in [
            [
                (PreviewKind::Webcam, snapshot.previews.webcam.as_ref()),
                (PreviewKind::Screen, snapshot.previews.screen.as_ref()),
            ],
            [
                (PreviewKind::Reference, snapshot.previews.reference.as_ref()),
                (PreviewKind::Output, snapshot.previews.final_output.as_ref()),
            ],
        ] {
            ui.horizontal_top(|ui| {
                for (kind, frame) in row {
                    ui.allocate_ui_with_layout(
                        egui::vec2(cell_width, cell_height),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            self.preview_cell(
                                ui,
                                kind,
                                frame,
                                [rendered_width, rendered_height],
                                snapshot.actual_output,
                                kind.pipeline_fps(snapshot),
                            );
                        },
                    );
                }
            });
        }
    }

    fn controls_workspace(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
        width: f32,
        height: f32,
    ) -> ControlsWorkspaceLayout {
        const FOOTER_HEIGHT: f32 = 40.0;
        const FOOTER_GAP: f32 = 10.0;
        ui.allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::bottom_up(egui::Align::Center),
            |ui| {
                let settings = icon_button(
                    ui,
                    UiIcon::Settings,
                    "Settings",
                    egui::vec2(width, FOOTER_HEIGHT),
                    false,
                    true,
                );
                if settings.clicked() {
                    self.open_settings();
                }
                ui.add_space(FOOTER_GAP);
                let body_height = ui.available_height().max(80.0);
                let body = ui.allocate_ui_with_layout(
                    egui::vec2(width, body_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_size(egui::vec2(width, body_height));
                        self.controls_body(ui, snapshot)
                    },
                );
                ControlsWorkspaceLayout {
                    body: body.inner,
                    footer: settings.rect,
                }
            },
        )
        .inner
    }

    fn controls_body(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
    ) -> DashboardControlsLayout {
        let health_heading = controls_section_heading(ui, UiIcon::Check, "Components health");
        let health_indicators = [
            health_state_group(ui, UiIcon::Camera, "Webcam", snapshot.webcam_state),
            health_state_group(ui, UiIcon::Monitor, "Screen", snapshot.screen_state),
            health_state_group(
                ui,
                UiIcon::Broadcast,
                "Output",
                snapshot.virtual_camera_state,
            ),
            detection_state_group(ui, snapshot.detection),
            screen_mix_group(ui, snapshot.transition.screen_mix),
        ];
        let health_section = health_heading.union(health_indicators[4]);
        let first_divider = controls_section_divider(ui);

        let main_heading = controls_section_heading(ui, UiIcon::Route, "Main controls");
        let automation_running =
            matches!(snapshot.run_state, RunState::Running | RunState::Starting);
        let (run_icon, run_label, run_accent) = if automation_running {
            (UiIcon::Stop, "Stop automation", LIVE_RED)
        } else {
            (UiIcon::Play, "Start automation", ACTIVE_GREEN)
        };
        let automation = accent_icon_button(
            ui,
            run_icon,
            run_label,
            egui::vec2(ui.available_width(), 36.0),
            run_accent,
        );
        if automation.clicked() {
            if automation_running {
                self.send(Command::Stop);
            } else {
                self.send(Command::Start);
            }
        }

        ui.add_space(8.0);
        let gap = 4.0;
        let row_height = 30.0;
        let automatic_width = 72.0;
        let icon_width = row_height;
        let heading_width =
            (ui.available_width() - automatic_width - icon_width * 2.0 - gap * 3.0).max(72.0);
        let output_mode = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.horizontal(|ui| {
                    indicator_heading(
                        ui,
                        UiIcon::Route,
                        "Output mode",
                        None,
                        heading_width,
                        row_height,
                    );
                    if icon_button(
                        ui,
                        UiIcon::Robot,
                        "Auto",
                        egui::vec2(automatic_width, row_height),
                        snapshot.mode == OutputMode::Automatic,
                        false,
                    )
                    .clicked()
                    {
                        self.set_mode(OutputMode::Automatic);
                    }
                    if icon_button(
                        ui,
                        UiIcon::Camera,
                        "",
                        egui::vec2(icon_width, row_height),
                        snapshot.mode == OutputMode::ForceCamera,
                        false,
                    )
                    .on_hover_text("Webcam")
                    .clicked()
                    {
                        self.set_mode(OutputMode::ForceCamera);
                    }
                    if icon_button(
                        ui,
                        UiIcon::Monitor,
                        "",
                        egui::vec2(icon_width, row_height),
                        snapshot.mode == OutputMode::ForceScreen,
                        false,
                    )
                    .on_hover_text("Screen")
                    .clicked()
                    {
                        self.set_mode(OutputMode::ForceScreen);
                    }
                })
                .response
                .rect
            })
            .inner;
        let main_section = main_heading.union(output_mode);
        let second_divider = controls_section_divider(ui);

        let other_heading = controls_section_heading(ui, UiIcon::Wrench, "Other");
        let capture = icon_button(
            ui,
            UiIcon::Capture,
            "Capture reference",
            egui::vec2(ui.available_width(), 32.0),
            false,
            false,
        );
        if capture.clicked() {
            self.send(Command::CaptureReference);
        }
        let rescan = icon_button(
            ui,
            UiIcon::Refresh,
            "Rescan screens",
            egui::vec2(ui.available_width(), 32.0),
            false,
            false,
        );
        if rescan.clicked() {
            self.send(Command::Rescan);
        }
        let other_actions = [capture.rect, rescan.rect];
        let other_section = other_heading.union(other_actions[1]);

        DashboardControlsLayout {
            sections: [health_section, main_section, other_section],
            section_headings: [health_heading, main_heading, other_heading],
            health_indicators,
            section_dividers: [first_divider, second_divider],
            other_actions,
        }
    }

    fn settings_view(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        if self.active_dialog.is_none()
            && context.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.close_settings();
            return;
        }

        let entrance = animation_progress(self.settings_opened_at, SETTINGS_ENTRANCE_DURATION);
        if entrance < 1.0 {
            context.request_repaint();
        }
        ui.painter()
            .rect_filled(ui.max_rect(), 0, Color32::from_rgb(17, 19, 23));
        ui.set_min_size(ui.available_size());
        ui.scope(|ui| {
            ui.set_opacity(0.72 + entrance * 0.28);
            if self.settings_workspace(ui) {
                self.close_settings();
            }
        });
    }

    fn settings_workspace(&mut self, ui: &mut egui::Ui) -> bool {
        let workspace_height = ui.available_height().max(0.0);
        ui.horizontal_top(|ui| {
            let sidebar = ui.allocate_ui_with_layout(
                egui::vec2(SETTINGS_SIDEBAR_WIDTH, workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.settings_sidebar(ui, workspace_height),
            );

            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.settings_content(ui),
            );
            debug_assert!(sidebar.inner.brand_icon.is_positive());
            debug_assert!(sidebar.inner.brand_title.is_positive());
            debug_assert!(sidebar.inner.brand_separator.is_positive());
            debug_assert!(sidebar.inner.back.is_positive());
            debug_assert_eq!(
                sidebar.inner.primary_navigation.len(),
                SettingsTab::PRIMARY.len()
            );
            debug_assert!(sidebar.inner.diagnostics.is_positive());
            debug_assert!(sidebar.inner.save_status.is_positive());
            sidebar.inner.go_back
        })
        .inner
    }

    fn settings_sidebar(&mut self, ui: &mut egui::Ui, height: f32) -> SettingsSidebarLayout {
        let app_icon_texture = self.app_icon_texture(ui.ctx());
        let disco_enabled = self.snapshot().disco_enabled;
        let disco_elapsed = Instant::now().saturating_duration_since(self.ui_animation_started_at);
        let sidebar_fill = if disco_enabled {
            mix_color(
                SETTINGS_SIDEBAR_FILL,
                disco_ui_color(disco_elapsed, 0.58),
                0.12,
            )
        } else {
            SETTINGS_SIDEBAR_FILL
        };
        egui::Frame::new()
            .fill(sidebar_fill)
            .inner_margin(egui::Margin::symmetric(10, 12))
            .show(ui, |ui| {
                ui.set_width(SETTINGS_SIDEBAR_WIDTH - 20.0);
                ui.set_min_height((height - 24.0).max(0.0));

                let brand_icon = ui
                    .allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 96.0),
                        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                        |ui| {
                            let (rect, response) =
                                ui.allocate_exact_size(egui::vec2(96.0, 96.0), Sense::click());
                            if disco_enabled {
                                for ring in 0..3 {
                                    let color = disco_ui_color(disco_elapsed, ring as f32 / 3.0);
                                    ui.painter().circle_stroke(
                                        rect.center(),
                                        rect.width() * (0.51 + ring as f32 * 0.035),
                                        Stroke::new(
                                            3.0 - ring as f32 * 0.7,
                                            Color32::from_rgba_unmultiplied(
                                                color.r(),
                                                color.g(),
                                                color.b(),
                                                (190 - ring * 45) as u8,
                                            ),
                                        ),
                                    );
                                }
                            }
                            ui.painter().image(
                                app_icon_texture,
                                rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                            if response.clicked()
                                || response.clicked_by(egui::PointerButton::Secondary)
                            {
                                self.disco_diagnostics_gesture.reset();
                            }
                            if admin_logo_activated(&response) {
                                self.open_admin_configuration();
                            }
                            rect
                        },
                    )
                    .inner;
                ui.add_space(6.0);
                let brand_title = ui
                    .allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 22.0),
                        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                        |ui| {
                            ui.label(
                                RichText::new("StageSwap")
                                    .size(16.0)
                                    .strong()
                                    .color(Color32::from_rgb(220, 225, 234)),
                            )
                            .rect
                        },
                    )
                    .inner;
                ui.add_space(10.0);
                let brand_separator = ui.separator().rect;
                ui.add_space(8.0);
                let back = settings_back_button(ui);
                let go_back = back.clicked();
                if go_back {
                    self.disco_diagnostics_gesture.reset();
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new("PREFERENCES")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(112, 120, 134)),
                );
                ui.add_space(8.0);
                let mut primary_navigation = Vec::with_capacity(SettingsTab::PRIMARY.len());
                for (index, (tab, icon, label)) in SettingsTab::PRIMARY.into_iter().enumerate() {
                    let response = settings_nav_button(
                        ui,
                        tab,
                        icon,
                        label,
                        self.settings_tab,
                        disco_enabled.then(|| disco_ui_color(disco_elapsed, index as f32 * 0.13)),
                    );
                    primary_navigation.push(response.rect);
                    if response.clicked() {
                        self.disco_diagnostics_gesture.reset();
                    }
                    if response.clicked() && self.settings_tab != tab {
                        self.settings_tab = tab;
                        self.dismiss_dialog();
                        self.settings_section_changed_at = Some(Instant::now());
                    }
                    if index + 1 < SettingsTab::PRIMARY.len() {
                        ui.add_space(3.0);
                    }
                }

                let (diagnostics, save_status) = ui
                    .with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        let save_status = self.settings_save_indicator(ui);
                        ui.add_space(10.0);
                        let (tab, icon, label) = SettingsTab::DIAGNOSTICS;
                        let response = settings_nav_button(
                            ui,
                            tab,
                            icon,
                            label,
                            self.settings_tab,
                            disco_enabled.then(|| disco_ui_color(disco_elapsed, 0.76)),
                        );
                        if response.clicked_by(egui::PointerButton::Secondary) {
                            self.disco_diagnostics_gesture.reset();
                        }
                        if response.clicked_by(egui::PointerButton::Primary)
                            && self
                                .disco_diagnostics_gesture
                                .register_primary_click(Instant::now())
                        {
                            self.toggle_disco();
                        }
                        if response.clicked() && self.settings_tab != tab {
                            self.settings_tab = tab;
                            self.dismiss_dialog();
                            self.settings_section_changed_at = Some(Instant::now());
                        }
                        (response.rect, save_status)
                    })
                    .inner;

                SettingsSidebarLayout {
                    brand_icon,
                    brand_title,
                    brand_separator,
                    back: back.rect,
                    primary_navigation,
                    diagnostics,
                    save_status,
                    go_back,
                }
            })
            .inner
    }

    fn settings_save_indicator(&self, ui: &mut egui::Ui) -> Rect {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 42.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.label(
                    RichText::new("AUTOSAVE")
                        .size(9.5)
                        .strong()
                        .color(Color32::from_rgb(103, 110, 122)),
                );
                let (icon, label, color, detail) = match &self.settings_save_state {
                    SettingsSaveState::Saved => (
                        UiIcon::Check,
                        "Saved",
                        Color32::from_rgb(134, 213, 169),
                        None,
                    ),
                    SettingsSaveState::Pending(_) => (
                        UiIcon::Loader,
                        "Saving…",
                        Color32::from_rgb(173, 181, 194),
                        None,
                    ),
                    SettingsSaveState::Failed(message) => (
                        UiIcon::Error,
                        "Couldn’t save",
                        Color32::from_rgb(244, 133, 133),
                        Some(message.as_str()),
                    ),
                };
                let status = ui
                    .horizontal(|ui| icon_text(ui, icon, label, color, false))
                    .response;
                if let Some(detail) = detail {
                    status.on_hover_text(detail);
                }
            },
        )
        .response
        .rect
    }

    fn settings_content(&mut self, ui: &mut egui::Ui) {
        let section_progress =
            animation_progress(self.settings_section_changed_at, SETTINGS_SECTION_DURATION);
        if section_progress < 1.0 {
            ui.ctx().request_repaint();
        }
        let before = self.config.clone();
        egui::ScrollArea::vertical()
            .id_salt(("settings-content", self.settings_tab))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let available_width = ui.available_width();
                let content_width = (available_width - 28.0).min(SETTINGS_CONTENT_WIDTH);
                ui.allocate_ui_with_layout(
                    egui::vec2(available_width, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(content_width.max(1.0), ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_opacity(0.62 + section_progress * 0.38);
                                ui.add_space(18.0 + (1.0 - section_progress) * 5.0);
                                ui.label(
                                    RichText::new(self.settings_tab.title())
                                        .size(23.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(self.settings_tab.description())
                                        .size(13.0)
                                        .color(Color32::from_rgb(154, 161, 174)),
                                );
                                ui.add_space(18.0);
                                match self.settings_tab {
                                    SettingsTab::General => self.general_settings(ui),
                                    SettingsTab::Webcam => self.webcam_settings(ui),
                                    SettingsTab::Screen => self.screen_settings(ui),
                                    SettingsTab::Matching => self.matching_settings(ui),
                                    SettingsTab::Diagnostics => self.diagnostics_settings(ui),
                                }
                                ui.add_space(22.0);
                            },
                        );
                    },
                );
            });
        if self.config != before {
            if self.config.selected_video_device_id != before.selected_video_device_id {
                self.awaiting_video_device_id = Some(self.config.selected_video_device_id.clone());
            }
            self.queue_settings_save();
        }
    }

    fn general_settings(&mut self, ui: &mut egui::Ui) {
        settings_section_heading(
            ui,
            UiIcon::Route,
            "How StageSwap works",
            "StageSwap watches your selected screen. While it matches your saved reference image, your video calls see your webcam. When the screen changes, StageSwap automatically switches to the screen. When the reference returns, it switches back to your webcam.",
        );

        settings_section_gap(ui);
        settings_section_heading(
            ui,
            UiIcon::Play,
            "Startup",
            "Applied the next time StageSwap starts.",
        );
        #[cfg(windows)]
        if self.portable_mode == stageswap_windows::PortableMode::RunOnce {
            let mut disabled = false;
            ui.add_enabled_ui(false, |ui| {
                settings_toggle_row(
                    ui,
                    &mut disabled,
                    "Start with Windows",
                    "Install StageSwap to use a stable Windows startup path.",
                );
            });
            if ui.button("Install StageSwap to enable startup").clicked()
                && let Err(error) = stageswap_windows::request_install()
            {
                self.load_warnings
                    .push(format!("Could not start installation: {error}"));
            }
        } else {
            settings_toggle_row(
                ui,
                &mut self.config.start_with_windows,
                "Start with Windows",
                "Launch after Windows sign-in.",
            );
        }
        #[cfg(not(windows))]
        {
            settings_toggle_row(
                ui,
                &mut self.config.start_with_windows,
                "Start with Windows",
                "Launch after Windows sign-in.",
            );
        }
        settings_toggle_row(
            ui,
            &mut self.config.start_minimized,
            "Start minimized",
            "Open in the system tray.",
        );
        let automatic_result = start_automatically_result(
            self.config.start_automatically,
            self.config.start_minimized,
            self.config.output_mode,
        );
        settings_conditional_toggle_row(
            ui,
            &mut self.config.start_automatically,
            "Start automation on launch",
            &automatic_result,
        );

        settings_section_gap(ui);
        settings_section_heading(
            ui,
            UiIcon::Window,
            "Window behavior",
            "Choose what closing StageSwap does.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.close_to_tray,
            "Close window to tray",
            "Keep StageSwap running after closing the window.",
        );
        settings_toggle_row_without_separator(
            ui,
            &mut self.config.confirm_exit,
            "Confirm before exit",
            "Ask before StageSwap fully exits.",
        );
        ui.add_space(4.0);
        settings_result_text(
            ui,
            window_behavior_result(self.config.close_to_tray, self.config.confirm_exit),
        );
        ui.separator();

        settings_section_gap(ui);
        settings_section_heading(
            ui,
            UiIcon::Bell,
            "Notifications",
            "Important Windows warnings.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.show_notifications,
            "Show status notifications",
            "Notify when a component needs attention.",
        );
    }

    fn webcam_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot();
        let selected_name = snapshot
            .video_devices
            .iter()
            .find(|device| device.id == self.config.selected_video_device_id)
            .map(|device| device.name.clone())
            .unwrap_or_else(|| {
                if self.config.selected_video_device_id.is_empty() {
                    "No camera selected".into()
                } else {
                    "Saved camera is unavailable".into()
                }
            });
        self.settings_preview_control_row(
            ui,
            SettingsSection {
                icon: UiIcon::Camera,
                title: "Camera input",
                description: "Used by Camera mode and whenever Automatic selects Camera. Output is always 16:9.",
            },
            SettingsPreview {
                kind: PreviewKind::Webcam,
                frame: snapshot.previews.webcam.as_ref(),
                label: "Selected webcam",
                empty_message: "No webcam frame — choose a camera or refresh the device list.",
                actual_output: snapshot.actual_output,
            },
            |app, ui| {
                settings_device_status(ui, UiIcon::Camera, "Webcam", snapshot.webcam_state);
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Camera")
                        .size(12.0)
                        .color(Color32::from_rgb(224, 228, 235)),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let geometry = selector_utility_geometry(
                        ui.available_width(),
                        ui.spacing().item_spacing.x,
                    );
                    egui::ComboBox::from_id_salt("webcam-selector")
                        .width(geometry.selector_width)
                        .selected_text(selected_name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut app.config.selected_video_device_id,
                                String::new(),
                                "No camera selected",
                            );
                            for device in snapshot.video_devices.iter() {
                                ui.selectable_value(
                                    &mut app.config.selected_video_device_id,
                                    device.id.clone(),
                                    &device.name,
                                );
                            }
                        });
                    if icon_only_button(
                        ui,
                        UiIcon::Refresh,
                        "Refresh camera devices",
                        egui::vec2(geometry.action_width, 32.0),
                    )
                    .on_hover_text("Refresh camera devices")
                    .clicked()
                    {
                        app.send(Command::RefreshVideoDevices);
                    }
                });
                ui.add_space(8.0);
                settings_toggle_row(
                    ui,
                    &mut app.config.crop_webcam_to_16_9,
                    "Crop webcam to 16:9",
                    "Crop non-16:9 cameras to fill the frame.",
                );
            },
        );
    }

    fn screen_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot();
        let selected_monitor = snapshot
            .selected_monitor
            .as_ref()
            .map_or("No display selected", |monitor| monitor.label.as_str());
        self.settings_preview_control_row(
            ui,
            SettingsSection {
                icon: UiIcon::Monitor,
                title: "Screen capture",
                description: "This display is used by Display mode and watched by Automatic mode for reference changes.",
            },
            SettingsPreview {
                kind: PreviewKind::Screen,
                frame: snapshot.previews.screen.as_ref(),
                label: "Live screen",
                empty_message: "No screen frame — choose a display or use Recovery in Diagnostics.",
                actual_output: snapshot.actual_output,
            },
            |app, ui| {
                settings_device_status(ui, UiIcon::Monitor, "Capture", snapshot.screen_state);
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Display")
                        .size(12.0)
                        .color(Color32::from_rgb(224, 228, 235)),
                );
                ui.add_space(4.0);
                egui::ComboBox::from_id_salt("monitor-selector")
                    .width(ui.available_width())
                    .selected_text(selected_monitor)
                    .show_ui(ui, |ui| {
                        for monitor in snapshot.monitors.iter() {
                            let label =
                                format!("{} — {}×{}", monitor.label, monitor.width, monitor.height);
                            if ui
                                .selectable_label(
                                    snapshot.selected_monitor.as_ref() == Some(monitor),
                                    label,
                                )
                                .clicked()
                            {
                                app.send(Command::SelectMonitor(monitor.clone()));
                            }
                        }
                    });
                ui.add_space(12.0);
                settings_group_label(ui, "Capture behavior");
                settings_toggle_row(
                    ui,
                    &mut app.config.cursor_visible,
                    "Include mouse cursor",
                    "New references follow this choice; existing and imported references do not change.",
                );

                ui.add_space(12.0);
                settings_group_label(ui, "Automatic discovery and recovery");
                let discovery_result =
                    automatic_display_discovery_result(app.config.automatic_monitor_rescans);
                settings_conditional_toggle_row(
                    ui,
                    &mut app.config.automatic_monitor_rescans,
                    "Find reference display automatically",
                    discovery_result,
                );
                let recovery_result = automatic_screen_recovery_result(
                    app.config.automatic_screen_capture_recovery,
                );
                settings_conditional_toggle_row(
                    ui,
                    &mut app.config.automatic_screen_capture_recovery,
                    "Recover black screen capture automatically",
                    recovery_result,
                );
            },
        );
    }

    fn matching_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot();
        self.settings_preview_control_row(
            ui,
            SettingsSection {
                icon: UiIcon::Target,
                title: "Reference matching",
                description:
                    "Reference matches → Camera. Reference changes → Display. Without a usable reference, Automatic mode stays on Camera.",
            },
            SettingsPreview {
                kind: PreviewKind::Reference,
                frame: snapshot.previews.reference.as_ref(),
                label: "Reference image",
                empty_message: "No reference image — capture the current screen or import one.",
                actual_output: snapshot.actual_output,
            },
            |app, ui| {
                ui.horizontal_wrapped(|ui| {
                    settings_reference_status(ui, snapshot.previews.reference.is_some());
                    ui.add_space(8.0);
                    settings_detection_status(ui, snapshot.detection);
                });
                ui.add_space(7.0);
                ui.add(
                    egui::Label::new(
                        RichText::new("Checks 4×/s · 5 matches or 3 mismatches · 0.5s fade")
                        .size(10.0)
                        .color(Color32::from_rgb(126, 134, 148)),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let geometry = reference_control_geometry(
                        ui.available_width(),
                        ui.spacing().item_spacing.x,
                    );
                    if icon_button(
                        ui,
                        UiIcon::Capture,
                        "Capture screen",
                        egui::vec2(geometry.action_width, 32.0),
                        false,
                        false,
                    )
                    .clicked()
                    {
                        app.send(Command::CaptureReference);
                    }
                    if icon_button(
                        ui,
                        UiIcon::Image,
                        "Import image…",
                        egui::vec2(geometry.action_width, 32.0),
                        false,
                        false,
                    )
                    .clicked()
                    {
                        app.import_reference_dialog();
                    }
                });
                ui.add_space(12.0);
                let strictness = format!("{:.0}%", app.config.similarity_threshold * 100.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Match strictness")
                            .size(12.0)
                            .color(Color32::from_rgb(224, 228, 235)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Reset 98%").clicked() {
                            app.config.similarity_threshold = 0.98;
                        }
                        ui.label(RichText::new(strictness).monospace());
                    });
                });
                let geometry = reference_control_geometry(
                    ui.available_width(),
                    ui.spacing().item_spacing.x,
                );
                match_strictness_slider(
                    ui,
                    &mut app.config.similarity_threshold,
                    geometry.slider_width,
                );
                let explanation =
                    match_strictness_explanation(app.config.similarity_threshold);
                ui.add_space(3.0);
                settings_result_text(
                    ui,
                    &format!("{} — {}", explanation.level, explanation.effect),
                );
            },
        );
    }

    fn diagnostics_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot();
        settings_section_heading(
            ui,
            UiIcon::Check,
            "Component health",
            "Current pipeline state.",
        );
        settings_device_status(ui, UiIcon::Camera, "Webcam", snapshot.webcam_state);
        settings_device_status(ui, UiIcon::Monitor, "Screen capture", snapshot.screen_state);
        settings_device_status(
            ui,
            UiIcon::Broadcast,
            "Virtual camera",
            snapshot.virtual_camera_state,
        );
        settings_detection_status(ui, snapshot.detection);
        ui.add_space(6.0);
        settings_result_text(
            ui,
            component_health_guidance(
                snapshot.webcam_state,
                snapshot.screen_state,
                snapshot.virtual_camera_state,
                snapshot.detection,
            ),
        );

        settings_section_gap(ui);
        settings_section_heading(
            ui,
            UiIcon::Wrench,
            "Recovery",
            "Rescan finds the reference display. Restart buttons reconnect only the named component.",
        );
        ui.horizontal_wrapped(|ui| {
            if icon_button(
                ui,
                UiIcon::Refresh,
                "Rescan displays",
                egui::vec2(142.0, 34.0),
                false,
                false,
            )
            .clicked()
            {
                self.send(Command::Rescan);
            }
            for (icon, label, width, target) in SETTINGS_RECOVERY_TARGETS {
                if icon_button(ui, icon, label, egui::vec2(width, 34.0), false, false).clicked() {
                    self.send(Command::Restart(target));
                }
            }
        });
        settings_section_gap(ui);
        settings_section_heading(
            ui,
            UiIcon::Info,
            "Technical details",
            "Identifiers, formats, and timing used by the active pipeline.",
        );
        settings_info_row(
            ui,
            "Webcam device ID",
            if self.config.selected_video_device_id.is_empty() {
                "No camera selected"
            } else {
                &self.config.selected_video_device_id
            },
        );
        let display_details = snapshot.selected_monitor.as_ref().map_or_else(
            || "No display selected".to_owned(),
            |monitor| {
                format!(
                    "{} · {}×{} at ({}, {}) · {}",
                    monitor.label,
                    monitor.width,
                    monitor.height,
                    monitor.x,
                    monitor.y,
                    monitor.display_name
                )
            },
        );
        settings_info_row(ui, "Display", &display_details);
        settings_info_row(
            ui,
            "Webcam format",
            "RGB32 1280×720 at 30 fps · NV12 720p fallback",
        );
        settings_info_row(
            ui,
            "Output pipeline",
            "CPU BGRA 1280×720 at 30 fps · aspect fit · black letterboxing",
        );
        settings_info_row(
            ui,
            "Transitions",
            "Reversible 500 ms fade using the live screen frame",
        );
        settings_info_row(
            ui,
            "Detection timing",
            "Every 250 ms · 5 matches · 3 mismatches · full scan every 30 seconds",
        );

        settings_section_gap(ui);
        settings_section_heading(
            ui,
            UiIcon::Folder,
            "Storage and logs",
            "Settings, references, and 14-day logs stay on this computer.",
        );
        settings_info_row(
            ui,
            "Configuration",
            &self.store.config_path().display().to_string(),
        );
        settings_info_row(ui, "Reference image", &self.config.reference_image_path);
        settings_info_row(
            ui,
            "Log directory",
            &self.log.directory().display().to_string(),
        );
        settings_action_row(
            ui,
            "Diagnostic logs",
            "Logs are retained for 14 days.",
            |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open folder").clicked() {
                        self.open_log_directory();
                    }
                    if ui.button("Export…").clicked() {
                        self.export_logs();
                    }
                    if ui.button("Clear…").clicked() {
                        self.request_log_clear();
                    }
                });
            },
        );
    }

    fn settings_preview_control_row(
        &mut self,
        ui: &mut egui::Ui,
        section: SettingsSection,
        preview: SettingsPreview<'_>,
        add_controls: impl FnOnce(&mut Self, &mut egui::Ui),
    ) -> SettingsPreviewControls {
        let available = ui.available_width();
        let side_by_side = available >= SETTINGS_PREVIEW_COLUMNS_BREAKPOINT;
        let mut heading_rect = Rect::NOTHING;
        let mut preview_rect = Rect::NOTHING;
        let mut controls_rect = Rect::NOTHING;
        if side_by_side {
            let gap = 22.0;
            let preview_width = ((available - gap) * 0.56).min(SETTINGS_PREVIEW_WIDTH);
            let controls_width = (available - gap - preview_width).max(220.0);
            let row_height = preview_width / WINDOW_ASPECT_RATIO + 26.0;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.horizontal_top(|ui| {
                    preview_rect = self.settings_preview_panel(ui, preview, preview_width);
                    let controls = ui.allocate_ui_with_layout(
                        egui::vec2(controls_width, row_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(controls_width);
                            heading_rect = settings_section_heading(
                                ui,
                                section.icon,
                                section.title,
                                section.description,
                            );
                            ui.add_space(10.0);
                            add_controls(self, ui);
                        },
                    );
                    controls_rect = controls.response.rect;
                });
            });
        } else {
            heading_rect =
                settings_section_heading(ui, section.icon, section.title, section.description);
            ui.add_space(8.0);
            preview_rect = self.settings_single_preview(ui, preview);
            ui.add_space(10.0);
            let controls = ui.allocate_ui_with_layout(
                egui::vec2(available, 1.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(available);
                    add_controls(self, ui);
                },
            );
            controls_rect = controls.response.rect;
        }
        let layout = SettingsPreviewControls {
            heading: heading_rect,
            preview: preview_rect,
            controls: controls_rect,
            side_by_side,
        };
        debug_assert!(layout.heading.is_positive());
        debug_assert!(layout.preview.is_positive());
        debug_assert!(layout.controls.is_positive());
        debug_assert_eq!(
            layout.side_by_side,
            available >= SETTINGS_PREVIEW_COLUMNS_BREAKPOINT
        );
        layout
    }

    fn settings_single_preview(&mut self, ui: &mut egui::Ui, preview: SettingsPreview<'_>) -> Rect {
        let width = ui.available_width().min(SETTINGS_PREVIEW_WIDTH);
        let mut preview_rect = Rect::NOTHING;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), width / WINDOW_ASPECT_RATIO + 26.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                preview_rect = self.settings_preview_panel(ui, preview, width);
            },
        );
        preview_rect
    }

    fn settings_preview_panel(
        &mut self,
        ui: &mut egui::Ui,
        preview: SettingsPreview<'_>,
        width: f32,
    ) -> Rect {
        let height = (width / WINDOW_ASPECT_RATIO).min(SETTINGS_PREVIEW_HEIGHT);
        ui.allocate_ui_with_layout(
            egui::vec2(width, height + 26.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                let rect = self.preview(
                    ui,
                    preview.kind,
                    preview.frame,
                    [width, height],
                    preview.actual_output,
                    PreviewOptions::settings(preview.empty_message),
                );
                ui.add_space(7.0);
                icon_text(
                    ui,
                    preview.kind.icon(),
                    preview.label,
                    Color32::from_rgb(181, 188, 200),
                    false,
                );
                rect
            },
        )
        .inner
    }

    fn dialog(&mut self, context: &egui::Context) {
        let Some(active) = self.active_dialog else {
            return;
        };
        let progress = animation_progress(Some(active.opened_at), DIALOG_ENTRANCE_DURATION);
        if progress < 1.0 {
            context.request_repaint();
        }
        let available_width = context.content_rect().width();
        let width = active
            .kind
            .preferred_width()
            .min((available_width - 32.0).max(280.0));
        let frame = egui::Frame::new()
            .fill(Color32::from_rgb(24, 27, 33))
            .stroke(Stroke::new(1.0, Color32::from_rgb(57, 63, 75)))
            .corner_radius(12)
            .inner_margin(24)
            .shadow(egui::Shadow {
                offset: [0, 8],
                blur: 28,
                spread: 2,
                color: Color32::from_black_alpha(150),
            });
        let area = egui::Modal::default_area(egui::Id::new(("stageswap-dialog", active.kind)))
            .anchor(
                Align2::CENTER_CENTER,
                egui::vec2(0.0, 8.0 * (1.0 - progress)),
            );
        let status = self.admin_profile_status;
        let response = egui::Modal::new(egui::Id::new(("stageswap-dialog", active.kind)))
            .area(area)
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha((145.0 * progress) as u8))
            .show(context, |ui| {
                ui.set_width(width);
                ui.set_opacity(0.72 + progress * 0.28);
                dialog_content(ui, active.kind, status, active.focus_safe_action)
            });
        if let Some(dialog) = self
            .active_dialog
            .as_mut()
            .filter(|dialog| dialog.opened_at == active.opened_at)
        {
            dialog.focus_safe_action = false;
        }

        let action = response
            .inner
            .or_else(|| response.should_close().then_some(DialogAction::Dismiss));
        match action {
            None => {}
            Some(DialogAction::Dismiss) => self.dismiss_dialog(),
            Some(DialogAction::Exit) => {
                self.exit_requested = true;
                self.dismiss_dialog();
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(DialogAction::ClearLogs) => self.confirm_log_clear(),
            Some(DialogAction::SaveAdminBaseline) => self.save_admin_baseline(),
            Some(DialogAction::ReplaceAdminBaseline) => {
                self.open_dialog(AppDialogKind::ReplaceAdminBaseline);
            }
            Some(DialogAction::LoadAdminConfig) => {
                if active.kind == AppDialogKind::LoadAdminConfig {
                    self.load_admin_config();
                } else {
                    self.open_dialog(AppDialogKind::LoadAdminConfig);
                }
            }
            Some(DialogAction::RemoveAdminBaseline) => {
                if active.kind == AppDialogKind::RemoveAdminBaseline {
                    self.remove_admin_baseline();
                } else {
                    self.open_dialog(AppDialogKind::RemoveAdminBaseline);
                }
            }
            Some(DialogAction::SetAdminAutoRestore(enabled)) => {
                self.set_admin_auto_restore(enabled);
            }
        }
    }

    fn preview_cell(
        &mut self,
        ui: &mut egui::Ui,
        kind: PreviewKind,
        frame: Option<&Arc<Frame>>,
        size: [f32; 2],
        actual_output: Source,
        fps: Option<u32>,
    ) {
        self.preview(
            ui,
            kind,
            frame,
            size,
            actual_output,
            PreviewOptions::dashboard(kind, fps),
        );
        ui.add_space(8.0);
        preview_caption(ui, kind);
        ui.add_space(16.0);
    }

    fn preview(
        &mut self,
        ui: &mut egui::Ui,
        kind: PreviewKind,
        frame: Option<&Arc<Frame>>,
        maximum: [f32; 2],
        actual_output: Source,
        options: PreviewOptions,
    ) -> Rect {
        let available = Vec2::new(maximum[0].min(ui.available_width()), maximum[1]);
        let contour = preview_contour(kind, actual_output);
        let disco_enabled = self.snapshot().disco_enabled;
        let disco_elapsed = Instant::now().saturating_duration_since(self.ui_animation_started_at);
        let active_amount = ui.ctx().animate_bool_with_time(
            ui.make_persistent_id((kind.key(), "active-contour")),
            contour == PreviewContour::Active,
            0.14,
        );
        let disco_offset = match kind {
            PreviewKind::Webcam => 0.0,
            PreviewKind::Screen => 0.22,
            PreviewKind::Reference => 0.46,
            PreviewKind::Output => 0.7,
        };
        let contour_color = if disco_enabled {
            disco_ui_color(disco_elapsed, disco_offset)
        } else {
            match contour {
                PreviewContour::Live => LIVE_RED,
                PreviewContour::Active | PreviewContour::Neutral => {
                    mix_color(PREVIEW_NEUTRAL, ACTIVE_GREEN, active_amount)
                }
            }
        };
        let contour_width = if disco_enabled { 4.0 } else { 3.0 };
        let frame_fill = if disco_enabled {
            mix_color(Color32::from_rgb(12, 14, 18), contour_color, 0.14)
        } else {
            Color32::from_rgb(12, 14, 18)
        };
        let preview_frame = egui::Frame::new()
            .fill(frame_fill)
            .stroke(Stroke::new(contour_width, contour_color))
            .corner_radius(8)
            .inner_margin(3);
        let frame_margin = preview_frame.total_margin();
        let inner_size = (available
            - egui::vec2(
                frame_margin.left + frame_margin.right,
                frame_margin.top + frame_margin.bottom,
            ))
        .max(egui::vec2(1.0, 1.0));
        let preview = preview_frame.show(ui, |ui| {
            ui.allocate_ui_with_layout(
                inner_size,
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    if let Some(frame) = frame {
                        let texture_size = preview_texture_size(
                            frame.as_ref(),
                            inner_size,
                            ui.ctx().pixels_per_point(),
                        );
                        let prepared = {
                            let converter = self
                                .preview_converters
                                .entry(kind)
                                .or_insert_with(|| PreviewConverter::new(kind));
                            converter.submit(Arc::clone(frame), texture_size);
                            converter.take_ready()
                        };
                        if let Some(prepared) = prepared {
                            if let Some(texture) = self.textures.get_mut(kind.key()) {
                                if !Arc::ptr_eq(&texture.source, &prepared.frame)
                                    || texture.size != prepared.size
                                {
                                    texture.texture.set(prepared.image, TextureOptions::LINEAR);
                                    texture.source = prepared.frame;
                                    texture.size = prepared.size;
                                }
                            } else {
                                let texture = ui.ctx().load_texture(
                                    kind.key(),
                                    prepared.image,
                                    TextureOptions::LINEAR,
                                );
                                self.textures.insert(
                                    kind.key(),
                                    PreviewTexture {
                                        source: prepared.frame,
                                        size: prepared.size,
                                        texture,
                                    },
                                );
                            }
                        }
                        if let Some(texture) = self.textures.get(kind.key()) {
                            ui.add(
                                egui::Image::new((texture.texture.id(), inner_size))
                                    .maintain_aspect_ratio(true),
                            );
                        } else {
                            ui.label(
                                RichText::new("Preparing preview…")
                                    .size(12.0)
                                    .color(Color32::from_rgb(112, 120, 134)),
                            );
                        }
                    } else {
                        ui.label(
                            RichText::new(options.empty_message)
                                .size(12.0)
                                .color(Color32::from_rgb(112, 120, 134)),
                        );
                    }
                },
            );
        });
        if options.show_fps {
            paint_fps_overlay(ui, preview.response.rect, options.fps);
        }
        preview.response.rect
    }
}

impl AppDialogKind {
    const fn preferred_width(self) -> f32 {
        match self {
            Self::Admin => 440.0,
            Self::Exit
            | Self::ClearLogs
            | Self::ReplaceAdminBaseline
            | Self::LoadAdminConfig
            | Self::RemoveAdminBaseline => 400.0,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Exit => "Exit StageSwap?",
            Self::ClearLogs => "Clear diagnostic logs?",
            Self::Admin => "Admin configuration",
            Self::ReplaceAdminBaseline => "Replace saved configuration?",
            Self::LoadAdminConfig => "Load saved configuration?",
            Self::RemoveAdminBaseline => "Delete saved configuration?",
        }
    }

    const fn icon(self) -> UiIcon {
        match self {
            Self::Exit => UiIcon::Stop,
            Self::ClearLogs => UiIcon::Folder,
            Self::Admin => UiIcon::Wrench,
            Self::ReplaceAdminBaseline => UiIcon::Refresh,
            Self::LoadAdminConfig => UiIcon::Refresh,
            Self::RemoveAdminBaseline => UiIcon::Error,
        }
    }

    const fn accent(self) -> Color32 {
        match self {
            Self::Admin | Self::ReplaceAdminBaseline | Self::LoadAdminConfig => SETTINGS_BLUE,
            Self::Exit | Self::ClearLogs | Self::RemoveAdminBaseline => {
                Color32::from_rgb(222, 90, 98)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum DialogButtonTone {
    Secondary,
    Primary,
    Danger,
}

fn dialog_content(
    ui: &mut egui::Ui,
    kind: AppDialogKind,
    admin_status: Option<AdminProfileStatus>,
    focus_safe_action: bool,
) -> Option<DialogAction> {
    dialog_header(ui, kind);
    ui.add_space(18.0);
    if kind == AppDialogKind::Admin {
        return admin_dialog_content(ui, admin_status, focus_safe_action);
    }

    let body = match kind {
        AppDialogKind::Exit => {
            "StageSwap will stop publishing. The virtual camera stays installed and shows the StageSwap off screen until the app starts again."
        }
        AppDialogKind::ClearLogs => {
            "This permanently removes locally stored diagnostic logs. New logs will continue to be recorded."
        }
        AppDialogKind::ReplaceAdminBaseline => {
            "Replace the saved admin config with the setup currently shown in Settings?"
        }
        AppDialogKind::LoadAdminConfig => {
            "Replace the current settings and reference image with the saved admin config? Current session changes will be lost."
        }
        AppDialogKind::RemoveAdminBaseline => {
            "Auto-restore will turn off. Your current settings and reference image will stay unchanged."
        }
        AppDialogKind::Admin => unreachable!(),
    };
    ui.label(
        RichText::new(body)
            .size(14.0)
            .line_height(Some(21.0))
            .color(Color32::from_rgb(184, 191, 203)),
    );
    ui.add_space(24.0);

    let (safe_icon, safe_label, primary_icon, primary_label, primary_action, primary_tone) =
        match kind {
            AppDialogKind::Exit => (
                UiIcon::Window,
                "Stay open",
                UiIcon::Stop,
                "Exit StageSwap",
                DialogAction::Exit,
                DialogButtonTone::Danger,
            ),
            AppDialogKind::ClearLogs => (
                UiIcon::Folder,
                "Keep logs",
                UiIcon::Error,
                "Clear logs",
                DialogAction::ClearLogs,
                DialogButtonTone::Danger,
            ),
            AppDialogKind::ReplaceAdminBaseline => (
                UiIcon::Check,
                "Keep saved configuration",
                UiIcon::Refresh,
                "Save current configuration",
                DialogAction::SaveAdminBaseline,
                DialogButtonTone::Primary,
            ),
            AppDialogKind::LoadAdminConfig => (
                UiIcon::Check,
                "Keep current config",
                UiIcon::Refresh,
                "Load saved configuration",
                DialogAction::LoadAdminConfig,
                DialogButtonTone::Primary,
            ),
            AppDialogKind::RemoveAdminBaseline => (
                UiIcon::Check,
                "Keep saved configuration",
                UiIcon::Error,
                "Delete saved configuration",
                DialogAction::RemoveAdminBaseline,
                DialogButtonTone::Danger,
            ),
            AppDialogKind::Admin => unreachable!(),
        };
    let mut action = None;
    let button_width = dialog_action_width(ui.available_width(), ui.spacing().item_spacing.x, 2);
    ui.horizontal(|ui| {
        let safe = dialog_button(
            ui,
            safe_icon,
            safe_label,
            DialogButtonTone::Secondary,
            button_width,
        );
        if focus_safe_action {
            safe.request_focus();
        }
        if safe.clicked() {
            action = Some(DialogAction::Dismiss);
        }
        if dialog_button(ui, primary_icon, primary_label, primary_tone, button_width).clicked() {
            action = Some(primary_action);
        }
    });
    action
}

fn dialog_header(ui: &mut egui::Ui, kind: AppDialogKind) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), Sense::hover());
        let accent = kind.accent();
        ui.painter().circle_filled(
            rect.center(),
            18.0,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 38),
        );
        draw_icon(ui.painter(), rect.shrink(9.0), kind.icon(), accent, 1.7);
        ui.add_space(4.0);
        ui.label(
            RichText::new(kind.title())
                .size(19.0)
                .strong()
                .color(Color32::from_rgb(235, 238, 244)),
        );
    });
}

fn admin_dialog_content(
    ui: &mut egui::Ui,
    status: Option<AdminProfileStatus>,
    focus_safe_action: bool,
) -> Option<DialogAction> {
    ui.label(
        RichText::new(
            "Keep a protected local copy of the current settings and reference image for managed setups.",
        )
        .size(14.0)
        .line_height(Some(21.0))
        .color(Color32::from_rgb(184, 191, 203)),
    );
    ui.add_space(20.0);
    match status {
        None => {
            ui.label(
                RichText::new("No admin config is saved.")
                    .size(13.0)
                    .color(Color32::from_rgb(137, 146, 160)),
            );
            ui.add_space(22.0);
            let mut action = None;
            let button_width = ui.available_width();
            if dialog_button(
                ui,
                UiIcon::Check,
                "Save current configuration",
                DialogButtonTone::Primary,
                button_width,
            )
            .clicked()
            {
                action = Some(DialogAction::SaveAdminBaseline);
            }
            let back = dialog_button(
                ui,
                UiIcon::Back,
                "Back",
                DialogButtonTone::Secondary,
                button_width,
            );
            if focus_safe_action {
                back.request_focus();
            }
            if back.clicked() {
                action = Some(DialogAction::Dismiss);
            }
            action
        }
        Some(status) => {
            let reference = if status.reference_included {
                "Settings and reference image saved in the admin config"
            } else {
                "Admin config saved without a reference image"
            };
            icon_text(
                ui,
                UiIcon::Check,
                reference,
                Color32::from_rgb(116, 205, 157),
                false,
            );
            ui.add_space(14.0);
            let mut auto_restore = status.auto_restore_on_launch;
            settings_toggle_row_without_separator(
                ui,
                &mut auto_restore,
                "Auto-restore on launch",
                "Replace session changes with this admin config whenever StageSwap starts.",
            );
            if auto_restore != status.auto_restore_on_launch {
                return Some(DialogAction::SetAdminAutoRestore(auto_restore));
            }
            ui.add_space(18.0);
            let mut action = None;
            let button_width = ui.available_width();
            if dialog_button(
                ui,
                UiIcon::Check,
                "Save current configuration",
                DialogButtonTone::Primary,
                button_width,
            )
            .clicked()
            {
                action = Some(DialogAction::ReplaceAdminBaseline);
            }
            if dialog_button(
                ui,
                UiIcon::Refresh,
                "Load saved configuration",
                DialogButtonTone::Primary,
                button_width,
            )
            .clicked()
            {
                action = Some(DialogAction::LoadAdminConfig);
            }
            if dialog_button(
                ui,
                UiIcon::Error,
                "Delete saved configuration",
                DialogButtonTone::Danger,
                button_width,
            )
            .clicked()
            {
                action = Some(DialogAction::RemoveAdminBaseline);
            }
            let back = dialog_button(
                ui,
                UiIcon::Back,
                "Back",
                DialogButtonTone::Secondary,
                button_width,
            );
            if focus_safe_action {
                back.request_focus();
            }
            if back.clicked() {
                action = Some(DialogAction::Dismiss);
            }
            action
        }
    }
}

fn dialog_button(
    ui: &mut egui::Ui,
    icon: UiIcon,
    label: &str,
    tone: DialogButtonTone,
    width: f32,
) -> egui::Response {
    let (base_fill, base_stroke, text_color) = match tone {
        DialogButtonTone::Secondary => (
            Color32::from_rgb(40, 44, 53),
            Stroke::new(1.0, Color32::from_rgb(66, 73, 87)),
            Color32::from_rgb(224, 228, 235),
        ),
        DialogButtonTone::Primary => (SETTINGS_BLUE, Stroke::NONE, Color32::WHITE),
        DialogButtonTone::Danger => (Color32::from_rgb(174, 58, 69), Stroke::NONE, Color32::WHITE),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 38.0), Sense::click());
    let interaction = if response.is_pointer_button_down_on() {
        0.14
    } else if response.hovered() {
        0.08
    } else {
        0.0
    };
    let fill = mix_color(base_fill, Color32::WHITE, interaction);
    let stroke = if response.has_focus() {
        Stroke::new(1.5, Color32::from_rgb(225, 232, 245))
    } else {
        base_stroke
    };
    ui.painter().rect_filled(rect, 7, fill);
    ui.painter()
        .rect_stroke(rect, 7, stroke, StrokeKind::Inside);

    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), FontId::proportional(14.0), text_color);
    let icon_size = 15.0;
    let gap = 7.0;
    let content_width = icon_size + gap + galley.size().x;
    let icon_rect = Rect::from_min_size(
        Pos2::new(
            rect.center().x - content_width / 2.0,
            rect.center().y - icon_size / 2.0,
        ),
        egui::vec2(icon_size, icon_size),
    );
    draw_icon(ui.painter(), icon_rect, icon, text_color, 1.45);
    ui.painter().galley(
        Pos2::new(
            icon_rect.right() + gap,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        text_color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn dialog_action_width(available_width: f32, gap: f32, action_count: usize) -> f32 {
    ((available_width - gap * (action_count.saturating_sub(1) as f32)) / action_count as f32)
        .max(1.0)
}

impl eframe::App for SwitcherApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        if let Some(readiness) = self.instance_readiness.take() {
            readiness.mark_ready();
        }
        #[cfg(windows)]
        if let Some(commands) = self.instance_commands.as_ref() {
            let commands: Vec<_> = commands.try_iter().collect();
            for command in commands {
                match command {
                    stageswap_windows::InstanceCommand::Show => {
                        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        context.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    stageswap_windows::InstanceCommand::ShutdownForReplacement => {
                        let _ = save_config(&self.store, &self.config);
                        self.exit_requested = true;
                        self.dismiss_dialog();
                        context.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        let snapshot = self.snapshot();
        if self
            .awaiting_video_device_id
            .as_ref()
            .is_some_and(|expected| *expected == snapshot.selected_video_device_id)
        {
            self.awaiting_video_device_id = None;
        } else if self.awaiting_video_device_id.is_none()
            && !snapshot.selected_video_device_id.is_empty()
            && snapshot.selected_video_device_id != self.config.selected_video_device_id
        {
            self.config.selected_video_device_id = snapshot.selected_video_device_id.clone();
            if let Err(error) = save_config(&self.store, &self.config) {
                self.load_warnings.push(format!(
                    "Could not save automatic webcam selection: {error}"
                ));
            }
        }
        if self
            .awaiting_monitor_label
            .as_ref()
            .is_some_and(|expected| {
                snapshot
                    .selected_monitor
                    .as_ref()
                    .is_some_and(|monitor| monitor.label == *expected)
            })
        {
            self.awaiting_monitor_label = None;
        } else if self.awaiting_monitor_label.is_none() {
            self.sync_selected_monitor_preference(&snapshot);
        }
        if self.settings_save_due(Instant::now()) {
            self.flush_settings();
        }
        let first_unlogged = self
            .last_activity
            .as_ref()
            .and_then(|last| {
                snapshot
                    .recent_activity
                    .iter()
                    .rposition(|activity| activity == last)
            })
            .map_or(0, |index| index + 1);
        for activity in snapshot.recent_activity.iter().skip(first_unlogged) {
            self.log
                .write("info", "runtime", "ACTIVITY", activity.as_str());
        }
        if let Some(activity) = snapshot.recent_activity.last() {
            self.last_activity = Some(activity.clone());
        }
        #[cfg(windows)]
        if self.config.show_notifications
            && let Some(warning) = snapshot.warning.as_ref()
            && self.last_notified_warning.as_ref() != Some(warning)
        {
            if let Err(error) = stageswap_windows::notify_warning(warning) {
                self.log.write(
                    "warning",
                    "notification",
                    "WINDOWS_NOTIFICATION_FAILED",
                    &error,
                );
            }
            self.last_notified_warning = Some(warning.clone());
        }
        #[cfg(windows)]
        if snapshot.warning.is_none() {
            self.last_notified_warning = None;
        }
        #[cfg(windows)]
        if let Some(tray) = self.tray.as_ref() {
            tray.sync(&snapshot);
        }
        #[cfg(windows)]
        if let Some(action) = self.tray.as_ref().and_then(tray::Tray::poll) {
            match action {
                tray::TrayAction::Show => {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayAction::OpenSettings => {
                    self.open_settings();
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayAction::ToggleAutomation => {
                    if matches!(snapshot.run_state, RunState::Running | RunState::Starting) {
                        self.send(Command::Stop);
                    } else {
                        self.send(Command::Start);
                    }
                }
                tray::TrayAction::SetMode(mode) => self.set_mode(mode),
                tray::TrayAction::Exit => {
                    if self.config.confirm_exit {
                        self.open_dialog(AppDialogKind::Exit);
                        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        context.send_viewport_cmd(egui::ViewportCommand::Focus);
                    } else {
                        self.exit_requested = true;
                        context.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        let (window_size, maximized, fullscreen) = context.input(|input| {
            let viewport = input.viewport();
            (
                viewport.inner_rect.map(|rect| rect.size()),
                viewport.maximized.unwrap_or(false),
                viewport.fullscreen.unwrap_or(false),
            )
        });
        if let Some(window_size) = window_size {
            if !maximized && !fullscreen {
                let desired = aspect_locked_window_size(window_size, self.last_window_size);
                if (desired.x - window_size.x).abs() > 1.0
                    || (desired.y - window_size.y).abs() > 1.0
                {
                    context.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
                    self.last_window_size = Some(desired);
                } else {
                    self.last_window_size = Some(window_size);
                }
            } else {
                self.last_window_size = Some(window_size);
            }
        }
        let close_requested = context.input(|input| input.viewport().close_requested());
        if close_requested && self.config.close_to_tray && !self.exit_requested {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else if close_requested && self.config.confirm_exit && !self.exit_requested {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.dialog_is(AppDialogKind::Exit) {
                self.open_dialog(AppDialogKind::Exit);
            }
        }
        context.request_repaint_after(repaint_interval(false));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.root_ui(ui);
        ui.ctx().request_repaint_after(repaint_interval(true));
    }
}

fn repaint_interval(ui_rendered: bool) -> Duration {
    if ui_rendered {
        VISIBLE_REFRESH
    } else {
        HIDDEN_REFRESH
    }
}

impl Drop for SwitcherApp {
    fn drop(&mut self) {
        #[cfg(not(windows))]
        if self.ui_preview.is_some() {
            return;
        }
        let _ = save_config(&self.store, &self.config);
    }
}

#[cfg(windows)]
fn save_config(store: &ConfigStore, config: &AppConfig) -> std::io::Result<()> {
    stageswap_windows::save_config_atomic(store, config)
}

#[cfg(not(windows))]
fn save_config(store: &ConfigStore, config: &AppConfig) -> std::io::Result<()> {
    store.save(config)
}

#[cfg(windows)]
fn save_admin_profile(
    store: &AdminProfileStore,
    config: &AppConfig,
) -> std::io::Result<AdminProfileStatus> {
    store.save_with_replace(config, stageswap_windows::replace_file_atomic)
}

#[cfg(not(windows))]
fn save_admin_profile(
    store: &AdminProfileStore,
    config: &AppConfig,
) -> std::io::Result<AdminProfileStatus> {
    store.save(config)
}

#[cfg(windows)]
fn set_admin_auto_restore(
    store: &AdminProfileStore,
    enabled: bool,
) -> std::io::Result<AdminProfileStatus> {
    store.set_auto_restore_with_replace(enabled, stageswap_windows::replace_file_atomic)
}

#[cfg(not(windows))]
fn set_admin_auto_restore(
    store: &AdminProfileStore,
    enabled: bool,
) -> std::io::Result<AdminProfileStatus> {
    store.set_auto_restore_on_launch(enabled)
}

fn admin_logo_activated(response: &egui::Response) -> bool {
    response.double_clicked_by(egui::PointerButton::Secondary)
}

fn animation_progress(started_at: Option<Instant>, duration: Duration) -> f32 {
    started_at.map_or(1.0, |started_at| {
        (Instant::now()
            .saturating_duration_since(started_at)
            .as_secs_f32()
            / duration.as_secs_f32())
        .clamp(0.0, 1.0)
    })
}

fn controls_section_heading(ui: &mut egui::Ui, icon: UiIcon, title: &str) -> Rect {
    let heading = ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
        draw_icon(
            ui.painter(),
            icon_rect,
            icon,
            Color32::from_rgb(119, 164, 247),
            1.4,
        );
        ui.label(
            RichText::new(title)
                .size(11.0)
                .strong()
                .color(Color32::from_rgb(151, 158, 171)),
        );
    });
    ui.add_space(6.0);
    heading.response.rect
}

fn controls_section_divider(ui: &mut egui::Ui) -> Rect {
    ui.add_space(2.0);
    let divider = ui.separator().rect;
    ui.add_space(2.0);
    divider
}

fn settings_back_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    let fill = if response.hovered() {
        SETTINGS_NAV_HOVERED
    } else {
        SETTINGS_SIDEBAR_FILL
    };
    ui.painter().rect_filled(rect, 6, fill);
    let foreground = if response.hovered() {
        Color32::WHITE
    } else {
        Color32::from_rgb(161, 168, 181)
    };
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 22.0, rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    draw_icon(ui.painter(), icon_rect, UiIcon::Back, foreground, 1.45);
    ui.painter().text(
        Pos2::new(rect.left() + 42.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Back to dashboard",
        FontId::proportional(13.0),
        foreground,
    );
    response
        .on_hover_text("Back to dashboard")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn settings_nav_button(
    ui: &mut egui::Ui,
    tab: SettingsTab,
    icon: UiIcon,
    label: &str,
    selected: SettingsTab,
    disco_accent: Option<Color32>,
) -> egui::Response {
    let active = tab == selected;
    let amount =
        ui.ctx()
            .animate_bool_with_time(ui.make_persistent_id(("settings-nav", tab)), active, 0.14);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    let base = if response.hovered() {
        SETTINGS_NAV_HOVERED
    } else {
        SETTINGS_SIDEBAR_FILL
    };
    let selected_fill = disco_accent.map_or(SETTINGS_NAV_SELECTED, |accent| {
        mix_color(SETTINGS_NAV_SELECTED, accent, 0.42)
    });
    ui.painter()
        .rect_filled(rect, 6, mix_color(base, selected_fill, amount));
    if amount > 0.0 {
        let indicator_height = 16.0 + 10.0 * amount;
        let indicator = Rect::from_center_size(
            Pos2::new(rect.left() + 2.0, rect.center().y),
            egui::vec2(3.0, indicator_height),
        );
        let indicator_color = disco_accent.unwrap_or(SETTINGS_NAV_INDICATOR);
        ui.painter()
            .rect_filled(indicator, 2, indicator_color.gamma_multiply(amount));
    }
    let foreground = mix_color(Color32::from_rgb(161, 168, 181), Color32::WHITE, amount);
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 22.0, rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    draw_icon(ui.painter(), icon_rect, icon, foreground, 1.45);
    ui.painter().text(
        Pos2::new(rect.left() + 42.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        foreground,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn settings_section_heading(
    ui: &mut egui::Ui,
    icon: UiIcon,
    title: &str,
    description: &str,
) -> Rect {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(17.0, 17.0), Sense::hover());
            draw_icon(
                ui.painter(),
                rect,
                icon,
                Color32::from_rgb(119, 164, 247),
                1.45,
            );
            ui.label(
                RichText::new(title)
                    .size(14.0)
                    .strong()
                    .color(Color32::from_rgb(228, 231, 237)),
            );
        });
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                RichText::new(description)
                    .size(11.0)
                    .color(Color32::from_rgb(128, 136, 150)),
            )
            .wrap(),
        );
    })
    .response
    .rect
}

fn settings_section_gap(ui: &mut egui::Ui) {
    ui.add_space(24.0);
}

fn settings_group_label(ui: &mut egui::Ui, label: &str) {
    ui.label(
        RichText::new(label)
            .size(11.5)
            .strong()
            .color(Color32::from_rgb(190, 196, 207)),
    );
    ui.add_space(3.0);
}

fn settings_result_text(ui: &mut egui::Ui, text: &str) -> Rect {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(10.0)
                .color(Color32::from_rgb(145, 165, 199)),
        )
        .wrap(),
    )
    .rect
}

const fn output_mode_name(mode: OutputMode) -> &'static str {
    match mode {
        OutputMode::Automatic => "Automatic",
        OutputMode::ForceCamera => "Camera",
        OutputMode::ForceScreen => "Display",
    }
}

fn start_automatically_result(
    enabled: bool,
    start_minimized: bool,
    output_mode: OutputMode,
) -> String {
    if enabled {
        format!(
            "On — Starts in {} mode{}.",
            output_mode_name(output_mode),
            if start_minimized {
                " in the tray"
            } else {
                " after the dashboard opens"
            }
        )
    } else {
        "Off — Shows the StageSwap off screen until automation starts.".into()
    }
}

const fn window_behavior_result(close_to_tray: bool, confirm_exit: bool) -> &'static str {
    match (close_to_tray, confirm_exit) {
        (true, true) => "Closing hides the window; Exit from the tray asks for confirmation.",
        (true, false) => {
            "Closing hides the window; Exit from the tray stops StageSwap immediately."
        }
        (false, true) => "Closing the window or choosing Exit asks before StageSwap stops.",
        (false, false) => "Closing the window or choosing Exit stops StageSwap immediately.",
    }
}

const fn automatic_display_discovery_result(enabled: bool) -> &'static str {
    if enabled {
        "On — Searches at launch, Settings open, reference changes, and every 30 seconds; confirms the same display twice."
    } else {
        "Off — Choose a display manually or use Rescan displays."
    }
}

const fn automatic_screen_recovery_result(enabled: bool) -> &'static str {
    if enabled {
        "On — Checks the selected display every 30 seconds and restarts after two black results. Black content can trigger recovery."
    } else {
        "Off — Use Restart screen capture in Diagnostics."
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MatchStrictnessExplanation {
    level: &'static str,
    effect: &'static str,
}

fn match_strictness_explanation(value: f64) -> MatchStrictnessExplanation {
    let percentage = (value * 100.0).round();
    if percentage >= 99.0 {
        MatchStrictnessExplanation {
            level: "Very strict",
            effect: "Small visual changes can switch Automatic mode to Display.",
        }
    } else if percentage >= 97.0 {
        MatchStrictnessExplanation {
            level: "Balanced",
            effect: "Minor rendering or cursor differences can still match the reference.",
        }
    } else if percentage >= 90.0 {
        MatchStrictnessExplanation {
            level: "Forgiving",
            effect: "Larger differences may still count as the reference.",
        }
    } else {
        MatchStrictnessExplanation {
            level: "Very forgiving",
            effect: "Meaningful changes may still be treated as a match.",
        }
    }
}

const fn component_health_guidance(
    webcam: DeviceState,
    screen: DeviceState,
    virtual_camera: DeviceState,
    detection: DetectionState,
) -> &'static str {
    if matches!(webcam, DeviceState::Failed | DeviceState::Unavailable) {
        "The webcam needs attention. Choose or refresh the camera in Webcam, then restart it here if needed."
    } else if matches!(screen, DeviceState::Failed | DeviceState::Unavailable) {
        "Screen capture needs attention. Choose a display in Screen, then restart capture here if needed."
    } else if matches!(
        virtual_camera,
        DeviceState::Failed | DeviceState::Unavailable
    ) {
        "The virtual camera needs attention. Restart it here, then reselect StageSwap in the meeting app if necessary."
    } else if matches!(webcam, DeviceState::Initializing)
        || matches!(screen, DeviceState::Initializing)
        || matches!(virtual_camera, DeviceState::Initializing)
    {
        "One or more components are still starting. Wait briefly before using a recovery action."
    } else {
        match detection {
            DetectionState::ReferenceMissing => {
                "The pipeline is ready, but Automatic mode needs a captured or imported reference."
            }
            DetectionState::Unknown => {
                "The components are ready; StageSwap is waiting for enough reference checks to decide."
            }
            DetectionState::Matching => "Everything is ready.",
            DetectionState::NotMatching => "Everything is ready.",
        }
    }
}

fn settings_toggle_row(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    description: &str,
) -> SettingsToggleLayout {
    settings_toggle_row_with_result(ui, value, title, Some(description), None, true)
}

fn settings_toggle_row_without_separator(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    description: &str,
) -> SettingsToggleLayout {
    settings_toggle_row_with_result(ui, value, title, Some(description), None, false)
}

fn settings_conditional_toggle_row(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    result: &str,
) -> SettingsToggleLayout {
    settings_toggle_row_with_result(ui, value, title, None, Some(result), true)
}

fn settings_toggle_row_with_result(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    description: Option<&str>,
    result: Option<&str>,
    separator: bool,
) -> SettingsToggleLayout {
    const TEXT_CONTROL_GAP: f32 = 12.0;
    const CONTROL_WIDTH: f32 = 83.0;
    const VERTICAL_PADDING: f32 = 7.0;
    const TEXT_GAP: f32 = 3.0;
    const RESULT_GAP: f32 = 4.0;
    let width = ui.available_width();
    let text_width = (width - CONTROL_WIDTH - TEXT_CONTROL_GAP).max(80.0);
    let title_color = Color32::from_rgb(224, 228, 235);
    let description_color = Color32::from_rgb(126, 134, 148);
    let result_color = Color32::from_rgb(145, 165, 199);
    let title_galley =
        ui.painter()
            .layout_no_wrap(title.to_owned(), FontId::proportional(12.5), title_color);
    let description_galley = description.map(|description| {
        ui.painter().layout(
            description.to_owned(),
            FontId::proportional(10.0),
            description_color,
            text_width,
        )
    });
    let result_galley = result.map(|result| {
        ui.painter().layout(
            result.to_owned(),
            FontId::proportional(10.0),
            result_color,
            text_width,
        )
    });
    let title_height = title_galley.size().y;
    let description_height = description_galley
        .as_ref()
        .map_or(0.0, |galley| TEXT_GAP + galley.size().y);
    let result_height = result_galley.as_ref().map_or(0.0, |galley| {
        if description_galley.is_some() {
            RESULT_GAP + galley.size().y
        } else {
            TEXT_GAP + galley.size().y
        }
    });
    let text_height = title_height + description_height + result_height;
    let row_height = (text_height + VERTICAL_PADDING * 2.0).max(52.0);
    let (row, response) = ui.allocate_exact_size(egui::vec2(width, row_height), Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    let amount = ui.ctx().animate_bool_with_time(
        ui.make_persistent_id(("settings-switch", title)),
        *value,
        0.12,
    );
    if amount > 0.0 && amount < 1.0 {
        ui.ctx().request_repaint();
    }
    let geometry = settings_switch_geometry(row);
    let text_top = row.center().y - text_height / 2.0;
    let title_position = Pos2::new(row.left(), text_top);
    ui.painter()
        .galley(title_position, title_galley, title_color);
    let description_rect = description_galley.map(|galley| {
        let position = Pos2::new(row.left(), text_top + title_height + TEXT_GAP);
        let rect = Rect::from_min_size(position, galley.size());
        ui.painter().galley(position, galley, description_color);
        rect
    });
    let result_rect = result_galley.map(|galley| {
        let result_position = Pos2::new(
            row.left(),
            text_top
                + title_height
                + if let Some(description_rect) = description_rect {
                    TEXT_GAP + description_rect.height() + RESULT_GAP
                } else {
                    TEXT_GAP
                },
        );
        let rect = Rect::from_min_size(result_position, galley.size());
        ui.painter().galley(result_position, galley, result_color);
        rect
    });
    let (off, on) = settings_switch_colors();
    ui.painter().rect_filled(
        geometry.track,
        geometry.track.height() / 2.0,
        mix_color(off, on, amount),
    );
    ui.painter().rect_stroke(
        geometry.track,
        geometry.track.height() / 2.0,
        Stroke::new(1.0, mix_color(Color32::from_rgb(76, 84, 98), on, amount)),
        StrokeKind::Inside,
    );
    let thumb_x = egui::lerp(
        (geometry.track.left() + 11.0)..=(geometry.track.right() - 11.0),
        amount,
    );
    ui.painter().circle_filled(
        Pos2::new(thumb_x, geometry.track.center().y),
        8.0,
        Color32::WHITE,
    );
    ui.painter().text(
        geometry.state.center(),
        Align2::CENTER_CENTER,
        if *value { "On" } else { "Off" },
        FontId::proportional(11.0),
        if *value {
            Color32::from_rgb(119, 164, 247)
        } else {
            Color32::from_rgb(132, 140, 153)
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, title)
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if separator {
        ui.separator();
    }
    SettingsToggleLayout {
        row,
        description: description_rect,
        result: result_rect,
        state: geometry.state,
        track: geometry.track,
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct SettingsToggleLayout {
    row: Rect,
    description: Option<Rect>,
    result: Option<Rect>,
    state: Rect,
    track: Rect,
}

#[derive(Clone, Copy, Debug)]
struct SettingsSwitchGeometry {
    state: Rect,
    track: Rect,
}

#[derive(Clone, Copy, Debug)]
struct SelectorUtilityGeometry {
    selector_width: f32,
    action_width: f32,
}

#[derive(Clone, Copy, Debug)]
struct ReferenceControlGeometry {
    action_width: f32,
    slider_width: f32,
}

fn reference_control_geometry(available: f32, gap: f32) -> ReferenceControlGeometry {
    ReferenceControlGeometry {
        action_width: ((available - gap) / 2.0).max(80.0),
        slider_width: available.max(1.0),
    }
}

fn match_strictness_slider(ui: &mut egui::Ui, value: &mut f64, width: f32) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = width.max(1.0);
        ui.add(egui::Slider::new(value, 0.50..=1.0).show_value(false))
    })
    .inner
}

fn selector_utility_geometry(available: f32, gap: f32) -> SelectorUtilityGeometry {
    let action_width = 32.0;
    SelectorUtilityGeometry {
        selector_width: (available - gap - action_width).max(80.0),
        action_width,
    }
}

fn settings_switch_geometry(row: Rect) -> SettingsSwitchGeometry {
    let track = Rect::from_center_size(
        Pos2::new(row.right() - 21.0, row.center().y),
        egui::vec2(42.0, 22.0),
    );
    let state = Rect::from_center_size(
        Pos2::new(track.left() - 24.0, row.center().y),
        egui::vec2(34.0, 22.0),
    );
    SettingsSwitchGeometry { state, track }
}

const fn settings_switch_colors() -> (Color32, Color32) {
    (SETTINGS_SWITCH_OFF, SETTINGS_BLUE)
}

fn settings_control_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let width = ui.available_width();
    let control_width = 300.0_f32.min((width * 0.46).max(120.0));
    let label_width = (width - control_width - 12.0).max(100.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, 54.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, 44.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(title)
                            .size(13.0)
                            .color(Color32::from_rgb(224, 228, 235)),
                    );
                    ui.label(
                        RichText::new(description)
                            .size(10.5)
                            .color(Color32::from_rgb(126, 134, 148)),
                    );
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(control_width, 44.0),
                egui::Layout::right_to_left(egui::Align::Center),
                add_control,
            );
        },
    );
    ui.separator();
}

fn settings_action_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_action: impl FnOnce(&mut egui::Ui),
) {
    settings_control_row(ui, title, description, add_action);
}

fn settings_info_row(ui: &mut egui::Ui, title: &str, value: &str) {
    settings_control_row(ui, title, "", |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(value)
                    .monospace()
                    .size(10.5)
                    .color(Color32::from_rgb(161, 168, 181)),
            )
            .wrap(),
        );
    });
}

fn settings_device_status(ui: &mut egui::Ui, icon: UiIcon, label: &str, state: DeviceState) {
    let (status_icon, color) = match state {
        DeviceState::Initializing => (UiIcon::Loader, TRANSITION_AMBER),
        DeviceState::Ready => (UiIcon::Check, ACTIVE_GREEN),
        DeviceState::Unavailable => (UiIcon::Unavailable, LIVE_RED),
        DeviceState::Failed => (UiIcon::Error, LIVE_RED),
    };
    settings_status_item(
        ui,
        icon,
        label,
        status_icon,
        friendly_device_state(state),
        color,
    );
}

fn settings_reference_status(ui: &mut egui::Ui, available: bool) {
    let (icon, status, color) = if available {
        (UiIcon::Check, "Ready", ACTIVE_GREEN)
    } else {
        (UiIcon::Unavailable, "Missing", LIVE_RED)
    };
    settings_status_item(ui, UiIcon::Image, "Reference", icon, status, color);
}

fn settings_detection_status(ui: &mut egui::Ui, state: DetectionState) {
    let (icon, status, color) = settings_detection_style(state);
    settings_status_item(ui, UiIcon::Target, "Detection", icon, status, color);
}

fn settings_detection_style(state: DetectionState) -> (UiIcon, &'static str, Color32) {
    match state {
        DetectionState::Unknown => (UiIcon::Question, "Waiting", TRANSITION_AMBER),
        DetectionState::Matching => (UiIcon::Check, "Matching", ACTIVE_GREEN),
        DetectionState::NotMatching => (UiIcon::Error, "Not matching", TRANSITION_AMBER),
        DetectionState::ReferenceMissing => (UiIcon::Unavailable, "Reference missing", LIVE_RED),
    }
}

fn settings_status_item(
    ui: &mut egui::Ui,
    icon: UiIcon,
    label: &str,
    status_icon: UiIcon,
    status: &str,
    color: Color32,
) {
    ui.horizontal(|ui| {
        icon_text(ui, icon, label, Color32::from_rgb(183, 190, 202), false);
        ui.add_space(3.0);
        icon_text(ui, status_icon, status, color, false);
    });
}

const HEALTH_STATES: [DeviceState; 4] = [
    DeviceState::Initializing,
    DeviceState::Ready,
    DeviceState::Unavailable,
    DeviceState::Failed,
];

fn preview_contour(kind: PreviewKind, actual_output: Source) -> PreviewContour {
    match kind {
        PreviewKind::Output => PreviewContour::Live,
        PreviewKind::Webcam if actual_output == Source::Camera => PreviewContour::Active,
        PreviewKind::Screen if actual_output == Source::Screen => PreviewContour::Active,
        PreviewKind::Webcam | PreviewKind::Screen | PreviewKind::Reference => {
            PreviewContour::Neutral
        }
    }
}

fn health_state_group(
    ui: &mut egui::Ui,
    icon: UiIcon,
    label: &'static str,
    current: DeviceState,
) -> Rect {
    let choices = HEALTH_STATES.map(|state| IndicatorChoice {
        icon: device_state_icon(state),
        label: friendly_device_state(state),
        current: state == current,
        tone: device_state_tone(state),
        span: 1,
    });
    indicator_group(ui, icon, label, &choices, None)
}

#[derive(Clone, Copy)]
struct IndicatorChoice {
    icon: UiIcon,
    label: &'static str,
    current: bool,
    tone: IndicatorTone,
    span: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IndicatorTone {
    Green,
    Amber,
    Red,
}

fn detection_state_group(ui: &mut egui::Ui, current: DetectionState) -> Rect {
    let choices = [
        IndicatorChoice {
            icon: UiIcon::Question,
            label: "Unknown",
            current: current == DetectionState::Unknown,
            tone: detection_indicator_tone(DetectionState::Unknown),
            span: 1,
        },
        IndicatorChoice {
            icon: UiIcon::Check,
            label: "Matching",
            current: current == DetectionState::Matching,
            tone: detection_indicator_tone(DetectionState::Matching),
            span: 1,
        },
        IndicatorChoice {
            icon: UiIcon::Error,
            label: "Not matching",
            current: current == DetectionState::NotMatching,
            tone: detection_indicator_tone(DetectionState::NotMatching),
            span: 1,
        },
        IndicatorChoice {
            icon: UiIcon::Unavailable,
            label: "Reference missing",
            current: current == DetectionState::ReferenceMissing,
            tone: detection_indicator_tone(DetectionState::ReferenceMissing),
            span: 1,
        },
    ];
    indicator_group(ui, UiIcon::Target, "Detection", &choices, None)
}

fn detection_indicator_tone(state: DetectionState) -> IndicatorTone {
    match state {
        DetectionState::Unknown | DetectionState::NotMatching => IndicatorTone::Amber,
        DetectionState::Matching => IndicatorTone::Green,
        DetectionState::ReferenceMissing => IndicatorTone::Red,
    }
}

fn screen_mix_group(ui: &mut egui::Ui, screen_mix: f64) -> Rect {
    let active = if screen_mix <= 0.01 {
        0
    } else if screen_mix >= 0.99 {
        2
    } else {
        1
    };
    let choices = [
        IndicatorChoice {
            icon: UiIcon::Camera,
            label: "Webcam only",
            current: active == 0,
            tone: IndicatorTone::Green,
            span: 1,
        },
        IndicatorChoice {
            icon: UiIcon::Layers,
            label: "Crossfading",
            current: active == 1,
            tone: IndicatorTone::Amber,
            span: 2,
        },
        IndicatorChoice {
            icon: UiIcon::Monitor,
            label: "Screen only",
            current: active == 2,
            tone: IndicatorTone::Green,
            span: 1,
        },
    ];
    let percentage = format!("{}%", (screen_mix * 100.0).round());
    indicator_group(
        ui,
        UiIcon::Layers,
        "Screen mix",
        &choices,
        Some((1, &percentage)),
    )
}

fn indicator_group(
    ui: &mut egui::Ui,
    icon: UiIcon,
    label: &'static str,
    choices: &[IndicatorChoice],
    chip_value: Option<(usize, &str)>,
) -> Rect {
    let gap = 4.0;
    let chip_size = 28.0;
    let slots = choices
        .iter()
        .map(|choice| usize::from(choice.span))
        .sum::<usize>();
    let chips_width = chip_size * slots as f32 + gap * (slots as f32 - 1.0);
    let heading_width = (ui.available_width() - chips_width - gap).max(64.0);
    let row = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.horizontal(|ui| {
            indicator_heading(ui, icon, label, None, heading_width, chip_size);
            for (index, choice) in choices.iter().enumerate() {
                let width =
                    chip_size * f32::from(choice.span) + gap * (f32::from(choice.span) - 1.0);
                let value = chip_value
                    .filter(|(value_index, _)| *value_index == index)
                    .map(|(_, value)| value);
                indicator_chip(ui, label, *choice, width, value);
            }
        });
    });
    ui.add_space(6.0);
    row.response.rect
}

fn indicator_heading(
    ui: &mut egui::Ui,
    icon: UiIcon,
    label: &'static str,
    value: Option<&str>,
    width: f32,
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let icon_rect = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - 7.0),
        egui::vec2(14.0, 14.0),
    );
    draw_icon(ui.painter(), icon_rect, icon, Color32::LIGHT_GRAY, 1.4);
    ui.painter().text(
        Pos2::new(icon_rect.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        Color32::LIGHT_GRAY,
    );
    if let Some(value) = value {
        ui.painter().text(
            Pos2::new(rect.right(), rect.center().y),
            Align2::RIGHT_CENTER,
            value,
            FontId::proportional(11.0),
            Color32::GRAY,
        );
    }
}

fn indicator_chip(
    ui: &mut egui::Ui,
    group: &'static str,
    choice: IndicatorChoice,
    width: f32,
    value: Option<&str>,
) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 28.0), Sense::hover());
    let amount = ui.ctx().animate_bool_with_time(
        ui.make_persistent_id((group, choice.label)),
        choice.current,
        0.12,
    );
    let (fill, stroke_color, icon_color) = indicator_palette(choice.tone, amount);
    ui.painter().rect_filled(rect, 5, fill);
    ui.painter().rect_stroke(
        rect,
        5,
        Stroke::new(1.0 + amount, stroke_color),
        StrokeKind::Inside,
    );
    if let Some(value) = value {
        let font = FontId::monospace(10.5);
        let galley = ui
            .painter()
            .layout_no_wrap(value.to_owned(), font, icon_color);
        let icon_size = 14.0;
        let gap = 5.0;
        let content_width = icon_size + gap + galley.size().x;
        let icon_rect = Rect::from_min_size(
            Pos2::new(
                rect.center().x - content_width / 2.0,
                rect.center().y - icon_size / 2.0,
            ),
            egui::vec2(icon_size, icon_size),
        );
        draw_icon(ui.painter(), icon_rect, choice.icon, icon_color, 1.5);
        ui.painter().galley(
            Pos2::new(
                icon_rect.right() + gap,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            icon_color,
        );
    } else {
        draw_icon(
            ui.painter(),
            Rect::from_center_size(rect.center(), egui::vec2(15.0, 15.0)),
            choice.icon,
            icon_color,
            1.55,
        );
    }
    response.on_hover_text(choice.label);
}

fn indicator_palette(tone: IndicatorTone, active_amount: f32) -> (Color32, Color32, Color32) {
    let inactive_fill = Color32::from_rgb(31, 34, 40);
    let inactive_stroke = Color32::from_rgb(91, 97, 108);
    let inactive_text = Color32::from_rgb(151, 158, 170);
    let (active_fill, active_stroke) = match tone {
        IndicatorTone::Green => (Color32::from_rgb(34, 81, 58), ACTIVE_GREEN),
        IndicatorTone::Amber => (Color32::from_rgb(82, 67, 33), TRANSITION_AMBER),
        IndicatorTone::Red => (Color32::from_rgb(86, 38, 42), LIVE_RED),
    };
    (
        mix_color(inactive_fill, active_fill, active_amount),
        mix_color(inactive_stroke, active_stroke, active_amount),
        mix_color(inactive_text, Color32::WHITE, active_amount),
    )
}

fn device_state_icon(state: DeviceState) -> UiIcon {
    match state {
        DeviceState::Initializing => UiIcon::Loader,
        DeviceState::Ready => UiIcon::Check,
        DeviceState::Unavailable => UiIcon::Unavailable,
        DeviceState::Failed => UiIcon::Error,
    }
}

fn device_state_tone(state: DeviceState) -> IndicatorTone {
    match state {
        DeviceState::Ready => IndicatorTone::Green,
        DeviceState::Initializing => IndicatorTone::Amber,
        DeviceState::Unavailable | DeviceState::Failed => IndicatorTone::Red,
    }
}

fn friendly_device_state(state: DeviceState) -> &'static str {
    match state {
        DeviceState::Initializing => "Initializing",
        DeviceState::Ready => "Ready",
        DeviceState::Unavailable => "Unavailable",
        DeviceState::Failed => "Failed",
    }
}

fn preview_caption(ui: &mut egui::Ui, kind: PreviewKind) {
    let font = FontId::proportional(11.0);
    let text_color = Color32::LIGHT_GRAY;
    let label = ui
        .painter()
        .layout_no_wrap(kind.label().to_owned(), font.clone(), text_color);
    let label_width = label.size().x;
    let live = (kind == PreviewKind::Output).then(|| {
        ui.painter()
            .layout_no_wrap("LIVE".to_owned(), font.clone(), LIVE_RED)
    });
    let icon_size = 13.0;
    let live_width = live.as_ref().map_or(0.0, |galley| 12.0 + galley.size().x);
    let total_width = icon_size + 6.0 + label_width + live_width;
    let height = label.size().y.max(icon_size);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(total_width, height), Sense::hover());
    let painter = ui.painter();
    let icon_rect = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );
    draw_icon(painter, icon_rect, kind.icon(), text_color, 1.35);
    let label_pos = Pos2::new(
        icon_rect.right() + 6.0,
        rect.center().y - label.size().y / 2.0,
    );
    painter.galley(label_pos, label, text_color);
    if let Some(live) = live {
        let live_x = label_pos.x + label_width + 12.0;
        painter.circle_filled(Pos2::new(live_x - 5.0, rect.center().y), 2.5, LIVE_RED);
        painter.galley(
            Pos2::new(live_x, rect.center().y - live.size().y / 2.0),
            live,
            LIVE_RED,
        );
    }
}

fn paint_fps_overlay(ui: &egui::Ui, preview_rect: Rect, fps: Option<u32>) {
    let text = fps.map_or_else(|| "-- FPS".to_owned(), |value| format!("{value} FPS"));
    let font = FontId::monospace(10.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text, font, Color32::from_rgb(232, 235, 240));
    let size = galley.size() + egui::vec2(10.0, 6.0);
    let overlay_max = preview_rect.max - egui::vec2(6.0, 6.0);
    let overlay = Rect::from_min_size(overlay_max - size, size);
    ui.painter()
        .rect_filled(overlay, 4, Color32::from_rgba_unmultiplied(6, 8, 12, 184));
    ui.painter().galley(
        overlay.min + egui::vec2(5.0, 3.0),
        galley,
        Color32::from_rgb(232, 235, 240),
    );
}

fn app_title(ui: &mut egui::Ui, texture: egui::TextureId) {
    const ICON_SIZE: f32 = 22.0;
    const GAP: f32 = 7.0;
    let color = Color32::from_rgb(205, 211, 222);
    let galley =
        ui.painter()
            .layout_no_wrap("StageSwap".to_owned(), FontId::proportional(14.0), color);
    let size = egui::vec2(
        ICON_SIZE + GAP + galley.size().x,
        galley.size().y.max(ICON_SIZE),
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let icon_rect = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - ICON_SIZE / 2.0),
        egui::vec2(ICON_SIZE, ICON_SIZE),
    );
    ui.painter().image(
        texture,
        icon_rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    ui.painter().galley(
        Pos2::new(
            icon_rect.right() + GAP,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}

fn icon_text(ui: &mut egui::Ui, icon: UiIcon, text: &str, color: Color32, strong: bool) {
    let font = FontId::proportional(if strong { 14.0 } else { 11.0 });
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let icon_size = if strong { 15.0 } else { 13.0 };
    let size = egui::vec2(
        icon_size + 6.0 + galley.size().x,
        galley.size().y.max(icon_size),
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let icon_rect = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );
    draw_icon(ui.painter(), icon_rect, icon, color, 1.4);
    ui.painter().galley(
        Pos2::new(
            icon_rect.right() + 6.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: UiIcon,
    text: &str,
    desired_size: Vec2,
    selected: bool,
    emphasized: bool,
) -> egui::Response {
    icon_button_impl(ui, icon, text, desired_size, selected, emphasized, None)
}

fn icon_only_button(
    ui: &mut egui::Ui,
    icon: UiIcon,
    accessible_label: &str,
    desired_size: Vec2,
) -> egui::Response {
    let response = icon_button_impl(ui, icon, "", desired_size, false, false, None);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), accessible_label)
    });
    response
}

fn accent_icon_button(
    ui: &mut egui::Ui,
    icon: UiIcon,
    text: &str,
    desired_size: Vec2,
    accent: Color32,
) -> egui::Response {
    icon_button_impl(ui, icon, text, desired_size, false, true, Some(accent))
}

fn icon_button_impl(
    ui: &mut egui::Ui,
    icon: UiIcon,
    text: &str,
    desired_size: Vec2,
    selected: bool,
    emphasized: bool,
    accent: Option<Color32>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    let visuals = ui.style().interact(&response);
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if let Some(accent) = accent {
        mix_color(
            Color32::from_rgb(25, 28, 33),
            accent,
            if response.hovered() { 0.48 } else { 0.34 },
        )
    } else if emphasized && !response.hovered() {
        Color32::from_rgb(38, 42, 50)
    } else {
        visuals.bg_fill
    };
    let stroke = if selected {
        Stroke::new(1.0, ui.visuals().selection.stroke.color)
    } else if let Some(accent) = accent {
        Stroke::new(1.25, accent)
    } else {
        visuals.bg_stroke
    };
    ui.painter().rect_filled(rect, 6, fill);
    ui.painter()
        .rect_stroke(rect, 6, stroke, StrokeKind::Inside);

    let color = if selected || emphasized || response.hovered() {
        Color32::WHITE
    } else {
        visuals.fg_stroke.color
    };
    let font = FontId::proportional(if emphasized { 14.0 } else { 12.0 });
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let icon_size = if emphasized { 16.0 } else { 14.0 };
    let text_gap = if text.is_empty() { 0.0 } else { 7.0 };
    let content_width = icon_size + text_gap + galley.size().x;
    let left = rect.center().x - content_width / 2.0;
    let icon_rect = Rect::from_min_size(
        Pos2::new(left, rect.center().y - icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );
    draw_icon(ui.painter(), icon_rect, icon, color, 1.45);
    if !text.is_empty() {
        ui.painter().galley(
            Pos2::new(
                icon_rect.right() + text_gap,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_arc_arrow(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    stroke: Stroke,
) {
    const SEGMENTS: usize = 12;
    let points = (0..=SEGMENTS)
        .map(|step| {
            let angle = egui::lerp(start_angle..=end_angle, step as f32 / SEGMENTS as f32);
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));

    let tip = center + egui::vec2(end_angle.cos(), end_angle.sin()) * radius;
    let direction = egui::vec2(-end_angle.sin(), end_angle.cos());
    let back = -direction * radius * 0.48;
    let wing = egui::vec2(-direction.y, direction.x) * radius * 0.24;
    painter.line_segment([tip, tip + back + wing], stroke);
    painter.line_segment([tip, tip + back - wing], stroke);
}

fn draw_icon(painter: &egui::Painter, rect: Rect, icon: UiIcon, color: Color32, width: f32) {
    let stroke = Stroke::new(width, color);
    let center = rect.center();
    let x = |fraction: f32| egui::lerp(rect.left()..=rect.right(), fraction);
    let y = |fraction: f32| egui::lerp(rect.top()..=rect.bottom(), fraction);
    match icon {
        UiIcon::Back => {
            painter.line_segment(
                [Pos2::new(x(0.62), y(0.14)), Pos2::new(x(0.28), y(0.5))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.28), y(0.5)), Pos2::new(x(0.62), y(0.86))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.3), y(0.5)), Pos2::new(x(0.9), y(0.5))],
                stroke,
            );
        }
        UiIcon::Camera => {
            let body = Rect::from_min_max(Pos2::new(x(0.08), y(0.27)), Pos2::new(x(0.92), y(0.85)));
            painter.rect_stroke(body, 2, stroke, StrokeKind::Inside);
            painter.line_segment(
                [Pos2::new(x(0.25), y(0.27)), Pos2::new(x(0.36), y(0.12))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.36), y(0.12)), Pos2::new(x(0.58), y(0.12))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.58), y(0.12)), Pos2::new(x(0.67), y(0.27))],
                stroke,
            );
            painter.circle_stroke(
                center + egui::vec2(0.0, rect.height() * 0.08),
                rect.width() * 0.18,
                stroke,
            );
        }
        UiIcon::Monitor => {
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(x(0.06), y(0.12)), Pos2::new(x(0.94), y(0.72))),
                2,
                stroke,
                StrokeKind::Inside,
            );
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.72)), Pos2::new(x(0.5), y(0.88))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.28), y(0.9)), Pos2::new(x(0.72), y(0.9))],
                stroke,
            );
        }
        UiIcon::Image => {
            painter.rect_stroke(rect.shrink(1.0), 2, stroke, StrokeKind::Inside);
            painter.circle_stroke(Pos2::new(x(0.7), y(0.3)), rect.width() * 0.08, stroke);
            painter.line_segment(
                [Pos2::new(x(0.12), y(0.78)), Pos2::new(x(0.38), y(0.48))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.38), y(0.48)), Pos2::new(x(0.56), y(0.67))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.56), y(0.67)), Pos2::new(x(0.72), y(0.53))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.72), y(0.53)), Pos2::new(x(0.9), y(0.78))],
                stroke,
            );
        }
        UiIcon::Broadcast => {
            painter.circle_filled(center, rect.width() * 0.1, color);
            painter.circle_stroke(center, rect.width() * 0.27, stroke);
            painter.circle_stroke(center, rect.width() * 0.43, stroke);
        }
        UiIcon::Robot => {
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.08)), Pos2::new(x(0.5), y(0.22))],
                stroke,
            );
            painter.circle_filled(Pos2::new(x(0.5), y(0.07)), rect.width() * 0.055, color);
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(x(0.13), y(0.22)), Pos2::new(x(0.87), y(0.78))),
                3,
                stroke,
                StrokeKind::Inside,
            );
            painter.circle_filled(Pos2::new(x(0.34), y(0.46)), rect.width() * 0.07, color);
            painter.circle_filled(Pos2::new(x(0.66), y(0.46)), rect.width() * 0.07, color);
            painter.line_segment(
                [Pos2::new(x(0.32), y(0.65)), Pos2::new(x(0.68), y(0.65))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.26), y(0.78)), Pos2::new(x(0.26), y(0.92))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.74), y(0.78)), Pos2::new(x(0.74), y(0.92))],
                stroke,
            );
        }
        UiIcon::Route => {
            painter.line_segment(
                [Pos2::new(x(0.18), y(0.5)), Pos2::new(x(0.48), y(0.5))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.48), y(0.5)), Pos2::new(x(0.72), y(0.22))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.48), y(0.5)), Pos2::new(x(0.72), y(0.78))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.72), y(0.22)), Pos2::new(x(0.9), y(0.22))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.72), y(0.78)), Pos2::new(x(0.9), y(0.78))],
                stroke,
            );
            painter.circle_filled(Pos2::new(x(0.14), y(0.5)), rect.width() * 0.08, color);
            painter.circle_filled(Pos2::new(x(0.9), y(0.22)), rect.width() * 0.08, color);
            painter.circle_filled(Pos2::new(x(0.9), y(0.78)), rect.width() * 0.08, color);
        }
        UiIcon::Loader => {
            for index in 0..8 {
                let angle = std::f32::consts::TAU * index as f32 / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                let alpha = 70 + index * 23;
                painter.circle_filled(
                    center + direction * rect.width() * 0.34,
                    rect.width() * 0.07,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha as u8),
                );
            }
        }
        UiIcon::Settings => {
            painter.circle_stroke(center, rect.width() * 0.18, stroke);
            painter.circle_stroke(center, rect.width() * 0.36, stroke);
            for direction in [
                egui::vec2(1.0, 0.0),
                egui::vec2(-1.0, 0.0),
                egui::vec2(0.0, 1.0),
                egui::vec2(0.0, -1.0),
                egui::vec2(0.7, 0.7),
                egui::vec2(-0.7, 0.7),
                egui::vec2(0.7, -0.7),
                egui::vec2(-0.7, -0.7),
            ] {
                painter.line_segment(
                    [
                        center + direction * rect.width() * 0.36,
                        center + direction * rect.width() * 0.48,
                    ],
                    stroke,
                );
            }
        }
        UiIcon::Play => {
            painter.line_segment(
                [Pos2::new(x(0.28), y(0.16)), Pos2::new(x(0.28), y(0.84))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.28), y(0.16)), Pos2::new(x(0.78), y(0.5))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.78), y(0.5)), Pos2::new(x(0.28), y(0.84))],
                stroke,
            );
        }
        UiIcon::Stop => {
            painter.rect_filled(rect.shrink(rect.width() * 0.22), 1, color);
        }
        UiIcon::Refresh => {
            let radius = rect.width() * 0.34;
            draw_arc_arrow(
                painter,
                center,
                radius,
                195.0_f32.to_radians(),
                345.0_f32.to_radians(),
                stroke,
            );
            draw_arc_arrow(
                painter,
                center,
                radius,
                15.0_f32.to_radians(),
                165.0_f32.to_radians(),
                stroke,
            );
        }
        UiIcon::Bell => {
            painter.line_segment(
                [Pos2::new(x(0.22), y(0.72)), Pos2::new(x(0.78), y(0.72))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.22), y(0.72)), Pos2::new(x(0.31), y(0.57))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.31), y(0.57)), Pos2::new(x(0.31), y(0.4))],
                stroke,
            );
            painter.circle_stroke(Pos2::new(x(0.5), y(0.4)), rect.width() * 0.19, stroke);
            painter.line_segment(
                [Pos2::new(x(0.69), y(0.4)), Pos2::new(x(0.69), y(0.57))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.69), y(0.57)), Pos2::new(x(0.78), y(0.72))],
                stroke,
            );
            painter.circle_filled(Pos2::new(x(0.5), y(0.83)), rect.width() * 0.07, color);
        }
        UiIcon::Window => {
            painter.rect_stroke(rect.shrink(1.0), 2, stroke, StrokeKind::Inside);
            painter.line_segment(
                [Pos2::new(x(0.08), y(0.3)), Pos2::new(x(0.92), y(0.3))],
                stroke,
            );
            for dot in [0.2, 0.32, 0.44] {
                painter.circle_filled(Pos2::new(x(dot), y(0.18)), rect.width() * 0.035, color);
            }
        }
        UiIcon::Folder => {
            painter.line_segment(
                [Pos2::new(x(0.08), y(0.28)), Pos2::new(x(0.38), y(0.28))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.38), y(0.28)), Pos2::new(x(0.48), y(0.4))],
                stroke,
            );
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(x(0.08), y(0.28)), Pos2::new(x(0.92), y(0.84))),
                2,
                stroke,
                StrokeKind::Inside,
            );
        }
        UiIcon::Info => {
            painter.circle_stroke(center, rect.width() * 0.42, stroke);
            painter.circle_filled(Pos2::new(x(0.5), y(0.29)), rect.width() * 0.055, color);
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.43)), Pos2::new(x(0.5), y(0.73))],
                stroke,
            );
        }
        UiIcon::Wrench => {
            painter.line_segment(
                [Pos2::new(x(0.22), y(0.8)), Pos2::new(x(0.7), y(0.32))],
                Stroke::new(width * 2.2, color),
            );
            painter.circle_stroke(Pos2::new(x(0.2), y(0.82)), rect.width() * 0.11, stroke);
            painter.line_segment(
                [Pos2::new(x(0.65), y(0.18)), Pos2::new(x(0.82), y(0.35))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.82), y(0.35)), Pos2::new(x(0.9), y(0.18))],
                stroke,
            );
        }
        UiIcon::Capture => {
            for (a, b) in [
                ((0.08, 0.35), (0.08, 0.08)),
                ((0.08, 0.08), (0.35, 0.08)),
                ((0.65, 0.08), (0.92, 0.08)),
                ((0.92, 0.08), (0.92, 0.35)),
                ((0.08, 0.65), (0.08, 0.92)),
                ((0.08, 0.92), (0.35, 0.92)),
                ((0.65, 0.92), (0.92, 0.92)),
                ((0.92, 0.92), (0.92, 0.65)),
            ] {
                painter.line_segment(
                    [Pos2::new(x(a.0), y(a.1)), Pos2::new(x(b.0), y(b.1))],
                    stroke,
                );
            }
            painter.circle_stroke(center, rect.width() * 0.18, stroke);
        }
        UiIcon::Check => {
            painter.line_segment(
                [Pos2::new(x(0.12), y(0.52)), Pos2::new(x(0.4), y(0.78))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.4), y(0.78)), Pos2::new(x(0.9), y(0.2))],
                stroke,
            );
        }
        UiIcon::Error => {
            painter.circle_stroke(center, rect.width() * 0.42, stroke);
            painter.line_segment(
                [Pos2::new(x(0.28), y(0.28)), Pos2::new(x(0.72), y(0.72))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.72), y(0.28)), Pos2::new(x(0.28), y(0.72))],
                stroke,
            );
        }
        UiIcon::Unavailable => {
            painter.circle_stroke(center, rect.width() * 0.42, stroke);
            painter.line_segment(
                [Pos2::new(x(0.2), y(0.8)), Pos2::new(x(0.8), y(0.2))],
                stroke,
            );
        }
        UiIcon::Question => {
            painter.line_segment(
                [Pos2::new(x(0.28), y(0.28)), Pos2::new(x(0.42), y(0.14))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.42), y(0.14)), Pos2::new(x(0.68), y(0.22))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.68), y(0.22)), Pos2::new(x(0.68), y(0.42))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.68), y(0.42)), Pos2::new(x(0.5), y(0.58))],
                stroke,
            );
            painter.circle_filled(Pos2::new(x(0.5), y(0.82)), rect.width() * 0.06, color);
        }
        UiIcon::Layers => {
            painter.line_segment(
                [Pos2::new(x(0.08), y(0.34)), Pos2::new(x(0.5), y(0.1))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.1)), Pos2::new(x(0.92), y(0.34))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.92), y(0.34)), Pos2::new(x(0.5), y(0.58))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.58)), Pos2::new(x(0.08), y(0.34))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.12), y(0.58)), Pos2::new(x(0.5), y(0.8))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.8)), Pos2::new(x(0.88), y(0.58))],
                stroke,
            );
        }
        UiIcon::Target => {
            painter.circle_stroke(center, rect.width() * 0.4, stroke);
            painter.circle_stroke(center, rect.width() * 0.16, stroke);
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.0)), Pos2::new(x(0.5), y(0.24))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.5), y(0.76)), Pos2::new(x(0.5), y(1.0))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.0), y(0.5)), Pos2::new(x(0.24), y(0.5))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.76), y(0.5)), Pos2::new(x(1.0), y(0.5))],
                stroke,
            );
        }
    }
}

fn mix_color(from: Color32, to: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let mix =
        |left: u8, right: u8| (left as f32 + (right as f32 - left as f32) * amount).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        mix(from.a(), to.a()),
    )
}

fn disco_ui_color(elapsed: Duration, offset: f32) -> Color32 {
    const COLORS: [Color32; 6] = [
        Color32::from_rgb(255, 28, 190),
        Color32::from_rgb(110, 70, 255),
        Color32::from_rgb(24, 176, 255),
        Color32::from_rgb(36, 245, 82),
        Color32::from_rgb(255, 224, 34),
        Color32::from_rgb(255, 70, 38),
    ];
    let position = ((elapsed.as_secs_f32() * 0.42 + offset).rem_euclid(1.0)) * COLORS.len() as f32;
    let index = position.floor() as usize % COLORS.len();
    mix_color(
        COLORS[index],
        COLORS[(index + 1) % COLORS.len()],
        position.fract(),
    )
}

fn disco_flash_envelope(elapsed: Duration) -> f32 {
    let seconds = elapsed.as_secs_f32();
    let beat = (seconds / 1.2).fract();
    let primary = if beat < 0.1 { 1.0 - beat / 0.1 } else { 0.0 };
    let secondary = if (0.22..0.29).contains(&beat) {
        1.0 - (beat - 0.22) / 0.07
    } else {
        0.0
    };
    let major = (seconds / 3.0).fract();
    let major = if major < 0.055 {
        1.0 - major / 0.055
    } else {
        0.0
    };
    primary.max(secondary * 0.72).max(major)
}

fn disco_ball_drop_progress(elapsed: Duration) -> f32 {
    let progress = (elapsed.as_secs_f32() / 1.35).clamp(0.0, 1.0);
    let shifted = progress - 1.0;
    1.0 + 2.701_58 * shifted.powi(3) + 1.701_58 * shifted.powi(2)
}

fn disco_ball_center(rect: Rect, activation_elapsed: Duration) -> Pos2 {
    let progress = disco_ball_drop_progress(activation_elapsed);
    Pos2::new(
        rect.center().x,
        rect.top() - 72.0 + (rect.top() + 158.0 - (rect.top() - 72.0)) * progress,
    )
}

fn paint_disco_ball(
    painter: &egui::Painter,
    rect: Rect,
    elapsed: Duration,
    activation_elapsed: Duration,
) {
    let center = disco_ball_center(rect, activation_elapsed);
    let radius = (rect.height() * 0.075).clamp(44.0, 62.0);
    let motor = Rect::from_center_size(
        Pos2::new(center.x, rect.top() + 11.0),
        egui::vec2(52.0, 22.0),
    );
    painter.rect_filled(motor, 5, Color32::from_rgb(54, 58, 68));
    painter.rect_stroke(
        motor,
        5,
        Stroke::new(1.5, Color32::from_rgb(154, 161, 174)),
        StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_center_size(
            Pos2::new(center.x, motor.bottom() + 4.0),
            egui::vec2(13.0, 8.0),
        ),
        2,
        Color32::from_rgb(104, 111, 123),
    );
    if center.y - radius > motor.bottom() {
        painter.line_segment(
            [
                Pos2::new(center.x - 1.2, motor.bottom() + 8.0),
                Pos2::new(center.x - 1.2, center.y - radius),
            ],
            Stroke::new(3.2, Color32::from_rgb(72, 76, 86)),
        );
        painter.line_segment(
            [
                Pos2::new(center.x + 1.1, motor.bottom() + 8.0),
                Pos2::new(center.x + 1.1, center.y - radius),
            ],
            Stroke::new(1.1, Color32::from_rgb(196, 201, 211)),
        );
    }
    let t = elapsed.as_secs_f32();
    let flash = disco_flash_envelope(elapsed);
    let shimmer = (t * 2.1).sin().mul_add(0.5, 0.5);
    painter.circle_filled(
        center,
        radius + 38.0 + shimmer * 5.0,
        Color32::from_rgba_unmultiplied(230, 240, 255, 9),
    );
    painter.circle_filled(
        center,
        radius + 25.0 + shimmer * 3.0,
        Color32::from_rgba_unmultiplied(220, 235, 255, 17),
    );
    painter.circle_filled(
        center,
        radius + 14.0,
        Color32::from_rgba_unmultiplied(245, 250, 255, 34),
    );
    painter.circle_filled(center, radius + 5.0, Color32::from_rgb(45, 50, 62));
    painter.circle_filled(center, radius, Color32::from_rgb(72, 78, 91));

    const LATITUDE_BANDS: usize = 12;
    const LONGITUDE_BANDS: usize = 16;
    let project = |latitude: f32, longitude: f32| {
        Pos2::new(
            center.x + radius * latitude.cos() * longitude.sin(),
            center.y + radius * latitude.sin(),
        )
    };
    for row in 0..LATITUDE_BANDS {
        let latitude_top = -std::f32::consts::FRAC_PI_2
            + row as f32 * std::f32::consts::PI / LATITUDE_BANDS as f32;
        let latitude_bottom = -std::f32::consts::FRAC_PI_2
            + (row + 1) as f32 * std::f32::consts::PI / LATITUDE_BANDS as f32;
        let latitude = (latitude_top + latitude_bottom) * 0.5;
        for column in 0..LONGITUDE_BANDS {
            let longitude_left = -std::f32::consts::FRAC_PI_2
                + column as f32 * std::f32::consts::PI / LONGITUDE_BANDS as f32;
            let longitude_right = -std::f32::consts::FRAC_PI_2
                + (column + 1) as f32 * std::f32::consts::PI / LONGITUDE_BANDS as f32;
            let longitude = (longitude_left + longitude_right) * 0.5;
            let normal_x = latitude.cos() * longitude.sin();
            let normal_y = latitude.sin();
            let normal_z = latitude.cos() * longitude.cos();
            let diffuse = (normal_x * -0.38 + normal_y * -0.48 + normal_z * 0.79).clamp(0.0, 1.0);
            let specular = diffuse.powi(7);
            let reflection =
                ((longitude * 3.2 + latitude * 1.6 + t * 1.15).sin() * 0.5 + 0.5).powi(6);
            let moving_glint =
                ((longitude * 5.4 - latitude * 2.7 + t * 1.85).sin() * 0.5 + 0.5).powi(14);
            let rim = (1.0 - normal_z.max(0.0)).powi(3);
            let brightness = (58.0
                + diffuse * 142.0
                + specular * 105.0
                + reflection * 52.0
                + moving_glint * 112.0
                + rim * 26.0)
                .min(255.0) as u8;
            let silver = Color32::from_rgb(
                brightness.saturating_sub(5),
                brightness,
                brightness.saturating_add(5),
            );
            let accent = disco_ui_color(
                elapsed,
                (column as f32 / LONGITUDE_BANDS as f32
                    + row as f32 / LATITUDE_BANDS as f32 * 0.31
                    + t * 0.035)
                    .fract(),
            );
            let reflected = mix_color(
                silver,
                accent,
                (0.03 + reflection * 0.48 + moving_glint * 0.12).min(0.62),
            );
            let reflected = mix_color(
                reflected,
                Color32::WHITE,
                (specular * 0.74 + reflection * 0.24 + moving_glint * 0.92 + flash * 0.11)
                    .min(0.96),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    project(latitude_top, longitude_left),
                    project(latitude_top, longitude_right),
                    project(latitude_bottom, longitude_right),
                    project(latitude_bottom, longitude_left),
                ],
                reflected,
                Stroke::new(0.65, Color32::from_rgba_unmultiplied(24, 28, 36, 135)),
            ));
        }
    }
    painter.circle_stroke(
        center,
        radius + 1.5,
        Stroke::new(5.0, Color32::from_rgba_unmultiplied(245, 250, 255, 116)),
    );
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(
            2.5,
            mix_color(Color32::WHITE, disco_ui_color(elapsed, 0.12), 0.34),
        ),
    );
    if flash > 0.0 {
        painter.circle_stroke(
            center,
            radius + flash * 3.0,
            Stroke::new(
                2.0 + flash * 1.5,
                Color32::from_rgba_unmultiplied(255, 255, 255, (flash * 80.0) as u8),
            ),
        );
    }
    painter.circle_stroke(
        center,
        radius * 0.72,
        Stroke::new(1.4, Color32::from_rgba_unmultiplied(245, 250, 255, 150)),
    );
    painter.circle_filled(
        Pos2::new(center.x - radius * 0.3, center.y - radius * 0.34),
        radius * 0.24,
        Color32::from_rgba_unmultiplied(255, 255, 255, 105),
    );
    painter.circle_filled(
        Pos2::new(center.x - radius * 0.34, center.y - radius * 0.38),
        radius * 0.085,
        Color32::from_rgba_unmultiplied(255, 255, 255, 242),
    );
    let glint_center = Pos2::new(
        center.x + (t * 1.25).sin() * radius * 0.48,
        center.y - radius * 0.12 + (t * 0.8).cos() * radius * 0.23,
    );
    painter.circle_filled(
        glint_center,
        radius * 0.055,
        Color32::from_rgba_unmultiplied(255, 255, 255, 250),
    );
    painter.line_segment(
        [
            Pos2::new(glint_center.x - radius * 0.22, glint_center.y),
            Pos2::new(glint_center.x + radius * 0.22, glint_center.y),
        ],
        Stroke::new(2.2, Color32::from_rgba_unmultiplied(255, 255, 255, 205)),
    );
    painter.line_segment(
        [
            Pos2::new(glint_center.x, glint_center.y - radius * 0.22),
            Pos2::new(glint_center.x, glint_center.y + radius * 0.22),
        ],
        Stroke::new(2.2, Color32::from_rgba_unmultiplied(255, 255, 255, 205)),
    );
}

fn paint_disco_interface(
    context: &egui::Context,
    rect: Rect,
    elapsed: Duration,
    activation_elapsed: Duration,
    enabled: bool,
) {
    let painter = context.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("disco-interface-overlay"),
    ));
    if enabled {
        let t = elapsed.as_secs_f32();
        let flash = disco_flash_envelope(elapsed);
        let pulse_color = disco_ui_color(elapsed, 0.88);
        let pulse_alpha = (10.0 + (t * 1.7).sin().mul_add(0.5, 0.5) * 16.0 + flash * 24.0) as u8;
        painter.rect_filled(
            rect,
            0,
            Color32::from_rgba_unmultiplied(
                pulse_color.r(),
                pulse_color.g(),
                pulse_color.b(),
                pulse_alpha,
            ),
        );
        if flash > 0.0 {
            painter.rect_filled(
                rect,
                0,
                Color32::from_rgba_unmultiplied(255, 255, 255, (flash * 4.0) as u8),
            );
        }

        let ball_center = disco_ball_center(rect, activation_elapsed);
        if flash > 0.0 {
            painter.circle_filled(
                ball_center,
                82.0 + flash * 45.0,
                Color32::from_rgba_unmultiplied(255, 255, 255, (flash * 16.0) as u8),
            );
            for index in 0..20 {
                let angle = index as f32 * std::f32::consts::TAU / 20.0 + t * 0.18;
                let start = Pos2::new(
                    ball_center.x + angle.cos() * 58.0,
                    ball_center.y + angle.sin() * 58.0,
                );
                let end = Pos2::new(
                    ball_center.x + angle.cos() * (95.0 + flash * 65.0),
                    ball_center.y + angle.sin() * (95.0 + flash * 65.0),
                );
                painter.line_segment(
                    [start, end],
                    Stroke::new(
                        1.0 + flash * 1.2,
                        Color32::from_rgba_unmultiplied(255, 255, 255, (flash * 72.0) as u8),
                    ),
                );
            }
        }
        for index in 0..28 {
            let angle = t * 0.48 + index as f32 * std::f32::consts::TAU / 28.0;
            let length = rect.width().max(rect.height()) * 1.2;
            let end = Pos2::new(
                ball_center.x + angle.cos() * length,
                ball_center.y + angle.sin() * length,
            );
            let color = disco_ui_color(elapsed, index as f32 / 28.0);
            painter.line_segment(
                [ball_center, end],
                Stroke::new(
                    1.4 + (index % 4) as f32,
                    Color32::from_rgba_unmultiplied(
                        color.r(),
                        color.g(),
                        color.b(),
                        (72.0 + flash * 32.0) as u8,
                    ),
                ),
            );
        }

        for index in 0..13 {
            let origin_x =
                ball_center.x + (t * 0.31 + index as f32 * 1.4).sin() * rect.width() * 0.012;
            let target_x = rect.left()
                + rect.width() * (index as f32 + 0.5) / 13.0
                + (t * 0.47 + index as f32).cos() * rect.width() * 0.11;
            let half_width = rect.width() * (0.035 + (index % 3) as f32 * 0.012);
            let color = disco_ui_color(elapsed, index as f32 / 13.0);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(origin_x - 3.0, ball_center.y),
                    Pos2::new(origin_x + 3.0, ball_center.y),
                    Pos2::new(target_x + half_width, rect.bottom()),
                    Pos2::new(target_x - half_width, rect.bottom()),
                ],
                Color32::from_rgba_unmultiplied(
                    color.r(),
                    color.g(),
                    color.b(),
                    (42.0 + flash * 22.0) as u8,
                ),
                Stroke::NONE,
            ));
        }

        let border = rect.shrink(2.0);
        for (index, segment) in [
            [border.left_top(), border.right_top()],
            [border.right_top(), border.right_bottom()],
            [border.right_bottom(), border.left_bottom()],
            [border.left_bottom(), border.left_top()],
        ]
        .into_iter()
        .enumerate()
        {
            painter.line_segment(
                segment,
                Stroke::new(
                    4.0 + (t * 2.0 + index as f32).sin().abs() * 2.0,
                    disco_ui_color(elapsed, index as f32 * 0.19),
                ),
            );
        }

        for ring in 0..5 {
            let color = disco_ui_color(elapsed, ring as f32 * 0.17);
            let radius = 82.0 + ring as f32 * 36.0 + (t * 1.4 + ring as f32).sin() * 9.0;
            painter.circle_stroke(
                ball_center,
                radius,
                Stroke::new(
                    8.0 - ring as f32,
                    Color32::from_rgba_unmultiplied(
                        color.r(),
                        color.g(),
                        color.b(),
                        48_u8.saturating_sub(ring * 6),
                    ),
                ),
            );
        }

        for index in 0..46 {
            let angle = t * (0.52 + (index % 4) as f32 * 0.035) + index as f32 * 2.399_963;
            let distance = 110.0 + (index % 11) as f32 * rect.width().min(rect.height()) * 0.055;
            let position = Pos2::new(
                ball_center.x + angle.cos() * distance,
                ball_center.y + angle.sin() * distance * 0.74,
            );
            let color = disco_ui_color(elapsed, index as f32 / 46.0);
            let pulse = (((t * 2.7 + index as f32 * 0.91).sin() * 0.5 + 0.5).powi(4)
                + flash * if index % 3 == 0 { 0.28 } else { 0.1 })
            .min(1.0);
            painter.circle_filled(
                position,
                9.0 + pulse * 16.0,
                Color32::from_rgba_unmultiplied(
                    color.r(),
                    color.g(),
                    color.b(),
                    (20.0 + pulse * 46.0) as u8,
                ),
            );
            painter.circle_filled(
                position,
                1.8 + pulse * 3.8,
                Color32::from_rgba_unmultiplied(255, 255, 255, (125.0 + pulse * 130.0) as u8),
            );
        }

        for index in 0..6 {
            let angle = t * 0.19 + index as f32 * std::f32::consts::TAU / 6.0;
            let position = Pos2::new(
                ball_center.x + angle.cos() * rect.width() * 0.42,
                ball_center.y + angle.sin() * rect.height() * 0.46,
            );
            let color = disco_ui_color(elapsed, index as f32 / 6.0);
            painter.circle_filled(
                position,
                rect.width().min(rect.height()) * (0.13 + (index % 2) as f32 * 0.04),
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 34),
            );
        }

        let dance_floor_top = rect.bottom() - 54.0;
        for row in 0..3 {
            for column in 0..18 {
                let tile_width = rect.width() / 18.0;
                let tile = Rect::from_min_max(
                    Pos2::new(
                        rect.left() + column as f32 * tile_width + 1.0,
                        dance_floor_top + row as f32 * 18.0 + 1.0,
                    ),
                    Pos2::new(
                        rect.left() + (column + 1) as f32 * tile_width - 1.0,
                        dance_floor_top + (row + 1) as f32 * 18.0 - 1.0,
                    ),
                );
                let color =
                    disco_ui_color(elapsed, (column as f32 * 0.08 + row as f32 * 0.21).fract());
                let alpha = if (column + row + (t * 4.0) as usize).is_multiple_of(3) {
                    105
                } else {
                    48
                };
                painter.rect_filled(
                    tile,
                    2,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
                );
            }
        }

        for index in 0..110 {
            let seed = ((index * 73 + 19) % 997) as f32 / 997.0;
            let speed = 0.08 + ((index * 41 + 7) % 100) as f32 / 720.0;
            let travel = (seed + t * speed).fract();
            let direction = if index % 2 == 0 { -1.0 } else { 1.0 };
            let spread = direction * travel * rect.width() * (0.14 + seed * 0.34);
            let sway = (t * (0.8 + seed) + index as f32 * 0.73).sin() * 18.0;
            let center = Pos2::new(
                ball_center.x + spread + sway,
                ball_center.y + 24.0 + travel * (rect.bottom() - ball_center.y + 30.0),
            );
            let angle = t * (1.1 + seed * 2.2) + index as f32;
            let length = 5.0 + ((index * 17) % 9) as f32;
            let delta = egui::vec2(angle.cos() * length, angle.sin() * length);
            let color = disco_ui_color(elapsed, seed);
            painter.line_segment(
                [center - delta, center + delta],
                Stroke::new(
                    2.0 + (index % 3) as f32 * 0.7,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 220),
                ),
            );
        }

        for index in 0..56 {
            let orbit_angle = t * (0.42 + (index % 5) as f32 * 0.08) + index as f32 * 2.399_963;
            let orbit_radius = 72.0 + (index % 7) as f32 * 31.0;
            let center = Pos2::new(
                ball_center.x + orbit_angle.cos() * orbit_radius,
                ball_center.y + orbit_angle.sin() * orbit_radius * 0.62,
            );
            let pulse = ((t * 2.2 + index as f32 * 1.7).sin() * 0.5 + 0.5).powi(3);
            let radius = 3.0 + pulse * 8.0;
            let color = disco_ui_color(elapsed, index as f32 / 56.0);
            let color = Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                (55.0 + pulse * 155.0) as u8,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - radius, center.y),
                    Pos2::new(center.x + radius, center.y),
                ],
                Stroke::new(1.3, color),
            );
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - radius),
                    Pos2::new(center.x, center.y + radius),
                ],
                Stroke::new(1.3, color),
            );
        }

        for index in 0..52 {
            let x = rect.left() + ((index * 149 + 43) % 1021) as f32 / 1021.0 * rect.width();
            let y = rect.top() + ((index * 263 + 71) % 1013) as f32 / 1013.0 * rect.height();
            let pulse = ((t * 2.5 + index as f32 * 1.31).sin() * 0.5 + 0.5).powi(5);
            let radius = 1.5 + pulse * 7.5;
            let center = Pos2::new(x, y);
            let color = disco_ui_color(elapsed, index as f32 / 52.0);
            let color = Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                (60.0 + pulse * 190.0) as u8,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - radius, center.y),
                    Pos2::new(center.x + radius, center.y),
                ],
                Stroke::new(1.5, color),
            );
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - radius),
                    Pos2::new(center.x, center.y + radius),
                ],
                Stroke::new(1.5, color),
            );
        }

        paint_disco_ball(&painter, rect, elapsed, activation_elapsed);
    }
}

fn preview_texture_size(frame: &Frame, maximum: Vec2, pixels_per_point: f32) -> [usize; 2] {
    let maximum_width =
        ((maximum.x * pixels_per_point).round().max(1.0) as u32).min(MAX_PREVIEW_TEXTURE_WIDTH);
    let maximum_height =
        ((maximum.y * pixels_per_point).round().max(1.0) as u32).min(MAX_PREVIEW_TEXTURE_HEIGHT);
    let scale = f64::min(
        f64::min(
            f64::from(maximum_width) / f64::from(frame.size.width),
            f64::from(maximum_height) / f64::from(frame.size.height),
        ),
        1.0,
    );
    [
        (f64::from(frame.size.width) * scale).round().max(1.0) as usize,
        (f64::from(frame.size.height) * scale).round().max(1.0) as usize,
    ]
}

fn frame_image(frame: &Frame, target: [usize; 2]) -> egui::ColorImage {
    let source_x = (0..target[0])
        .map(|x| x * frame.size.width as usize / target[0] * 4)
        .collect::<Vec<_>>();
    let source_rows = (0..target[1])
        .map(|y| y * frame.size.height as usize / target[1] * frame.stride as usize)
        .collect::<Vec<_>>();
    let mut pixels = Vec::with_capacity(target[0] * target[1]);
    for source_row in source_rows {
        for &source_x in &source_x {
            let offset = source_row + source_x;
            pixels.push(Color32::from_rgba_unmultiplied(
                frame.pixels()[offset + 2],
                frame.pixels()[offset + 1],
                frame.pixels()[offset],
                frame.pixels()[offset + 3],
            ));
        }
    }
    egui::ColorImage::new(target, pixels)
}

#[cfg(test)]
fn bgra_color(value: u32) -> Color32 {
    let [b, g, r, a] = value.to_le_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ui_fonts_are_bundled() {
        assert!(!egui::FontDefinitions::default().font_data.is_empty());
    }

    #[test]
    fn user_data_directory_is_stageswap() {
        assert_eq!(
            local_data_directory()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("StageSwap")
        );
    }

    #[test]
    fn window_title_includes_the_package_version() {
        assert_eq!(
            WINDOW_TITLE,
            format!("StageSwap - v{}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn minimized_start_is_reasserted_after_eframe_initialization() {
        assert_eq!(initial_visibility_override(true, true), None);
        assert_eq!(initial_visibility_override(false, true), Some(false));
        assert_eq!(initial_visibility_override(false, false), Some(true));
    }

    #[test]
    fn visible_ui_and_hidden_logic_use_distinct_repaint_cadences() {
        assert_eq!(repaint_interval(true), VISIBLE_REFRESH);
        assert_eq!(repaint_interval(false), HIDDEN_REFRESH);
        assert_eq!(VISIBLE_REFRESH, Duration::from_nanos(1_000_000_000 / 30));
        assert_eq!(HIDDEN_REFRESH, Duration::from_millis(250));
    }

    #[test]
    fn active_disco_interface_renders_ball_beams_particles_and_borders() {
        let context = egui::Context::default();
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..egui::RawInput::default()
            },
            |ui| {
                paint_disco_interface(
                    ui.ctx(),
                    screen_rect,
                    Duration::from_millis(750),
                    Duration::from_millis(750),
                    true,
                );
            },
        );
        assert!(
            output.shapes.len() >= 600,
            "the active disco overlay should include dense reflections and effects"
        );
    }

    #[test]
    fn disco_ball_lowers_from_above_the_window_and_settles_on_screen() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let hidden = disco_ball_center(rect, Duration::ZERO);
        let settled = disco_ball_center(rect, Duration::from_secs(2));
        assert!(hidden.y < rect.top());
        assert!((settled.y - (rect.top() + 158.0)).abs() < 0.01);
    }

    #[test]
    fn disco_flash_envelope_produces_double_hits_and_quiet_gaps() {
        assert_eq!(disco_flash_envelope(Duration::ZERO), 1.0);
        assert_eq!(disco_flash_envelope(Duration::from_millis(180)), 0.0);
        assert!(disco_flash_envelope(Duration::from_millis(270)) > 0.6);
        assert_eq!(disco_flash_envelope(Duration::from_millis(600)), 0.0);
        assert_eq!(disco_flash_envelope(Duration::from_secs(3)), 1.0);
    }

    #[test]
    fn preview_conversion_preserves_size_and_bgra_channels() {
        let frame = Frame::new(
            vec![3, 2, 1, 255, 30, 20, 10, 255].into(),
            stageswap_core::Size::new(2, 1),
            8,
            1,
            0,
            Instant::now(),
        )
        .unwrap();
        let image = frame_image(&frame, [2, 1]);
        assert_eq!(image.size, [2, 1]);
        assert_eq!(image.as_raw(), &[1, 2, 3, 255, 10, 20, 30, 255]);
    }

    #[test]
    fn preview_texture_is_capped_to_its_display_size() {
        let frame = Frame::placeholder(
            stageswap_core::Size::new(1280, 720),
            0xff03_0201,
            1,
            0,
            Instant::now(),
        );
        let size = preview_texture_size(&frame, egui::vec2(320.0, 180.0), 1.0);
        assert_eq!(size, [320, 180]);
        let image = frame_image(&frame, size);
        assert_eq!(image.size, size);
        assert_eq!(image.pixels[0], Color32::from_rgb(3, 2, 1));

        let high_dpi = preview_texture_size(&frame, egui::vec2(640.0, 360.0), 2.0);
        assert_eq!(high_dpi, [480, 270]);
    }

    #[test]
    fn preview_converter_collapses_pending_jobs_to_latest_frame() {
        let converter = PreviewConverter::new(PreviewKind::Output);
        let now = Instant::now();
        for sequence in 1..=12 {
            converter.submit(
                Arc::new(Frame::placeholder(
                    stageswap_core::Size::new(1280, 720),
                    0xff00_0000 | sequence,
                    u64::from(sequence),
                    0,
                    now,
                )),
                [480, 270],
            );
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(prepared) = converter.take_ready()
                && prepared.frame.sequence == 12
            {
                assert_eq!(prepared.size, [480, 270]);
                assert_eq!(prepared.image.pixels[0], Color32::from_rgb(0, 0, 12));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "latest preview frame was not prepared"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn preview_converter_keeps_completed_frame_when_newer_request_is_pending() {
        let now = Instant::now();
        let completed = Arc::new(Frame::placeholder(
            stageswap_core::Size::new(2, 2),
            0xff00_0001,
            1,
            0,
            now,
        ));
        let pending = Arc::new(Frame::placeholder(
            stageswap_core::Size::new(2, 2),
            0xff00_0002,
            2,
            0,
            now,
        ));
        let size = [2, 2];
        let mut state = PreviewConverterState {
            latest_request: Some((Arc::clone(&pending), size)),
            pending: Some(PreviewJob {
                frame: Arc::clone(&pending),
                size,
            }),
            ..PreviewConverterState::default()
        };

        store_completed_preview(
            &mut state,
            PreparedPreview {
                image: frame_image(&completed, size),
                frame: completed,
                size,
            },
        );

        assert_eq!(state.ready.as_ref().unwrap().frame.sequence, 1);
        assert_eq!(state.pending.as_ref().unwrap().frame.sequence, 2);
    }

    #[test]
    fn placeholder_color_round_trips_from_bgra() {
        assert_eq!(
            bgra_color(0x4433_2211),
            Color32::from_rgba_unmultiplied(0x33, 0x22, 0x11, 0x44)
        );
    }

    #[test]
    fn window_resize_stays_at_sixteen_by_nine() {
        let wider =
            aspect_locked_window_size(egui::vec2(1440.0, 720.0), Some(egui::vec2(1280.0, 720.0)));
        assert!((wider.x / wider.y - WINDOW_ASPECT_RATIO).abs() < 0.000_001);
        let taller =
            aspect_locked_window_size(egui::vec2(1280.0, 800.0), Some(egui::vec2(1280.0, 720.0)));
        assert!((taller.x / taller.y - WINDOW_ASPECT_RATIO).abs() < 0.000_001);
        let minimum = aspect_locked_window_size(egui::vec2(400.0, 300.0), None);
        assert_eq!(minimum.y, MIN_WINDOW_HEIGHT);
    }

    #[test]
    fn preview_contours_mark_live_output_and_active_source() {
        assert_eq!(
            preview_contour(PreviewKind::Output, Source::Camera),
            PreviewContour::Live
        );
        assert_eq!(
            preview_contour(PreviewKind::Webcam, Source::Camera),
            PreviewContour::Active
        );
        assert_eq!(
            preview_contour(PreviewKind::Screen, Source::Camera),
            PreviewContour::Neutral
        );
        assert_eq!(
            preview_contour(PreviewKind::Screen, Source::Screen),
            PreviewContour::Active
        );
        assert_eq!(
            preview_contour(PreviewKind::Webcam, Source::Placeholder),
            PreviewContour::Neutral
        );
        assert_eq!(
            preview_contour(PreviewKind::Reference, Source::Screen),
            PreviewContour::Neutral
        );
    }

    #[test]
    fn health_states_have_lifecycle_order_icons_and_semantic_colors() {
        assert_eq!(
            HEALTH_STATES,
            [
                DeviceState::Initializing,
                DeviceState::Ready,
                DeviceState::Unavailable,
                DeviceState::Failed,
            ]
        );
        assert_eq!(device_state_icon(DeviceState::Initializing), UiIcon::Loader);
        assert_eq!(device_state_icon(DeviceState::Ready), UiIcon::Check);
        assert_eq!(
            device_state_icon(DeviceState::Unavailable),
            UiIcon::Unavailable
        );
        assert_eq!(device_state_icon(DeviceState::Failed), UiIcon::Error);
        assert_eq!(
            indicator_palette(device_state_tone(DeviceState::Ready), 1.0).1,
            ACTIVE_GREEN
        );
        assert_eq!(
            indicator_palette(device_state_tone(DeviceState::Initializing), 1.0).1,
            TRANSITION_AMBER
        );
        assert_eq!(
            indicator_palette(device_state_tone(DeviceState::Failed), 1.0).1,
            LIVE_RED
        );
        assert_eq!(
            indicator_palette(device_state_tone(DeviceState::Unavailable), 1.0).1,
            LIVE_RED
        );
        let inactive = indicator_palette(device_state_tone(DeviceState::Ready), 0.0);
        assert_eq!(inactive.1, Color32::from_rgb(91, 97, 108));
        assert_eq!(
            inactive,
            indicator_palette(device_state_tone(DeviceState::Failed), 0.0)
        );
    }

    #[test]
    fn not_matching_is_warning_amber_but_missing_reference_is_error_red() {
        assert_eq!(
            detection_indicator_tone(DetectionState::NotMatching),
            IndicatorTone::Amber
        );
        assert_eq!(
            settings_detection_style(DetectionState::NotMatching).2,
            TRANSITION_AMBER
        );
        assert_eq!(
            detection_indicator_tone(DetectionState::ReferenceMissing),
            IndicatorTone::Red
        );
        assert_eq!(
            settings_detection_style(DetectionState::ReferenceMissing).2,
            LIVE_RED
        );
    }

    #[test]
    fn fps_overlays_use_runtime_metrics_for_all_live_pipelines() {
        assert!(PreviewKind::Webcam.shows_fps());
        assert!(PreviewKind::Screen.shows_fps());
        assert!(PreviewKind::Output.shows_fps());
        assert!(!PreviewKind::Reference.shows_fps());
        assert!(!PreviewOptions::settings("missing").show_fps);
        assert!(PreviewOptions::dashboard(PreviewKind::Webcam, Some(30)).show_fps);
        let output = PreviewOptions::dashboard(PreviewKind::Output, Some(30));
        assert!(output.show_fps);
        assert_eq!(output.fps, Some(30));
    }

    #[test]
    fn indicator_rows_keep_content_height_inside_a_tall_panel() {
        let context = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 600.0))),
            ..egui::RawInput::default()
        };
        let _ = context.run_ui(input, |ui| {
            ui.set_min_height(560.0);
            let choices = HEALTH_STATES.map(|state| IndicatorChoice {
                icon: device_state_icon(state),
                label: friendly_device_state(state),
                current: state == DeviceState::Ready,
                tone: device_state_tone(state),
                span: 1,
            });
            let rect = indicator_group(ui, UiIcon::Camera, "Webcam", &choices, None);
            assert!(
                rect.height() <= 30.0,
                "indicator row expanded to {} px",
                rect.height()
            );
        });
    }

    #[test]
    fn settings_sidebar_navigation_is_vertical_and_non_overlapping() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
        );
        let context = egui::Context::default();
        for (viewport, dpi_scale) in [
            (egui::vec2(820.0, 540.0), 1.0),
            (egui::vec2(820.0, 540.0), 1.5),
            (egui::vec2(820.0, 600.0), 1.0),
            (
                egui::vec2(MIN_WINDOW_HEIGHT * WINDOW_ASPECT_RATIO, MIN_WINDOW_HEIGHT),
                1.5,
            ),
            (egui::vec2(1280.0, 720.0), 1.5),
        ] {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                ..egui::RawInput::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(dpi_scale);
            let mut sidebar_rect = Rect::NOTHING;
            let mut sidebar_layout = None;
            let _ = context.run_ui(input, |ui| {
                let sidebar = ui.allocate_ui_with_layout(
                    egui::vec2(SETTINGS_SIDEBAR_WIDTH, viewport.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| app.settings_sidebar(ui, viewport.y),
                );
                sidebar_rect = sidebar.response.rect;
                sidebar_layout = Some(sidebar.inner);
            });

            let sidebar_layout = sidebar_layout.unwrap();
            for rect in [
                sidebar_layout.brand_icon,
                sidebar_layout.brand_title,
                sidebar_layout.brand_separator,
                sidebar_layout.back,
            ] {
                assert!(rect.is_positive());
                assert!(sidebar_rect.contains_rect(rect));
            }
            assert!((sidebar_layout.brand_icon.width() - 96.0).abs() < 0.01);
            assert!((sidebar_layout.brand_icon.height() - 96.0).abs() < 0.01);
            assert!(sidebar_layout.brand_icon.bottom() < sidebar_layout.brand_title.top());
            assert!(sidebar_layout.brand_title.bottom() < sidebar_layout.brand_separator.top());
            assert!(sidebar_layout.brand_separator.bottom() < sidebar_layout.back.top());
            assert_eq!(
                sidebar_layout.primary_navigation.len(),
                SettingsTab::PRIMARY.len()
            );
            for rect in &sidebar_layout.primary_navigation {
                assert!(rect.is_positive(), "invalid navigation rect: {rect:?}");
                assert!(
                    sidebar_rect.contains_rect(*rect),
                    "navigation rect escaped sidebar: {rect:?} outside {sidebar_rect:?}"
                );
                assert!((rect.height() - 36.0).abs() < 0.01);
            }
            for pair in sidebar_layout.primary_navigation.windows(2) {
                assert!(
                    pair[1].top() - pair[0].bottom() >= 2.9,
                    "navigation rows overlap: {:?} and {:?}",
                    pair[0],
                    pair[1]
                );
                assert!(!pair[0].intersects(pair[1]));
            }
            assert!(sidebar_layout.back.bottom() < sidebar_layout.primary_navigation[0].top());
            assert!(
                sidebar_layout.primary_navigation.last().unwrap().bottom()
                    < sidebar_layout.diagnostics.top(),
                "primary navigation overlaps Diagnostics: primary={:?}, diagnostics={:?}",
                sidebar_layout.primary_navigation.last().unwrap(),
                sidebar_layout.diagnostics
            );
            assert!(sidebar_layout.diagnostics.bottom() < sidebar_layout.save_status.top());
            assert!(sidebar_rect.contains_rect(sidebar_layout.diagnostics));
            assert!(sidebar_rect.contains_rect(sidebar_layout.save_status));
            assert!(
                sidebar_rect.bottom() - sidebar_layout.save_status.bottom() <= 24.0,
                "autosave footer was not bottom-aligned: sidebar={sidebar_rect:?}, save={:?}",
                sidebar_layout.save_status
            );
        }
    }

    #[test]
    fn settings_sidebar_navigation_uses_neutral_gray_colors() {
        assert_eq!(SETTINGS_SIDEBAR_FILL, Color32::from_rgb(20, 22, 27));
        assert_eq!(SETTINGS_NAV_HOVERED, Color32::from_rgb(32, 35, 41));
        assert_eq!(SETTINGS_NAV_SELECTED, Color32::from_rgb(45, 48, 55));
        assert_eq!(SETTINGS_NAV_INDICATOR, Color32::from_rgb(151, 157, 168));
    }

    #[cfg(not(windows))]
    #[test]
    fn ui_preview_cli_selects_each_settings_page() {
        for tab in SettingsTab::ALL {
            let args = vec![
                "StageSwap".to_owned(),
                "--ui-preview".to_owned(),
                tab.preview_name().to_owned(),
            ];
            assert_eq!(
                parse_ui_preview_request(&args).unwrap(),
                Some(UiPreviewRequest {
                    target: UiPreviewTarget::Settings(tab),
                })
            );
        }
        let default_page = vec!["StageSwap".to_owned(), "--ui-preview".to_owned()];
        assert_eq!(
            parse_ui_preview_request(&default_page).unwrap(),
            Some(UiPreviewRequest {
                target: UiPreviewTarget::Settings(SettingsTab::General),
            })
        );
        for (name, kind) in [
            ("dialog-exit", AppDialogKind::Exit),
            ("dialog-clear-logs", AppDialogKind::ClearLogs),
            ("dialog-admin", AppDialogKind::Admin),
            (
                "dialog-replace-baseline",
                AppDialogKind::ReplaceAdminBaseline,
            ),
            ("dialog-load-admin-config", AppDialogKind::LoadAdminConfig),
            ("dialog-remove-baseline", AppDialogKind::RemoveAdminBaseline),
        ] {
            let args = vec![
                "StageSwap".to_owned(),
                "--ui-preview".to_owned(),
                name.to_owned(),
            ];
            assert_eq!(
                parse_ui_preview_request(&args).unwrap(),
                Some(UiPreviewRequest {
                    target: UiPreviewTarget::Dialog(kind),
                })
            );
        }
        assert!(
            parse_ui_preview_request(&["StageSwap".to_owned()])
                .unwrap()
                .is_none()
        );
        assert!(
            parse_ui_preview_request(&[
                "StageSwap".to_owned(),
                "--ui-preview".to_owned(),
                "unknown".to_owned(),
            ])
            .is_err()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn ui_preview_snapshot_has_realistic_ready_inputs_and_frames() {
        let snapshot = ui_preview_snapshot();
        assert_eq!(snapshot.run_state, RunState::Running);
        assert_eq!(snapshot.mode, OutputMode::Automatic);
        assert_eq!(snapshot.detection, DetectionState::Matching);
        assert_eq!(snapshot.webcam_state, DeviceState::Ready);
        assert_eq!(snapshot.screen_state, DeviceState::Ready);
        assert_eq!(snapshot.virtual_camera_state, DeviceState::Ready);
        assert!(snapshot.previews.webcam.is_some());
        assert!(snapshot.previews.screen.is_some());
        assert!(snapshot.previews.reference.is_some());
        assert!(snapshot.previews.final_output.is_some());
        assert_eq!(snapshot.video_devices.len(), 2);
        assert_eq!(snapshot.monitors.len(), 2);
    }

    #[cfg(not(windows))]
    #[test]
    fn ui_preview_disco_state_stays_enabled_until_toggled_off() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
        )
        .with_ui_preview(UiPreviewRequest {
            target: UiPreviewTarget::Settings(SettingsTab::Diagnostics),
        });

        assert!(!app.snapshot().disco_enabled);
        app.toggle_disco();
        assert!(app.snapshot().disco_enabled);
        assert!(app.snapshot().disco_enabled);
        app.toggle_disco();
        assert!(!app.snapshot().disco_enabled);
    }

    #[test]
    fn screen_and_matching_are_separate_settings_pages() {
        assert_eq!(SettingsTab::Screen.title(), "Screen");
        assert_eq!(SettingsTab::Matching.title(), "Matching");
        assert!(SettingsTab::Screen.description().contains("display"));
        assert!(SettingsTab::Matching.description().contains("webcam"));
        assert_ne!(SettingsTab::Screen, SettingsTab::Matching);
    }

    #[test]
    fn settings_switches_use_fixed_aligned_geometry_and_blue_slate_states() {
        let first = settings_switch_geometry(Rect::from_min_size(
            Pos2::new(10.0, 20.0),
            egui::vec2(500.0, 52.0),
        ));
        let second = settings_switch_geometry(Rect::from_min_size(
            Pos2::new(10.0, 80.0),
            egui::vec2(500.0, 52.0),
        ));
        assert_eq!(first.track.size(), egui::vec2(42.0, 22.0));
        assert_eq!(first.track.right(), second.track.right());
        assert_eq!(first.state.right(), second.state.right());
        assert_eq!(
            settings_switch_colors(),
            (SETTINGS_SWITCH_OFF, SETTINGS_BLUE)
        );
    }

    #[test]
    fn conditional_settings_text_wraps_clear_of_the_switch() {
        let context = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 220.0))),
            ..egui::RawInput::default()
        };
        let mut layout = None;
        let _ = context.run_ui(input, |ui| {
            ui.set_width(300.0);
            let mut enabled = true;
            let result = automatic_screen_recovery_result(enabled);
            layout = Some(settings_conditional_toggle_row(
                ui,
                &mut enabled,
                "Recover black screen capture automatically",
                result,
            ));
        });
        let layout = layout.unwrap();
        assert!(layout.row.height() > 52.0);
        assert!(layout.description.is_none());
        let result = layout.result.expect("conditional result is rendered");
        assert!(result.height() > 12.0);
        assert!(layout.row.contains_rect(result));
        assert!(result.right() < layout.state.left());
        assert!(!result.intersects(layout.state));
        assert!(!result.intersects(layout.track));
    }

    #[test]
    fn conditional_settings_copy_reflects_values_and_dependencies() {
        let automatic = start_automatically_result(true, true, OutputMode::ForceScreen);
        assert!(automatic.starts_with("On —"));
        assert!(automatic.contains("Display mode in the tray"));
        let stopped = start_automatically_result(false, false, OutputMode::Automatic);
        assert!(stopped.starts_with("Off —"));
        assert!(stopped.contains("off screen"));

        assert!(window_behavior_result(true, true).contains("asks for confirmation"));
        assert!(window_behavior_result(true, false).contains("stops StageSwap immediately"));
        assert!(window_behavior_result(false, true).contains("asks before"));
        assert!(window_behavior_result(false, false).contains("stops StageSwap immediately"));

        let discovery_on = automatic_display_discovery_result(true);
        assert!(discovery_on.starts_with("On —"));
        assert!(discovery_on.contains("every 30 seconds"));
        assert!(discovery_on.contains("twice"));
        let discovery_off = automatic_display_discovery_result(false);
        assert!(discovery_off.starts_with("Off —"));
        assert!(discovery_off.contains("Rescan displays"));

        let recovery_on = automatic_screen_recovery_result(true);
        assert!(recovery_on.starts_with("On —"));
        assert!(recovery_on.contains("two black results"));
        assert!(recovery_on.contains("Black content"));
        let recovery_off = automatic_screen_recovery_result(false);
        assert!(recovery_off.starts_with("Off —"));
        assert!(recovery_off.contains("Diagnostics"));

        assert_eq!(
            component_health_guidance(
                DeviceState::Ready,
                DeviceState::Ready,
                DeviceState::Ready,
                DetectionState::Matching,
            ),
            "Everything is ready."
        );
    }

    #[test]
    fn match_strictness_explanations_use_documented_boundaries() {
        assert_eq!(match_strictness_explanation(1.0).level, "Very strict");
        assert_eq!(match_strictness_explanation(0.99).level, "Very strict");
        assert_eq!(match_strictness_explanation(0.98).level, "Balanced");
        assert_eq!(match_strictness_explanation(0.97).level, "Balanced");
        assert_eq!(match_strictness_explanation(0.96).level, "Forgiving");
        assert_eq!(match_strictness_explanation(0.90).level, "Forgiving");
        assert_eq!(match_strictness_explanation(0.89).level, "Very forgiving");
        assert_eq!(match_strictness_explanation(0.50).level, "Very forgiving");
    }

    #[test]
    fn webcam_selector_and_refresh_share_one_bounded_row() {
        for available in [220.0, 360.0, 520.0] {
            let gap = 8.0;
            let geometry = selector_utility_geometry(available, gap);
            assert_eq!(geometry.action_width, 32.0);
            assert!(geometry.selector_width >= 80.0);
            assert!(geometry.selector_width + gap + geometry.action_width <= available + 0.01);
        }
    }

    #[test]
    fn reference_actions_share_available_width_without_overflow() {
        for available in [220.0, 360.0, 520.0] {
            let gap = 8.0;
            let geometry = reference_control_geometry(available, gap);
            assert!(geometry.action_width >= 80.0);
            assert!((geometry.action_width * 2.0 + gap - available).abs() < 0.01);
            assert_eq!(geometry.slider_width, available);
        }
    }

    #[test]
    fn match_strictness_slider_uses_the_full_control_width() {
        for width in [220.0, 360.0, 520.0] {
            let context = egui::Context::default();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 80.0))),
                ..egui::RawInput::default()
            };
            let mut slider_rect = Rect::NOTHING;
            let _ = context.run_ui(input, |ui| {
                ui.set_width(width);
                let mut value = 0.98;
                slider_rect = match_strictness_slider(ui, &mut value, width).rect;
                assert_eq!(value, 0.98);
            });
            assert!((slider_rect.width() - width).abs() < 0.01);
        }
        assert_eq!(AppConfig::default().similarity_threshold, 0.98);
    }

    #[test]
    fn settings_sections_use_whitespace_instead_of_divider_lines() {
        let context = egui::Context::default();
        let mut first = Rect::NOTHING;
        let mut second = Rect::NOTHING;
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(520.0);
            first = ui.allocate_space(egui::vec2(ui.available_width(), 30.0)).1;
            settings_section_gap(ui);
            second = ui.allocate_space(egui::vec2(ui.available_width(), 30.0)).1;
        });
        assert!(second.top() - first.bottom() >= 24.0);
    }

    #[test]
    fn five_settings_categories_keep_every_recovery_target_in_diagnostics() {
        assert_eq!(SettingsTab::ALL.len(), 5);
        assert_eq!(SettingsTab::PRIMARY.len(), 4);
        for target in [
            RestartTarget::Webcam,
            RestartTarget::ScreenCapture,
            RestartTarget::VirtualCamera,
            RestartTarget::All,
        ] {
            assert!(
                SETTINGS_RECOVERY_TARGETS
                    .iter()
                    .any(|entry| entry.3 == target)
            );
        }
    }

    #[test]
    fn automatic_route_and_refresh_icons_are_distinct() {
        assert_ne!(UiIcon::Robot, UiIcon::Route);
        assert_ne!(UiIcon::Robot, UiIcon::Refresh);
        assert_ne!(UiIcon::Route, UiIcon::Refresh);
        let counts = [UiIcon::Robot, UiIcon::Route, UiIcon::Refresh].map(|icon| {
            let context = egui::Context::default();
            context
                .run_ui(egui::RawInput::default(), |ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
                    draw_icon(ui.painter(), rect, icon, Color32::WHITE, 1.5);
                })
                .shapes
                .len()
        });
        assert!(counts.into_iter().all(|count| count > 0));
        assert_eq!(
            counts[2], 6,
            "refresh should draw two arcs and four arrowhead lines"
        );
    }

    #[test]
    fn settings_footer_stays_inside_the_dashboard_at_supported_dpi() {
        for dpi_scale in [1.0, 1.5] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = SwitcherApp::new(
                AppConfig::default(),
                Vec::new(),
                ConfigStore::new(directory.path()),
            );
            let context = egui::Context::default();
            let viewport = egui::vec2(320.0, MIN_WINDOW_HEIGHT);
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                ..egui::RawInput::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(dpi_scale);
            let snapshot = app.runtime.snapshot();
            let mut footer = Rect::NOTHING;
            let _ = context.run_ui(input, |ui| {
                footer = app
                    .controls_workspace(ui, &snapshot, viewport.x, viewport.y)
                    .footer;
            });
            assert!(footer.is_positive());
            assert!(footer.left() >= 0.0 && footer.right() <= viewport.x + 0.01);
            assert!(footer.top() >= 0.0 && footer.bottom() <= viewport.y + 0.01);
        }
    }

    #[test]
    fn dashboard_controls_have_three_separated_full_width_sections() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
        );
        let context = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 530.0))),
            ..egui::RawInput::default()
        };
        let snapshot = app.runtime.snapshot();
        let mut workspace = None;
        let mut available_width = 0.0;
        let _ = context.run_ui(input, |ui| {
            available_width = ui.available_width();
            workspace = Some(app.controls_workspace(ui, &snapshot, 320.0, 530.0));
        });
        let workspace = workspace.unwrap();
        let layout = workspace.body;

        for rect in layout.sections {
            assert!(rect.is_positive());
        }
        for rect in layout.section_headings {
            assert!(rect.is_positive());
        }
        for pair in layout.sections.windows(2) {
            assert!(pair[0].bottom() < pair[1].top());
            assert!(!pair[0].intersects(pair[1]));
        }
        assert_eq!(layout.health_indicators.len(), 5);
        for pair in layout.health_indicators.windows(2) {
            assert!(pair[0].bottom() < pair[1].top());
        }
        assert!(layout.sections[0].bottom() < layout.section_dividers[0].top());
        assert!(layout.section_dividers[0].bottom() < layout.sections[1].top());
        assert!(layout.sections[1].bottom() < layout.section_dividers[1].top());
        assert!(layout.section_dividers[1].bottom() < layout.sections[2].top());
        assert_eq!(layout.other_actions.len(), 2);
        assert!(layout.other_actions[0].bottom() < layout.other_actions[1].top());
        for action in layout.other_actions {
            assert!((action.width() - available_width).abs() < 0.01);
        }
        assert!(layout.sections[2].bottom() < workspace.footer.top());
    }

    #[test]
    fn settings_previews_are_bounded_and_responsive() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
        );
        for kind in [
            PreviewKind::Webcam,
            PreviewKind::Screen,
            PreviewKind::Reference,
        ] {
            for (width, expected_side_by_side) in [(760.0, true), (560.0, false)] {
                let context = egui::Context::default();
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 900.0))),
                    ..egui::RawInput::default()
                };
                let mut layout = None;
                let output = context.run_ui(input, |ui| {
                    ui.set_width(width);
                    layout = Some(app.settings_preview_control_row(
                        ui,
                        SettingsSection {
                            icon: UiIcon::Camera,
                            title: "Test section",
                            description: "Test section description.",
                        },
                        SettingsPreview {
                            kind,
                            frame: None,
                            label: "Preview",
                            empty_message: "Missing preview",
                            actual_output: Source::Camera,
                        },
                        |_, ui| {
                            ui.label("Controls");
                        },
                    ));
                });
                let layout = layout.unwrap();
                assert_eq!(layout.side_by_side, expected_side_by_side);
                assert!(layout.heading.is_positive());
                assert!(layout.preview.is_positive());
                assert!(layout.controls.is_positive());
                assert!(layout.preview.left() >= -0.01 && layout.preview.right() <= width + 0.01);
                assert!(layout.preview.width() <= SETTINGS_PREVIEW_WIDTH + 0.01);
                assert!(layout.preview.height() <= SETTINGS_PREVIEW_HEIGHT + 0.01);
                assert!(
                    (layout.preview.width() / layout.preview.height() - WINDOW_ASPECT_RATIO).abs()
                        < 0.01
                );
                assert!(!layout.preview.intersects(layout.controls));
                if expected_side_by_side {
                    assert!(layout.controls.left() > layout.preview.right());
                    assert!(layout.controls.contains_rect(layout.heading));
                } else {
                    assert!(layout.heading.bottom() < layout.preview.top());
                    assert!(layout.controls.top() > layout.preview.bottom());
                }
                assert!(!output.shapes.is_empty());
            }
        }
    }

    #[test]
    fn clearing_logs_requires_explicit_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
        );
        app.log.write("info", "test", "KEEP", "keep this entry");
        let before = directory.path().join("before.jsonl");

        app.confirm_log_clear();
        app.log.export_to(&before).unwrap();
        assert!(std::fs::read_to_string(&before).unwrap().contains("KEEP"));

        app.request_log_clear();
        assert!(app.dialog_is(AppDialogKind::ClearLogs));
        app.dismiss_dialog();
        assert!(app.active_dialog.is_none());
        let after_cancel = directory.path().join("after-cancel.jsonl");
        app.log.export_to(&after_cancel).unwrap();
        assert!(
            std::fs::read_to_string(&after_cancel)
                .unwrap()
                .contains("KEEP")
        );

        app.request_log_clear();
        app.confirm_log_clear();
        assert!(app.active_dialog.is_none());
        let after_clear = directory.path().join("after-clear.jsonl");
        app.log.export_to(&after_clear).unwrap();
        let contents = std::fs::read_to_string(&after_clear).unwrap();
        assert!(!contents.contains("KEEP"));
        assert!(contents.contains("LOGS_CLEARED"));
    }

    #[test]
    fn polished_dialogs_render_responsively_and_escape_cancels() {
        for kind in [
            AppDialogKind::Exit,
            AppDialogKind::ClearLogs,
            AppDialogKind::Admin,
            AppDialogKind::ReplaceAdminBaseline,
            AppDialogKind::LoadAdminConfig,
            AppDialogKind::RemoveAdminBaseline,
        ] {
            for (viewport, dpi_scale) in [
                (egui::vec2(820.0, 600.0), 1.0),
                (egui::vec2(820.0, 600.0), 1.5),
                (egui::vec2(1280.0, 720.0), 1.5),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = SwitcherApp::new(
                    AppConfig::default(),
                    Vec::new(),
                    ConfigStore::new(directory.path()),
                );
                app.admin_profile_status = Some(AdminProfileStatus {
                    auto_restore_on_launch: true,
                    reference_included: true,
                });
                app.active_dialog = Some(ActiveDialog {
                    kind,
                    opened_at: Instant::now() - DIALOG_ENTRANCE_DURATION,
                    focus_safe_action: true,
                });
                let context = egui::Context::default();
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                    ..egui::RawInput::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(dpi_scale);
                let output = context.run_ui(input, |_ui| app.dialog(&context));
                assert!(
                    !output.shapes.is_empty(),
                    "{kind:?} did not render its backdrop, surface, and controls"
                );
                assert!(app.dialog_is(kind));
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
        );
        app.open_dialog(AppDialogKind::RemoveAdminBaseline);
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |_ui| app.dialog(&context));
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..egui::RawInput::default()
        };
        let _ = context.run_ui(input, |_ui| app.dialog(&context));
        assert!(app.active_dialog.is_none());
    }

    #[test]
    fn dialog_actions_fill_the_available_width() {
        for (available, gap, count) in [(352.0, 10.0, 2), (392.0, 10.0, 2), (392.0, 10.0, 3)] {
            let width = dialog_action_width(available, gap, count);
            let used = width * count as f32 + gap * (count - 1) as f32;
            assert!((used - available).abs() < 0.01);
        }
    }

    #[test]
    fn settings_save_is_debounced_and_back_flushes_pending_changes() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(directory.path());
        let mut app = SwitcherApp::new(AppConfig::default(), Vec::new(), store.clone());
        let started_at = Instant::now();
        app.config.cursor_visible = true;
        app.config.selected_video_device_id = "new-camera".into();
        app.settings_save_state = SettingsSaveState::Pending(started_at);

        assert!(!app.settings_save_due(started_at + SETTINGS_SAVE_DEBOUNCE / 2));
        assert!(app.settings_save_due(started_at + SETTINGS_SAVE_DEBOUNCE));

        app.view = AppView::Settings;
        app.close_settings();
        assert_eq!(app.view, AppView::Dashboard);
        assert!(matches!(app.settings_save_state, SettingsSaveState::Saved));
        assert!(store.load().config.cursor_visible);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if app.runtime.snapshot().selected_video_device_id == "new-camera" {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "runtime did not receive the flushed settings"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn accepted_monitor_selection_is_persisted_by_label() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(directory.path());
        let mut app = SwitcherApp::new(AppConfig::default(), Vec::new(), store.clone());
        let snapshot = AppSnapshot {
            selected_monitor: Some(stageswap_core::MonitorDescriptor {
                display_name: r"\\.\DISPLAY2".into(),
                label: "Stage Display".into(),
                ..stageswap_core::MonitorDescriptor::default()
            }),
            ..AppSnapshot::default()
        };

        app.sync_selected_monitor_preference(&snapshot);

        assert_eq!(app.config.selected_monitor_label, "Stage Display");
        assert_eq!(store.load().config.selected_monitor_label, "Stage Display");
    }

    #[test]
    fn opening_settings_does_not_rescan_when_automatic_rescans_are_disabled() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig {
                automatic_monitor_rescans: false,
                ..AppConfig::default()
            },
            Vec::new(),
            ConfigStore::new(directory.path()),
        );

        app.open_settings();

        let deadline = Instant::now() + Duration::from_secs(2);
        let snapshot = loop {
            let snapshot = app.runtime.snapshot();
            if snapshot
                .recent_activity
                .iter()
                .any(|activity| activity == "Video device list refreshed")
            {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "runtime did not process the settings refresh"
            );
            std::thread::yield_now();
        };
        assert!(
            snapshot
                .recent_activity
                .iter()
                .all(|activity| activity != "Monitor rescan requested")
        );
    }

    #[test]
    fn settings_save_failure_preserves_the_active_value() {
        let directory = tempfile::tempdir().unwrap();
        let blocked_path = directory.path().join("not-a-directory");
        std::fs::write(&blocked_path, "blocking file").unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(&blocked_path),
        );
        app.config.show_notifications = false;
        app.queue_settings_save();
        app.flush_settings();

        assert!(!app.config.show_notifications);
        assert!(matches!(
            &app.settings_save_state,
            SettingsSaveState::Failed(message) if message.contains("Could not save settings")
        ));
        assert!(
            app.load_warnings
                .iter()
                .any(|warning| warning.contains("Could not save settings"))
        );
    }

    #[test]
    fn admin_baseline_captures_pending_settings_and_toggle_is_independent() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(directory.path());
        let mut app = SwitcherApp::new(AppConfig::default(), Vec::new(), store.clone());
        assert_eq!(app.admin_profile_status, None);

        app.config.selected_video_device_id = "pending-admin-camera".into();
        app.settings_save_state = SettingsSaveState::Pending(Instant::now());
        app.save_admin_baseline();
        assert_eq!(
            app.admin_profile_status,
            Some(AdminProfileStatus {
                auto_restore_on_launch: false,
                reference_included: false,
            })
        );

        app.set_admin_auto_restore(true);
        assert!(app.admin_profile_status.unwrap().auto_restore_on_launch);
        save_config(
            &store,
            &AppConfig {
                selected_video_device_id: "temporary-user-camera".into(),
                reference_image_path: store.reference_path().display().to_string(),
                ..AppConfig::default()
            },
        )
        .unwrap();
        assert_eq!(
            restore_admin_profile(&AdminProfileStore::new(directory.path())).unwrap(),
            AdminRestoreOutcome::Restored
        );
        assert_eq!(
            store.load().config.selected_video_device_id,
            "pending-admin-camera"
        );

        app.set_admin_auto_restore(false);
        app.config.selected_video_device_id = "replacement-admin-camera".into();
        app.save_admin_baseline();
        assert!(!app.admin_profile_status.unwrap().auto_restore_on_launch);
        app.remove_admin_baseline();
        assert_eq!(app.admin_profile_status, None);
        assert_eq!(
            AdminProfileStore::new(directory.path()).status().unwrap(),
            None
        );
    }

    #[test]
    fn manual_admin_load_updates_working_config_runtime_and_reference() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(directory.path());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(store.reference_path())
            .unwrap();
        let mut app = SwitcherApp::new(AppConfig::default(), Vec::new(), store.clone());
        app.view = AppView::Settings;
        let admin = AppConfig {
            selected_video_device_id: "admin-camera".into(),
            selected_monitor_label: "Admin display".into(),
            similarity_threshold: 0.91,
            cursor_visible: true,
            output_mode: OutputMode::ForceScreen,
            reference_image_path: store.reference_path().display().to_string(),
            ..AppConfig::default()
        };
        app.config = admin.clone();
        app.save_admin_baseline();
        assert!(!app.admin_profile_status.unwrap().auto_restore_on_launch);

        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255]))
            .save(store.reference_path())
            .unwrap();
        let user = AppConfig {
            selected_video_device_id: "user-camera".into(),
            reference_image_path: store.reference_path().display().to_string(),
            ..AppConfig::default()
        };
        save_config(&store, &user).unwrap();
        app.config = user;
        app.settings_save_state = SettingsSaveState::Pending(Instant::now());
        app.open_dialog(AppDialogKind::LoadAdminConfig);

        app.load_admin_config();

        assert_eq!(app.view, AppView::Settings);
        assert!(app.dialog_is(AppDialogKind::Admin));
        assert_eq!(app.config, admin);
        assert_eq!(store.load().config, admin);
        assert!(matches!(app.settings_save_state, SettingsSaveState::Saved));
        assert!(!app.admin_profile_status.unwrap().auto_restore_on_launch);

        let deadline = Instant::now() + Duration::from_secs(2);
        let snapshot = loop {
            let snapshot = app.runtime.snapshot();
            if snapshot.selected_video_device_id == "admin-camera"
                && snapshot.mode == OutputMode::ForceScreen
                && snapshot.previews.reference.is_some()
                && snapshot
                    .recent_activity
                    .iter()
                    .any(|activity| activity == "Monitor rescan requested")
            {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "runtime did not apply the manually loaded admin config"
            );
            std::thread::yield_now();
        };
        assert_eq!(
            &snapshot.previews.reference.unwrap().pixels()[..4],
            &[0, 0, 255, 255]
        );
    }

    #[test]
    fn startup_admin_restore_failures_preserve_the_working_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(directory.path());
        let user = AppConfig {
            selected_video_device_id: "user-camera".into(),
            ..AppConfig::default()
        };
        save_config(&store, &user).unwrap();
        std::fs::write(
            AdminProfileStore::new(directory.path()).profile_path(),
            "invalid admin profile",
        )
        .unwrap();

        let loaded = load_config_with_admin_restore(&store);
        assert_eq!(loaded.config, user);
        assert!(
            loaded
                .warnings
                .iter()
                .any(|warning| warning.contains("Could not restore the admin configuration"))
        );
    }

    #[test]
    fn only_a_secondary_double_click_activates_the_admin_logo() {
        fn click_input(time: f64, button: egui::PointerButton) -> egui::RawInput {
            let position = Pos2::new(20.0, 20.0);
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(140.0, 140.0))),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: position,
                        button,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..egui::RawInput::default()
            }
        }

        fn render_logo(context: &egui::Context, input: egui::RawInput) -> bool {
            let mut activated = false;
            let _ = context.run_ui(input, |ui| {
                let (_, response) = ui.allocate_exact_size(egui::vec2(96.0, 96.0), Sense::click());
                activated = admin_logo_activated(&response);
            });
            activated
        }

        let secondary = egui::Context::default();
        assert!(!render_logo(
            &secondary,
            click_input(0.0, egui::PointerButton::Secondary)
        ));
        assert!(render_logo(
            &secondary,
            click_input(0.1, egui::PointerButton::Secondary)
        ));

        let primary = egui::Context::default();
        assert!(!render_logo(
            &primary,
            click_input(0.0, egui::PointerButton::Primary)
        ));
        assert!(!render_logo(
            &primary,
            click_input(0.1, egui::PointerButton::Primary)
        ));
    }

    #[test]
    fn five_primary_diagnostics_clicks_toggle_disco_without_mixed_sequences() {
        let start = Instant::now();
        let mut gesture = DiscoDiagnosticsGesture::default();
        for click in 0..4 {
            assert!(!gesture.register_primary_click(start + Duration::from_millis(click * 200)));
        }
        assert!(gesture.register_primary_click(start + Duration::from_millis(800)));

        let mut slow_gesture = DiscoDiagnosticsGesture::default();
        for click in 0..5 {
            assert!(!slow_gesture.register_primary_click(
                start + Duration::from_secs(3) + Duration::from_millis(click * 800)
            ));
        }

        for click in 0..4 {
            assert!(!gesture.register_primary_click(
                start + Duration::from_secs(4) + Duration::from_millis(click * 100)
            ));
        }
        // A click on another control resets the hidden Diagnostics sequence.
        gesture.reset();
        assert!(!gesture.register_primary_click(start + Duration::from_secs(5)));
    }

    #[test]
    fn dashboard_and_full_window_settings_render_responsively() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig {
                start_automatically: false,
                ..AppConfig::default()
            },
            vec!["Test warning banner".into()],
            ConfigStore::new(directory.path()),
        );
        let context = egui::Context::default();
        for viewport in [
            egui::vec2(820.0, 600.0),
            egui::vec2(MIN_WINDOW_HEIGHT * WINDOW_ASPECT_RATIO, MIN_WINDOW_HEIGHT),
            egui::vec2(1280.0, 720.0),
        ] {
            for dpi_scale in [1.0, 1.5] {
                app.view = AppView::Dashboard;
                let mut dashboard_input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                    ..egui::RawInput::default()
                };
                dashboard_input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(dpi_scale);
                let dashboard_output = context.run_ui(dashboard_input, |ui| {
                    app.root_ui(ui);
                });
                assert!(!dashboard_output.shapes.is_empty());

                for tab in SettingsTab::ALL {
                    app.settings_tab = tab;
                    app.view = AppView::Settings;
                    app.settings_opened_at = None;
                    app.settings_section_changed_at = None;
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                        ..egui::RawInput::default()
                    };
                    input
                        .viewports
                        .get_mut(&egui::ViewportId::ROOT)
                        .unwrap()
                        .native_pixels_per_point = Some(dpi_scale);
                    let output = context.run_ui(input, |ui| {
                        assert_eq!(ui.ctx().native_pixels_per_point(), Some(dpi_scale));
                        let content_rect = app.root_ui(ui);
                        assert!(
                            content_rect.min.x >= 8.0 && content_rect.min.y >= 8.0,
                            "settings starts outside the root panel margin: {content_rect:?}"
                        );
                        assert!(
                            content_rect.max.x <= viewport.x - 8.0
                                && content_rect.max.y <= viewport.y - 8.0,
                            "settings ends outside the root panel margin: {content_rect:?}"
                        );
                    });
                    assert!(
                        !output.shapes.is_empty(),
                        "{tab:?} at {viewport:?} and {dpi_scale}× produced no UI shapes"
                    );
                }
            }
        }
    }
}
