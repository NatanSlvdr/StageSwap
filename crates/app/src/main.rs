#![cfg_attr(windows, windows_subsystem = "windows")]

use asc_app::RuntimeHandle;
use asc_core::{
    AppConfig, AppSnapshot, Command, ConfigStore, DetectionState, DeviceState, Frame, OutputMode,
    RestartTarget, RunState, Source,
};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const WINDOW_ASPECT_RATIO: f32 = 16.0 / 9.0;
const MIN_WINDOW_HEIGHT: f32 = 600.0;
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
const SETTINGS_SIDEBAR_WIDTH: f32 = 196.0;
const SETTINGS_CONTENT_WIDTH: f32 = 960.0;
const SETTINGS_PREVIEW_WIDTH: f32 = 480.0;
const SETTINGS_PREVIEW_HEIGHT: f32 = 270.0;
const SETTINGS_PREVIEW_COLUMNS_BREAKPOINT: f32 = 700.0;

mod local_log;
mod portable_payload;
#[cfg(windows)]
mod tray;
use local_log::LocalLog;

fn main() -> eframe::Result {
    let _embedded_payload = portable_payload::bytes();
    #[cfg(windows)]
    match asc_windows::portable_startup(_embedded_payload) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(error) => {
            asc_windows::show_error_dialog(&format!(
                "Automatic Screen Camera deployment failed:\n\n{error}"
            ));
            eprintln!("Automatic Screen Camera deployment failed: {error}");
            std::process::exit(1);
        }
    }
    #[cfg(windows)]
    let _single_instance = match asc_windows::SingleInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            asc_windows::show_error_dialog(
                "Automatic Screen Camera is already running. Open it from the system tray, or exit the tray application before launching this copy.",
            );
            return Ok(());
        }
        Err(error) => {
            asc_windows::show_error_dialog(&error);
            return Ok(());
        }
    };
    let store = ConfigStore::new(local_data_directory());
    let loaded = store.load();
    let start_visible = !loaded.config.start_minimized;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Automatic Screen Camera")
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([MIN_WINDOW_HEIGHT * WINDOW_ASPECT_RATIO, MIN_WINDOW_HEIGHT])
            .with_visible(start_visible),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Automatic Screen Camera",
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
            #[cfg(windows)]
            if !start_visible && app.tray.is_none() {
                context
                    .egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            Ok(Box::new(app))
        }),
    )
}

fn local_data_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
        .join("AutomaticScreenCameraRust")
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
    App,
    Webcam,
    ScreenDetection,
    Diagnostics,
}

impl SettingsTab {
    const ALL: [(Self, UiIcon, &'static str); 4] = [
        (Self::App, UiIcon::Settings, "App"),
        (Self::Webcam, UiIcon::Camera, "Webcam"),
        (Self::ScreenDetection, UiIcon::Monitor, "Screen & detection"),
        (Self::Diagnostics, UiIcon::Layers, "Diagnostics"),
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::App => "App",
            Self::Webcam => "Webcam",
            Self::ScreenDetection => "Screen & detection",
            Self::Diagnostics => "Diagnostics",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::App => "Choose how the app launches, stays available, and reports problems.",
            Self::Webcam => "Select, verify, and recover the camera used for webcam output.",
            Self::ScreenDetection => {
                "Choose what to capture and teach Automatic mode when to show the webcam."
            }
            Self::Diagnostics => {
                "Inspect component health, technical details, logs, and recovery tools."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AppView {
    #[default]
    Dashboard,
    Settings,
}

#[derive(Clone, Debug, Default)]
enum SettingsSaveState {
    #[default]
    Saved,
    Pending(Instant),
    Failed(String),
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
            .name(format!("asc-preview-{}", kind.key()))
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

struct SwitcherApp {
    config: AppConfig,
    runtime: RuntimeHandle,
    store: ConfigStore,
    load_warnings: Vec<String>,
    view: AppView,
    settings_tab: SettingsTab,
    settings_save_state: SettingsSaveState,
    confirm_clear_logs: bool,
    awaiting_video_device_id: Option<String>,
    settings_opened_at: Option<Instant>,
    settings_section_changed_at: Option<Instant>,
    textures: HashMap<&'static str, PreviewTexture>,
    preview_converters: HashMap<PreviewKind, PreviewConverter>,
    log: LocalLog,
    last_activity: Option<String>,
    #[cfg(windows)]
    last_notified_warning: Option<String>,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    exit_requested: bool,
    show_exit_confirmation: bool,
    last_window_size: Option<Vec2>,
}

impl SwitcherApp {
    fn new(mut config: AppConfig, load_warnings: Vec<String>, store: ConfigStore) -> Self {
        if config.reference_image_path.is_empty() {
            config.reference_image_path = store.reference_path().display().to_string();
        }
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
            settings_tab: SettingsTab::App,
            settings_save_state: SettingsSaveState::Saved,
            confirm_clear_logs: false,
            awaiting_video_device_id: None,
            settings_opened_at: None,
            settings_section_changed_at: None,
            textures: HashMap::new(),
            preview_converters: HashMap::new(),
            log,
            last_activity: None,
            #[cfg(windows)]
            last_notified_warning: None,
            #[cfg(windows)]
            tray: tray::Tray::new().ok(),
            exit_requested: false,
            show_exit_confirmation: false,
            last_window_size: None,
        }
    }

    fn send(&self, command: Command) {
        let _ = self.runtime.send(command);
    }

    fn set_mode(&mut self, mode: OutputMode) {
        self.config.output_mode = mode;
        self.send(Command::SetMode(mode));
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

    fn open_settings(&mut self) {
        self.view = AppView::Settings;
        self.settings_opened_at = Some(Instant::now());
        self.settings_section_changed_at = Some(Instant::now());
        self.send(Command::RefreshVideoDevices);
        self.send(Command::Rescan);
    }

    fn close_settings(&mut self) {
        self.flush_settings();
        self.view = AppView::Dashboard;
        self.confirm_clear_logs = false;
        self.settings_opened_at = None;
        self.settings_section_changed_at = None;
    }

    fn import_reference_dialog(&mut self) {
        #[cfg(windows)]
        if let Some(path) = asc_windows::pick_reference_image() {
            self.send(Command::ImportReference(path));
        }
        #[cfg(not(windows))]
        self.load_warnings
            .push("Reference file dialogs are available in the Windows application".into());
    }

    fn open_log_directory(&mut self) {
        #[cfg(windows)]
        if let Err(error) = asc_windows::open_directory(self.log.directory()) {
            self.load_warnings.push(error);
        }
        #[cfg(not(windows))]
        self.load_warnings
            .push(format!("Log directory: {}", self.log.directory().display()));
    }

    fn export_logs(&mut self) {
        #[cfg(windows)]
        if let Some(path) = asc_windows::pick_log_export_path()
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
        self.confirm_clear_logs = true;
    }

    fn cancel_log_clear(&mut self) {
        self.confirm_clear_logs = false;
    }

    fn confirm_log_clear(&mut self) {
        if self.confirm_clear_logs {
            self.clear_logs();
            self.confirm_clear_logs = false;
        }
    }

    fn root_ui(&mut self, ui: &mut egui::Ui) -> Rect {
        let context = ui.ctx().clone();
        let content_rect = egui::CentralPanel::default()
            .show(ui, |ui| {
                let content_rect = ui.max_rect();
                match self.view {
                    AppView::Dashboard => self.dashboard(ui),
                    AppView::Settings => self.settings_view(&context, ui),
                }
                content_rect
            })
            .inner;
        self.exit_confirmation(&context);
        content_rect
    }

    fn dashboard(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.runtime.snapshot();
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
            ui.allocate_ui_with_layout(
                egui::vec2(controls_width, workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.controls_workspace(ui, &snapshot, controls_width, workspace_height),
            );
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
        ui.allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::bottom_up(egui::Align::Center),
            |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, FOOTER_HEIGHT),
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        icon_text(
                            ui,
                            UiIcon::Camera,
                            "Automatic Screen Camera",
                            Color32::from_rgb(205, 211, 222),
                            true,
                        );
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
    ) {
        const FOOTER_HEIGHT: f32 = 40.0;
        const FOOTER_GAP: f32 = 10.0;
        let body_height = (height - FOOTER_HEIGHT - FOOTER_GAP).max(80.0);
        ui.allocate_ui_with_layout(
            egui::vec2(width, body_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_size(egui::vec2(width, body_height));
                self.controls_body(ui, snapshot);
            },
        );
        ui.add_space(FOOTER_GAP);
        if icon_button(
            ui,
            UiIcon::Settings,
            "Settings",
            egui::vec2(width, FOOTER_HEIGHT),
            false,
            true,
        )
        .clicked()
        {
            self.open_settings();
        }
    }

    fn controls_body(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        health_state_group(ui, UiIcon::Camera, "Webcam", snapshot.webcam_state);
        health_state_group(ui, UiIcon::Monitor, "Screen", snapshot.screen_state);
        health_state_group(
            ui,
            UiIcon::Broadcast,
            "Output",
            snapshot.virtual_camera_state,
        );
        ui.separator();
        detection_state_group(ui, snapshot.detection);
        screen_mix_group(ui, snapshot.transition.screen_mix);
        ui.add_space(12.0);

        let automation_running =
            matches!(snapshot.run_state, RunState::Running | RunState::Starting);
        let (run_icon, run_label, run_accent) = if automation_running {
            (UiIcon::Stop, "Stop automation", LIVE_RED)
        } else {
            (UiIcon::Play, "Start automation", ACTIVE_GREEN)
        };
        if accent_icon_button(
            ui,
            run_icon,
            run_label,
            egui::vec2(ui.available_width(), 36.0),
            run_accent,
        )
        .clicked()
        {
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
        ui.scope(|ui| {
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
            });
        });

        ui.add_space(8.0);
        let gap = ui.spacing().item_spacing.x;
        if ui.available_width() >= 285.0 {
            let action_width = (ui.available_width() - gap) / 2.0;
            ui.horizontal(|ui| {
                if icon_button(
                    ui,
                    UiIcon::Capture,
                    "Capture reference",
                    egui::vec2(action_width, 32.0),
                    false,
                    false,
                )
                .clicked()
                {
                    self.send(Command::CaptureReference);
                }
                if icon_button(
                    ui,
                    UiIcon::Refresh,
                    "Rescan screens",
                    egui::vec2(action_width, 32.0),
                    false,
                    false,
                )
                .clicked()
                {
                    self.send(Command::Rescan);
                }
            });
        } else {
            if icon_button(
                ui,
                UiIcon::Capture,
                "Capture reference",
                egui::vec2(ui.available_width(), 32.0),
                false,
                false,
            )
            .clicked()
            {
                self.send(Command::CaptureReference);
            }
            if icon_button(
                ui,
                UiIcon::Refresh,
                "Rescan screens",
                egui::vec2(ui.available_width(), 32.0),
                false,
                false,
            )
            .clicked()
            {
                self.send(Command::Rescan);
            }
        }
    }

    fn settings_view(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        if !self.show_exit_confirmation
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
            ui.add_space((1.0 - entrance) * 7.0);
            if self.settings_header(ui) {
                return;
            }
            ui.separator();
            self.settings_workspace(ui);
        });
    }

    fn settings_header(&mut self, ui: &mut egui::Ui) -> bool {
        let mut go_back = false;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 54.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let back = icon_button(ui, UiIcon::Back, "", egui::vec2(34.0, 34.0), false, false)
                    .on_hover_text("Back to dashboard");
                if back.clicked() {
                    go_back = true;
                }
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Settings")
                            .size(22.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.label(
                        RichText::new("Automatic Screen Camera")
                            .size(11.0)
                            .color(Color32::from_rgb(133, 140, 153)),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                });
            },
        );
        if go_back {
            self.close_settings();
        }
        go_back
    }

    fn settings_workspace(&mut self, ui: &mut egui::Ui) {
        let workspace_height = ui.available_height().max(0.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(SETTINGS_SIDEBAR_WIDTH, workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.settings_sidebar(ui, workspace_height);
                },
            );

            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.settings_content(ui),
            );
        });
    }

    fn settings_sidebar(&mut self, ui: &mut egui::Ui, height: f32) -> Vec<Rect> {
        egui::Frame::new()
            .fill(Color32::from_rgb(20, 22, 27))
            .inner_margin(egui::Margin::symmetric(10, 12))
            .show(ui, |ui| {
                ui.set_width(SETTINGS_SIDEBAR_WIDTH - 20.0);
                ui.set_min_height((height - 24.0).max(0.0));
                ui.label(
                    RichText::new("PREFERENCES")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(112, 120, 134)),
                );
                ui.add_space(8.0);
                let mut navigation_rects = Vec::with_capacity(SettingsTab::ALL.len());
                for (tab, icon, label) in SettingsTab::ALL {
                    let response = settings_nav_button(ui, tab, icon, label, self.settings_tab);
                    navigation_rects.push(response.rect);
                    if response.clicked() && self.settings_tab != tab {
                        self.settings_tab = tab;
                        self.confirm_clear_logs = false;
                        self.settings_section_changed_at = Some(Instant::now());
                    }
                    ui.add_space(3.0);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new("Changes save automatically")
                            .size(10.5)
                            .color(Color32::from_rgb(112, 120, 134)),
                    );
                });
                navigation_rects
            })
            .inner
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
                                    SettingsTab::App => self.app_settings(ui),
                                    SettingsTab::Webcam => self.webcam_settings(ui),
                                    SettingsTab::ScreenDetection => {
                                        self.screen_detection_settings(ui)
                                    }
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

    fn app_settings(&mut self, ui: &mut egui::Ui) {
        settings_section_heading(
            ui,
            UiIcon::Play,
            "Startup",
            "These choices take effect the next time Windows or the app starts.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.start_with_windows,
            "Start with Windows",
            "Launch automatically after you sign in to Windows.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.start_minimized,
            "Start minimized",
            "Open in the system tray instead of showing the main window.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.start_automatically,
            "Start automation on launch",
            "Begin monitoring and switching after the app is ready.",
        );

        settings_section_divider(ui);
        settings_section_heading(
            ui,
            UiIcon::Window,
            "Window behavior",
            "Control what happens when you close or fully exit the app.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.close_to_tray,
            "Close window to tray",
            "Keep capture and virtual-camera output running after closing the window.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.confirm_exit,
            "Confirm before exit",
            "Ask before stopping the app and its active capture pipeline.",
        );

        settings_section_divider(ui);
        settings_section_heading(
            ui,
            UiIcon::Bell,
            "Notifications",
            "Choose whether Windows should surface important app warnings.",
        );
        settings_toggle_row(
            ui,
            &mut self.config.show_notifications,
            "Show status notifications",
            "Display a Windows notification when a component needs attention.",
        );
    }

    fn webcam_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.runtime.snapshot();
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
                description: "Preview and choose the camera used whenever webcam output is active.",
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
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Selection applies automatically.")
                        .size(10.5)
                        .color(Color32::from_rgb(126, 134, 148)),
                );
            },
        );
    }

    fn screen_detection_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.runtime.snapshot();
        let selected_monitor = snapshot
            .selected_monitor
            .as_ref()
            .map_or("No display selected", |monitor| monitor.label.as_str());
        self.settings_preview_control_row(
            ui,
            SettingsSection {
                icon: UiIcon::Monitor,
                title: "Screen capture",
                description: "Preview and choose the display Automatic mode watches.",
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
                ui.add_space(8.0);
                settings_toggle_row(
                    ui,
                    &mut app.config.cursor_visible,
                    "Include mouse cursor",
                    "Also add it to new references.",
                );
            },
        );

        settings_section_divider(ui);
        self.settings_preview_control_row(
            ui,
            SettingsSection {
                icon: UiIcon::Target,
                title: "Reference and matching",
                description:
                    "Automatic mode shows the webcam while the screen resembles this reference.",
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
                    ui.separator();
                    settings_detection_status(ui, snapshot.detection);
                });
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
                ui.label(
                    RichText::new("Higher values require a closer visual match.")
                        .size(10.5)
                        .color(Color32::from_rgb(126, 134, 148)),
                );
                let geometry = reference_control_geometry(
                    ui.available_width(),
                    ui.spacing().item_spacing.x,
                );
                ui.add_sized(
                    [geometry.slider_width, 24.0],
                    egui::Slider::new(&mut app.config.similarity_threshold, 0.50..=1.0)
                        .show_value(false),
                );
            },
        );
    }

    fn diagnostics_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.runtime.snapshot();
        settings_section_heading(
            ui,
            UiIcon::Check,
            "Component health",
            "Use these states to identify which part of the pipeline needs attention.",
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

        settings_section_divider(ui);
        settings_section_heading(
            ui,
            UiIcon::Wrench,
            "Recovery",
            "Reconnect one component, rescan displays, or restart the complete pipeline.",
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

        settings_section_divider(ui);
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

        settings_section_divider(ui);
        settings_section_heading(
            ui,
            UiIcon::Folder,
            "Storage and logs",
            "Configuration, reference images, and logs remain on this computer.",
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
                    if self.confirm_clear_logs {
                        if ui.button("Cancel").clicked() {
                            self.cancel_log_clear();
                        }
                        if ui
                            .button(
                                RichText::new("Clear logs").color(Color32::from_rgb(244, 133, 133)),
                            )
                            .clicked()
                        {
                            self.confirm_log_clear();
                        }
                    } else {
                        if ui.button("Open folder").clicked() {
                            self.open_log_directory();
                        }
                        if ui.button("Export…").clicked() {
                            self.export_logs();
                        }
                        if ui.button("Clear…").clicked() {
                            self.request_log_clear();
                        }
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

    fn exit_confirmation(&mut self, context: &egui::Context) {
        if !self.show_exit_confirmation {
            return;
        }
        egui::Window::new("Exit Automatic Screen Camera?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label("The virtual camera will remain registered and show the crossed-camera off screen when the publisher stops.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.show_exit_confirmation = false;
                    }
                    if ui.button("Exit").clicked() {
                        self.exit_requested = true;
                        self.show_exit_confirmation = false;
                        context.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
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
        let active_amount = ui.ctx().animate_bool_with_time(
            ui.make_persistent_id((kind.key(), "active-contour")),
            contour == PreviewContour::Active,
            0.14,
        );
        let contour_color = match contour {
            PreviewContour::Live => LIVE_RED,
            PreviewContour::Active | PreviewContour::Neutral => {
                mix_color(PREVIEW_NEUTRAL, ACTIVE_GREEN, active_amount)
            }
        };
        let contour_width = 3.0;
        let preview_frame = egui::Frame::new()
            .fill(Color32::from_rgb(12, 14, 18))
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

impl eframe::App for SwitcherApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = self.runtime.snapshot();
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
            asc_windows::notify_warning(warning.clone());
            self.last_notified_warning = Some(warning.clone());
        }
        #[cfg(windows)]
        if snapshot.warning.is_none() {
            self.last_notified_warning = None;
        }
        #[cfg(windows)]
        if let Some(action) = self.tray.as_ref().and_then(tray::Tray::poll) {
            match action {
                tray::TrayAction::Show => {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayAction::Command(command) => self.send(command),
                tray::TrayAction::Exit => {
                    if self.config.confirm_exit {
                        self.show_exit_confirmation = true;
                        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
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
            self.show_exit_confirmation = true;
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
        let _ = save_config(&self.store, &self.config);
    }
}

#[cfg(windows)]
fn save_config(store: &ConfigStore, config: &AppConfig) -> std::io::Result<()> {
    asc_windows::save_config_atomic(store, config)
}

#[cfg(not(windows))]
fn save_config(store: &ConfigStore, config: &AppConfig) -> std::io::Result<()> {
    store.save(config)
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

fn settings_nav_button(
    ui: &mut egui::Ui,
    tab: SettingsTab,
    icon: UiIcon,
    label: &str,
    selected: SettingsTab,
) -> egui::Response {
    let active = tab == selected;
    let amount =
        ui.ctx()
            .animate_bool_with_time(ui.make_persistent_id(("settings-nav", tab)), active, 0.14);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    let idle = Color32::from_rgb(20, 22, 27);
    let hovered = Color32::from_rgb(31, 35, 43);
    let selected_fill = Color32::from_rgb(33, 47, 75);
    let base = if response.hovered() { hovered } else { idle };
    ui.painter()
        .rect_filled(rect, 6, mix_color(base, selected_fill, amount));
    if amount > 0.0 {
        let indicator_height = 16.0 + 10.0 * amount;
        let indicator = Rect::from_center_size(
            Pos2::new(rect.left() + 2.0, rect.center().y),
            egui::vec2(3.0, indicator_height),
        );
        ui.painter().rect_filled(
            indicator,
            2,
            ui.visuals().selection.bg_fill.gamma_multiply(amount),
        );
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

fn settings_section_divider(ui: &mut egui::Ui) -> Rect {
    ui.add_space(14.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0, Color32::from_rgb(61, 67, 78));
    ui.add_space(14.0);
    rect
}

fn settings_toggle_row(ui: &mut egui::Ui, value: &mut bool, title: &str, description: &str) {
    let (row, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), Sense::click());
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
    let title_pos = Pos2::new(row.left(), row.center().y - 7.0);
    ui.painter().text(
        title_pos,
        Align2::LEFT_BOTTOM,
        title,
        FontId::proportional(12.5),
        Color32::from_rgb(224, 228, 235),
    );
    ui.painter().text(
        Pos2::new(row.left(), row.center().y + 4.0),
        Align2::LEFT_TOP,
        description,
        FontId::proportional(10.0),
        Color32::from_rgb(126, 134, 148),
    );
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
    ui.separator();
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
    let (icon, status, color) = match state {
        DetectionState::Unknown => (UiIcon::Question, "Waiting", TRANSITION_AMBER),
        DetectionState::Matching => (UiIcon::Check, "Matching", ACTIVE_GREEN),
        DetectionState::NotMatching => (UiIcon::Error, "Not matching", LIVE_RED),
        DetectionState::ReferenceMissing => (UiIcon::Unavailable, "Reference missing", LIVE_RED),
    };
    settings_status_item(ui, UiIcon::Target, "Detection", icon, status, color);
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

fn health_state_group(ui: &mut egui::Ui, icon: UiIcon, label: &'static str, current: DeviceState) {
    let choices = HEALTH_STATES.map(|state| IndicatorChoice {
        icon: device_state_icon(state),
        label: friendly_device_state(state),
        current: state == current,
        tone: device_state_tone(state),
        span: 1,
    });
    indicator_group(ui, icon, label, &choices, None);
}

#[derive(Clone, Copy)]
struct IndicatorChoice {
    icon: UiIcon,
    label: &'static str,
    current: bool,
    tone: IndicatorTone,
    span: u8,
}

#[derive(Clone, Copy)]
enum IndicatorTone {
    Green,
    Amber,
    Red,
}

fn detection_state_group(ui: &mut egui::Ui, current: DetectionState) {
    let choices = [
        IndicatorChoice {
            icon: UiIcon::Question,
            label: "Unknown",
            current: current == DetectionState::Unknown,
            tone: IndicatorTone::Amber,
            span: 1,
        },
        IndicatorChoice {
            icon: UiIcon::Check,
            label: "Matching",
            current: current == DetectionState::Matching,
            tone: IndicatorTone::Green,
            span: 1,
        },
        IndicatorChoice {
            icon: UiIcon::Error,
            label: "Not matching",
            current: current == DetectionState::NotMatching,
            tone: IndicatorTone::Red,
            span: 1,
        },
        IndicatorChoice {
            icon: UiIcon::Unavailable,
            label: "Reference missing",
            current: current == DetectionState::ReferenceMissing,
            tone: IndicatorTone::Red,
            span: 1,
        },
    ];
    indicator_group(ui, UiIcon::Target, "Detection", &choices, None);
}

fn screen_mix_group(ui: &mut egui::Ui, screen_mix: f64) {
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
    );
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
            painter.circle_stroke(center, rect.width() * 0.35, stroke);
            painter.line_segment(
                [Pos2::new(x(0.72), y(0.12)), Pos2::new(x(0.9), y(0.14))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.9), y(0.14)), Pos2::new(x(0.84), y(0.32))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.28), y(0.88)), Pos2::new(x(0.1), y(0.86))],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x(0.1), y(0.86)), Pos2::new(x(0.16), y(0.68))],
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
    fn visible_ui_and_hidden_logic_use_distinct_repaint_cadences() {
        assert_eq!(repaint_interval(true), VISIBLE_REFRESH);
        assert_eq!(repaint_interval(false), HIDDEN_REFRESH);
        assert_eq!(VISIBLE_REFRESH, Duration::from_nanos(1_000_000_000 / 30));
        assert_eq!(HIDDEN_REFRESH, Duration::from_millis(250));
    }

    #[test]
    fn preview_conversion_preserves_size_and_bgra_channels() {
        let frame = Frame::new(
            vec![3, 2, 1, 255, 30, 20, 10, 255].into(),
            asc_core::Size::new(2, 1),
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
            asc_core::Size::new(1280, 720),
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
                    asc_core::Size::new(1280, 720),
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
            asc_core::Size::new(2, 2),
            0xff00_0001,
            1,
            0,
            now,
        ));
        let pending = Arc::new(Frame::placeholder(
            asc_core::Size::new(2, 2),
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
            let mut navigation_rects = Vec::new();
            let _ = context.run_ui(input, |ui| {
                let sidebar = ui.allocate_ui_with_layout(
                    egui::vec2(SETTINGS_SIDEBAR_WIDTH, viewport.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| app.settings_sidebar(ui, viewport.y),
                );
                sidebar_rect = sidebar.response.rect;
                navigation_rects = sidebar.inner;
            });

            assert_eq!(navigation_rects.len(), SettingsTab::ALL.len());
            for rect in &navigation_rects {
                assert!(rect.is_positive(), "invalid navigation rect: {rect:?}");
                assert!(
                    sidebar_rect.contains_rect(*rect),
                    "navigation rect escaped sidebar: {rect:?} outside {sidebar_rect:?}"
                );
                assert!((rect.height() - 36.0).abs() < 0.01);
            }
            for pair in navigation_rects.windows(2) {
                assert!(
                    pair[1].top() - pair[0].bottom() >= 2.9,
                    "navigation rows overlap: {:?} and {:?}",
                    pair[0],
                    pair[1]
                );
                assert!(!pair[0].intersects(pair[1]));
            }
        }
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
    fn settings_category_divider_is_full_width_and_between_sections() {
        let context = egui::Context::default();
        let mut first = Rect::NOTHING;
        let mut divider = Rect::NOTHING;
        let mut second = Rect::NOTHING;
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(520.0);
            first = ui.allocate_space(egui::vec2(ui.available_width(), 30.0)).1;
            divider = settings_section_divider(ui);
            second = ui.allocate_space(egui::vec2(ui.available_width(), 30.0)).1;
        });
        assert!(first.bottom() < divider.top());
        assert!(divider.bottom() < second.top());
        assert!((divider.width() - 520.0).abs() < 0.01);
    }

    #[test]
    fn four_settings_categories_keep_every_recovery_target_in_diagnostics() {
        assert_eq!(SettingsTab::ALL.len(), 4);
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
        assert!(app.confirm_clear_logs);
        app.cancel_log_clear();
        assert!(!app.confirm_clear_logs);
        let after_cancel = directory.path().join("after-cancel.jsonl");
        app.log.export_to(&after_cancel).unwrap();
        assert!(
            std::fs::read_to_string(&after_cancel)
                .unwrap()
                .contains("KEEP")
        );

        app.request_log_clear();
        app.confirm_log_clear();
        assert!(!app.confirm_clear_logs);
        let after_clear = directory.path().join("after-clear.jsonl");
        app.log.export_to(&after_clear).unwrap();
        let contents = std::fs::read_to_string(&after_clear).unwrap();
        assert!(!contents.contains("KEEP"));
        assert!(contents.contains("LOGS_CLEARED"));
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

                for tab in [
                    SettingsTab::App,
                    SettingsTab::Webcam,
                    SettingsTab::ScreenDetection,
                    SettingsTab::Diagnostics,
                ] {
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
