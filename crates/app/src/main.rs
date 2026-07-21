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
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WINDOW_ASPECT_RATIO: f32 = 16.0 / 9.0;
const MIN_WINDOW_HEIGHT: f32 = 600.0;
const LIVE_RED: Color32 = Color32::from_rgb(235, 90, 90);
const ACTIVE_GREEN: Color32 = Color32::from_rgb(76, 205, 132);
const TRANSITION_AMBER: Color32 = Color32::from_rgb(245, 190, 75);
const PREVIEW_NEUTRAL: Color32 = Color32::from_rgb(42, 47, 55);
const FPS_WINDOW: Duration = Duration::from_secs(1);
const FPS_REFRESH: Duration = Duration::from_millis(250);
const PREVIEW_REFRESH: Duration = Duration::from_nanos(1_000_000_000 / 30);
const HIDDEN_REFRESH: Duration = Duration::from_millis(250);
const MAX_PREVIEW_TEXTURE_WIDTH: u32 = 480;
const MAX_PREVIEW_TEXTURE_HEIGHT: u32 = 270;
const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
const SETTINGS_ENTRANCE_DURATION: Duration = Duration::from_millis(160);
const SETTINGS_SECTION_DURATION: Duration = Duration::from_millis(120);
const SETTINGS_SIDEBAR_WIDTH: f32 = 222.0;
const SETTINGS_CONTENT_WIDTH: f32 = 760.0;
const SETTINGS_PREVIEW_WIDTH: f32 = 480.0;
const SETTINGS_PREVIEW_HEIGHT: f32 = 270.0;
const SETTINGS_PREVIEW_COLUMNS_BREAKPOINT: f32 = 620.0;

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
            let app = SwitcherApp::new(loaded.config, loaded.warnings, store, start_visible);
            #[cfg(windows)]
            let mut app = app;
            #[cfg(windows)]
            if !start_visible && app.tray.is_none() {
                app.window_visible = true;
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
    VirtualCamera,
    Diagnostics,
}

impl SettingsTab {
    const ALL: [(Self, UiIcon, &'static str); 5] = [
        (Self::App, UiIcon::Settings, "App"),
        (Self::Webcam, UiIcon::Camera, "Webcam"),
        (Self::ScreenDetection, UiIcon::Monitor, "Screen & detection"),
        (Self::VirtualCamera, UiIcon::Broadcast, "Virtual camera"),
        (Self::Diagnostics, UiIcon::Layers, "Diagnostics"),
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::App => "App",
            Self::Webcam => "Webcam",
            Self::ScreenDetection => "Screen & detection",
            Self::VirtualCamera => "Virtual camera",
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
            Self::VirtualCamera => {
                "Verify the feed sent to other apps and configure its missing-source fallback."
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
    sequence: u64,
    size: [usize; 2],
    texture: TextureHandle,
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
    empty_message: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct SettingsPreviewPair {
    screen: Rect,
    reference: Rect,
    side_by_side: bool,
}

#[derive(Clone, Copy)]
struct SettingsPreview<'a> {
    kind: PreviewKind,
    frame: Option<&'a Frame>,
    label: &'a str,
    empty_message: &'static str,
    actual_output: Source,
}

impl PreviewOptions {
    const fn dashboard(kind: PreviewKind) -> Self {
        Self {
            show_fps: kind.shows_fps(),
            empty_message: kind.empty_message(),
        }
    }

    const fn settings(empty_message: &'static str) -> Self {
        Self {
            show_fps: false,
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
enum FpsReading {
    Pending,
    Value(u32),
}

#[derive(Default)]
struct FpsTracker {
    samples: VecDeque<(Instant, u64)>,
    last_sequence: Option<u64>,
    last_advanced_at: Option<Instant>,
    last_display_update: Option<Instant>,
    displayed: Option<u32>,
}

impl FpsTracker {
    fn observe(&mut self, frame: Option<&Frame>, now: Instant) -> FpsReading {
        if let Some(frame) = frame
            && self.last_sequence != Some(frame.sequence)
        {
            if self
                .last_sequence
                .is_some_and(|sequence| frame.sequence <= sequence)
            {
                self.samples.clear();
                self.displayed = None;
                self.last_display_update = None;
            }
            self.last_sequence = Some(frame.sequence);
            self.last_advanced_at = Some(now);
            self.samples.push_back((frame.received_at, frame.sequence));
            while self.samples.front().is_some_and(|(received_at, _)| {
                frame
                    .received_at
                    .checked_duration_since(*received_at)
                    .is_some_and(|age| age > FPS_WINDOW)
            }) {
                self.samples.pop_front();
            }

            if self
                .last_display_update
                .is_none_or(|updated| now.saturating_duration_since(updated) >= FPS_REFRESH)
            {
                self.last_display_update = Some(now);
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
        }

        if self
            .last_advanced_at
            .is_some_and(|advanced| now.saturating_duration_since(advanced) >= FPS_WINDOW)
        {
            FpsReading::Value(0)
        } else {
            self.displayed
                .map_or(FpsReading::Pending, FpsReading::Value)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiIcon {
    Automatic,
    Back,
    Broadcast,
    Camera,
    Capture,
    Check,
    Error,
    Image,
    Layers,
    Loader,
    Monitor,
    Play,
    Question,
    Refresh,
    Settings,
    Stop,
    Target,
    Unavailable,
}

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
    fps_trackers: HashMap<PreviewKind, FpsTracker>,
    log: LocalLog,
    last_activity: Option<String>,
    #[cfg(windows)]
    last_notified_warning: Option<String>,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    exit_requested: bool,
    show_exit_confirmation: bool,
    last_window_size: Option<Vec2>,
    window_visible: bool,
}

impl SwitcherApp {
    fn new(
        mut config: AppConfig,
        load_warnings: Vec<String>,
        store: ConfigStore,
        window_visible: bool,
    ) -> Self {
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
            fps_trackers: HashMap::new(),
            log,
            last_activity: None,
            #[cfg(windows)]
            last_notified_warning: None,
            #[cfg(windows)]
            tray: tray::Tray::new().ok(),
            exit_requested: false,
            show_exit_confirmation: false,
            last_window_size: None,
            window_visible,
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
                (PreviewKind::Webcam, snapshot.previews.webcam.as_deref()),
                (PreviewKind::Screen, snapshot.previews.screen.as_deref()),
            ],
            [
                (
                    PreviewKind::Reference,
                    snapshot.previews.reference.as_deref(),
                ),
                (
                    PreviewKind::Output,
                    snapshot.previews.final_output.as_deref(),
                ),
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
                                rendered_width,
                                rendered_height,
                                snapshot.actual_output,
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
                    UiIcon::Broadcast,
                    "Output mode",
                    None,
                    heading_width,
                    row_height,
                );
                if icon_button(
                    ui,
                    UiIcon::Automatic,
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
            egui::vec2(ui.available_width(), 62.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let back = icon_button(ui, UiIcon::Back, "", egui::vec2(38.0, 38.0), false, false)
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
            .inner_margin(egui::Margin::symmetric(12, 18))
            .show(ui, |ui| {
                ui.set_width(SETTINGS_SIDEBAR_WIDTH - 24.0);
                ui.set_min_height((height - 36.0).max(0.0));
                ui.label(
                    RichText::new("PREFERENCES")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(112, 120, 134)),
                );
                ui.add_space(10.0);
                let mut navigation_rects = Vec::with_capacity(SettingsTab::ALL.len());
                for (tab, icon, label) in SettingsTab::ALL {
                    let response = settings_nav_button(ui, tab, icon, label, self.settings_tab);
                    navigation_rects.push(response.rect);
                    if response.clicked() && self.settings_tab != tab {
                        self.settings_tab = tab;
                        self.confirm_clear_logs = false;
                        self.settings_section_changed_at = Some(Instant::now());
                    }
                    ui.add_space(4.0);
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
                let content_width = (available_width - 44.0).min(SETTINGS_CONTENT_WIDTH);
                ui.allocate_ui_with_layout(
                    egui::vec2(available_width, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(content_width.max(1.0), ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_opacity(0.62 + section_progress * 0.38);
                                ui.add_space(28.0 + (1.0 - section_progress) * 5.0);
                                ui.label(
                                    RichText::new(self.settings_tab.title())
                                        .size(26.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(self.settings_tab.description())
                                        .size(13.0)
                                        .color(Color32::from_rgb(154, 161, 174)),
                                );
                                ui.add_space(28.0);
                                match self.settings_tab {
                                    SettingsTab::App => self.app_settings(ui),
                                    SettingsTab::Webcam => self.webcam_settings(ui),
                                    SettingsTab::ScreenDetection => {
                                        self.screen_detection_settings(ui)
                                    }
                                    SettingsTab::VirtualCamera => self.virtual_camera_settings(ui),
                                    SettingsTab::Diagnostics => self.diagnostics_settings(ui),
                                }
                                ui.add_space(32.0);
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

        settings_section_heading(
            ui,
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

        settings_section_heading(
            ui,
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
        settings_device_status(ui, UiIcon::Camera, "Webcam", snapshot.webcam_state);
        ui.add_space(14.0);
        self.settings_single_preview(
            ui,
            SettingsPreview {
                kind: PreviewKind::Webcam,
                frame: snapshot.previews.webcam.as_deref(),
                label: "Selected webcam",
                empty_message: "No webcam frame — select, refresh, or restart the camera below.",
                actual_output: snapshot.actual_output,
            },
        );

        settings_section_heading(
            ui,
            "Camera input",
            "Choose the camera used whenever webcam output is active.",
        );
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
        settings_control_row(ui, "Camera", "Changes apply automatically.", |ui| {
            egui::ComboBox::from_id_salt("webcam-selector")
                .width(260.0)
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.selected_video_device_id,
                        String::new(),
                        "No camera selected",
                    );
                    for device in snapshot.video_devices.iter() {
                        ui.selectable_value(
                            &mut self.config.selected_video_device_id,
                            device.id.clone(),
                            &device.name,
                        );
                    }
                });
        });
        settings_action_row(
            ui,
            "Camera connection",
            "Refresh the device list or reconnect the selected camera.",
            |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Refresh devices").clicked() {
                        self.send(Command::RefreshVideoDevices);
                    }
                    restart_button(ui, &self.runtime, "Restart webcam", RestartTarget::Webcam);
                });
            },
        );
    }

    fn screen_detection_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.runtime.snapshot();
        ui.horizontal_wrapped(|ui| {
            settings_device_status(ui, UiIcon::Monitor, "Screen capture", snapshot.screen_state);
            ui.separator();
            settings_reference_status(ui, snapshot.previews.reference.is_some());
            ui.separator();
            settings_detection_status(ui, snapshot.detection);
        });
        ui.add_space(14.0);
        let preview_layout = self.settings_screen_reference_previews(ui, &snapshot);
        debug_assert!(preview_layout.screen.is_positive());
        debug_assert!(preview_layout.reference.is_positive());
        debug_assert_eq!(
            preview_layout.side_by_side,
            ui.available_width() >= SETTINGS_PREVIEW_COLUMNS_BREAKPOINT
        );

        settings_section_heading(
            ui,
            "Screen capture",
            "Choose the display Automatic mode watches and what its frames include.",
        );
        let selected_monitor = snapshot
            .selected_monitor
            .as_ref()
            .map_or("No display selected", |monitor| monitor.label.as_str());
        settings_control_row(
            ui,
            "Display",
            "Used for screen output, reference capture, and matching.",
            |ui| {
                egui::ComboBox::from_id_salt("monitor-selector")
                    .width(260.0)
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
                                self.send(Command::SelectMonitor(monitor.clone()));
                            }
                        }
                    });
            },
        );
        settings_toggle_row(
            ui,
            &mut self.config.cursor_visible,
            "Include mouse cursor",
            "Show the pointer in screen output and newly captured references.",
        );

        settings_section_heading(
            ui,
            "Reference and matching",
            "Automatic mode shows the webcam while the screen resembles this reference.",
        );
        settings_action_row(
            ui,
            "Reference image",
            "Use the current screen or import an image from disk.",
            |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Set current screen").clicked() {
                        self.send(Command::CaptureReference);
                    }
                    if ui.button("Import image…").clicked() {
                        self.import_reference_dialog();
                    }
                });
            },
        );
        let strictness = format!("{:.0}%", self.config.similarity_threshold * 100.0);
        settings_control_row(
            ui,
            "Match strictness",
            "Higher values require the live screen to look more like the reference.",
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [142.0, 24.0],
                        egui::Slider::new(&mut self.config.similarity_threshold, 0.50..=1.0)
                            .show_value(false),
                    );
                    ui.label(RichText::new(strictness).monospace());
                    if ui.small_button("Reset 98%").clicked() {
                        self.config.similarity_threshold = 0.98;
                    }
                });
            },
        );
        settings_action_row(
            ui,
            "Capture recovery",
            "Rescan displays or reconnect the selected display capture.",
            |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Rescan displays").clicked() {
                        self.send(Command::Rescan);
                    }
                    restart_button(
                        ui,
                        &self.runtime,
                        "Restart capture",
                        RestartTarget::ScreenCapture,
                    );
                });
            },
        );
    }

    fn virtual_camera_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.runtime.snapshot();
        settings_device_status(
            ui,
            UiIcon::Broadcast,
            "Virtual camera",
            snapshot.virtual_camera_state,
        );
        ui.add_space(14.0);
        self.settings_single_preview(
            ui,
            SettingsPreview {
                kind: PreviewKind::Output,
                frame: snapshot.previews.final_output.as_deref(),
                label: "Current virtual-camera output",
                empty_message: "No output frame — restart the virtual camera below.",
                actual_output: snapshot.actual_output,
            },
        );

        settings_section_heading(
            ui,
            "Missing-source appearance",
            "This color appears when automation is running but its selected source is unavailable.",
        );
        fallback_color_swatch(ui, bgra_color(self.config.placeholder_color_bgra));
        ui.add_space(10.0);
        settings_control_row(
            ui,
            "Fallback color",
            "The preview above updates before the change is saved.",
            |ui| {
                let mut color = bgra_color(self.config.placeholder_color_bgra);
                if ui.color_edit_button_srgba(&mut color).changed() {
                    self.config.placeholder_color_bgra =
                        u32::from_le_bytes([color.b(), color.g(), color.r(), color.a()]);
                }
            },
        );
        settings_action_row(
            ui,
            "Virtual-camera recovery",
            "Reconnect the feed exposed to camera applications.",
            |ui| {
                restart_button(
                    ui,
                    &self.runtime,
                    "Restart virtual camera",
                    RestartTarget::VirtualCamera,
                );
            },
        );
    }

    fn diagnostics_settings(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.runtime.snapshot();
        settings_section_heading(
            ui,
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

        settings_section_heading(
            ui,
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

        settings_section_heading(
            ui,
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
        settings_action_row(
            ui,
            "Full pipeline recovery",
            "Reconnect the webcam, screen capture, and virtual camera.",
            |ui| {
                restart_button(
                    ui,
                    &self.runtime,
                    "Restart all components",
                    RestartTarget::All,
                );
            },
        );
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

    fn settings_screen_reference_previews(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
    ) -> SettingsPreviewPair {
        let available = ui.available_width();
        let side_by_side = available >= SETTINGS_PREVIEW_COLUMNS_BREAKPOINT;
        let mut screen = Rect::NOTHING;
        let mut reference = Rect::NOTHING;
        if side_by_side {
            let gap = 16.0;
            let width = ((available - gap) / 2.0).min(SETTINGS_PREVIEW_WIDTH);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.horizontal_top(|ui| {
                    screen = self.settings_preview_panel(
                        ui,
                        SettingsPreview {
                            kind: PreviewKind::Screen,
                            frame: snapshot.previews.screen.as_deref(),
                            label: "Live screen",
                            empty_message: "No screen frame — rescan or restart capture below.",
                            actual_output: snapshot.actual_output,
                        },
                        width,
                    );
                    reference = self.settings_preview_panel(
                        ui,
                        SettingsPreview {
                            kind: PreviewKind::Reference,
                            frame: snapshot.previews.reference.as_deref(),
                            label: "Reference image",
                            empty_message: "No reference image — capture or import one below.",
                            actual_output: snapshot.actual_output,
                        },
                        width,
                    );
                });
            });
        } else {
            screen = self.settings_single_preview(
                ui,
                SettingsPreview {
                    kind: PreviewKind::Screen,
                    frame: snapshot.previews.screen.as_deref(),
                    label: "Live screen",
                    empty_message: "No screen frame — rescan or restart capture below.",
                    actual_output: snapshot.actual_output,
                },
            );
            ui.add_space(14.0);
            reference = self.settings_single_preview(
                ui,
                SettingsPreview {
                    kind: PreviewKind::Reference,
                    frame: snapshot.previews.reference.as_deref(),
                    label: "Reference image",
                    empty_message: "No reference image — capture or import one below.",
                    actual_output: snapshot.actual_output,
                },
            );
        }
        SettingsPreviewPair {
            screen,
            reference,
            side_by_side,
        }
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
        frame: Option<&Frame>,
        width: f32,
        height: f32,
        actual_output: Source,
    ) {
        self.preview(
            ui,
            kind,
            frame,
            [width, height],
            actual_output,
            PreviewOptions::dashboard(kind),
        );
        ui.add_space(8.0);
        preview_caption(ui, kind);
        ui.add_space(16.0);
    }

    fn preview(
        &mut self,
        ui: &mut egui::Ui,
        kind: PreviewKind,
        frame: Option<&Frame>,
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
        let fps_reading = options.show_fps.then(|| {
            self.fps_trackers
                .entry(kind)
                .or_default()
                .observe(frame, Instant::now())
        });
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
                        let texture_size =
                            preview_texture_size(frame, inner_size, ui.ctx().pixels_per_point());
                        let texture =
                            self.textures
                                .entry(kind.key())
                                .or_insert_with(|| PreviewTexture {
                                    sequence: 0,
                                    size: texture_size,
                                    texture: ui.ctx().load_texture(
                                        kind.key(),
                                        frame_image(frame, texture_size),
                                        TextureOptions::LINEAR,
                                    ),
                                });
                        if texture.sequence != frame.sequence || texture.size != texture_size {
                            texture
                                .texture
                                .set(frame_image(frame, texture_size), TextureOptions::LINEAR);
                            texture.sequence = frame.sequence;
                            texture.size = texture_size;
                        }
                        ui.add(
                            egui::Image::new((texture.texture.id(), inner_size))
                                .maintain_aspect_ratio(true),
                        );
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
        if let Some(reading) = fps_reading {
            paint_fps_overlay(ui, preview.response.rect, reading);
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
                    self.window_visible = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayAction::Command(command) => self.send(command),
                tray::TrayAction::Exit => {
                    if self.config.confirm_exit {
                        self.show_exit_confirmation = true;
                        self.window_visible = true;
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
            self.window_visible = false;
        } else if close_requested && self.config.confirm_exit && !self.exit_requested {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_exit_confirmation = true;
        }
        context.request_repaint_after(if self.window_visible {
            PREVIEW_REFRESH
        } else {
            HIDDEN_REFRESH
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.root_ui(ui);
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
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
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

fn settings_section_heading(ui: &mut egui::Ui, title: &str, description: &str) {
    if ui.cursor().top() > ui.min_rect().top() + 110.0 {
        ui.add_space(28.0);
    }
    ui.label(
        RichText::new(title)
            .size(15.0)
            .strong()
            .color(Color32::from_rgb(228, 231, 237)),
    );
    ui.add_space(3.0);
    ui.label(
        RichText::new(description)
            .size(11.5)
            .color(Color32::from_rgb(128, 136, 150)),
    );
    ui.add_space(10.0);
    ui.separator();
}

fn settings_toggle_row(ui: &mut egui::Ui, value: &mut bool, title: &str, description: &str) {
    settings_control_row(ui, title, description, |ui| {
        ui.add(egui::Checkbox::without_text(value));
    });
}

fn settings_control_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let width = ui.available_width();
    let control_width = 280.0_f32.min((width * 0.44).max(180.0));
    let label_width = (width - control_width - 16.0).max(180.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, 66.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, 54.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add_space(7.0);
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
                egui::vec2(control_width, 54.0),
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

fn fallback_color_swatch(ui: &mut egui::Ui, color: Color32) -> Rect {
    let width = ui.available_width().min(240.0);
    let height = width / WINDOW_ASPECT_RATIO;
    let mut rect = Rect::NOTHING;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let (allocated, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
            ui.painter().rect_filled(allocated, 8, color);
            ui.painter().rect_stroke(
                allocated,
                8,
                Stroke::new(2.0, Color32::from_rgb(70, 76, 88)),
                StrokeKind::Inside,
            );
            rect = allocated;
        },
    );
    rect
}

fn restart_button(ui: &mut egui::Ui, runtime: &RuntimeHandle, label: &str, target: RestartTarget) {
    if ui.button(label).clicked() {
        let _ = runtime.try_send(Command::Restart(target));
    }
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

fn paint_fps_overlay(ui: &egui::Ui, preview_rect: Rect, reading: FpsReading) {
    let text = match reading {
        FpsReading::Pending => "-- FPS".to_owned(),
        FpsReading::Value(value) => format!("{value} FPS"),
    };
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
        UiIcon::Automatic | UiIcon::Refresh => {
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

fn bgra_color(value: u32) -> Color32 {
    let [b, g, r, a] = value.to_le_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_at(sequence: u64, received_at: Instant) -> Frame {
        Frame::new(
            vec![0, 0, 0, 255].into(),
            asc_core::Size::new(1, 1),
            4,
            sequence,
            0,
            received_at,
        )
        .unwrap()
    }

    #[test]
    fn default_ui_fonts_are_bundled() {
        assert!(!egui::FontDefinitions::default().font_data.is_empty());
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
    fn fps_tracker_measures_steady_and_skipped_sequences() {
        let start = Instant::now();
        let mut steady = FpsTracker::default();
        let mut reading = FpsReading::Pending;
        for sequence in 1..=31 {
            let elapsed = Duration::from_secs_f64((sequence - 1) as f64 / 30.0);
            let frame = frame_at(sequence, start + elapsed);
            reading = steady.observe(Some(&frame), start + elapsed);
        }
        assert_eq!(reading, FpsReading::Value(30));

        let mut skipped = FpsTracker::default();
        let first = frame_at(1, start);
        assert_eq!(skipped.observe(Some(&first), start), FpsReading::Pending);
        let later = frame_at(11, start + Duration::from_millis(333));
        assert_eq!(
            skipped.observe(Some(&later), start + Duration::from_millis(333)),
            FpsReading::Value(30)
        );
    }

    #[test]
    fn fps_tracker_resets_and_reports_stalls() {
        let start = Instant::now();
        let mut tracker = FpsTracker::default();
        let first = frame_at(10, start);
        tracker.observe(Some(&first), start);
        let later = frame_at(20, start + Duration::from_millis(333));
        assert_eq!(
            tracker.observe(Some(&later), start + Duration::from_millis(333)),
            FpsReading::Value(30)
        );

        let reset = frame_at(1, start + Duration::from_millis(500));
        assert_eq!(
            tracker.observe(Some(&reset), start + Duration::from_millis(500)),
            FpsReading::Pending
        );
        assert_eq!(
            tracker.observe(Some(&reset), start + Duration::from_millis(1500)),
            FpsReading::Value(0)
        );
    }

    #[test]
    fn fps_overlays_skip_the_static_reference() {
        assert!(PreviewKind::Webcam.shows_fps());
        assert!(PreviewKind::Screen.shows_fps());
        assert!(PreviewKind::Output.shows_fps());
        assert!(!PreviewKind::Reference.shows_fps());
        assert!(!PreviewOptions::settings("missing").show_fps);
        assert!(PreviewOptions::dashboard(PreviewKind::Webcam).show_fps);
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
            true,
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
                assert!((rect.height() - 40.0).abs() < 0.01);
            }
            for pair in navigation_rects.windows(2) {
                assert!(
                    pair[1].top() - pair[0].bottom() >= 3.9,
                    "navigation rows overlap: {:?} and {:?}",
                    pair[0],
                    pair[1]
                );
                assert!(!pair[0].intersects(pair[1]));
            }
        }
    }

    #[test]
    fn settings_previews_are_bounded_and_responsive() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
            true,
        );
        let snapshot = AppSnapshot::default();
        for (width, expected_side_by_side) in [(760.0, true), (560.0, false)] {
            let context = egui::Context::default();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 900.0))),
                ..egui::RawInput::default()
            };
            let mut layout = None;
            let output = context.run_ui(input, |ui| {
                ui.set_width(width);
                layout = Some(app.settings_screen_reference_previews(ui, &snapshot));
            });
            let layout = layout.unwrap();
            assert_eq!(layout.side_by_side, expected_side_by_side);
            for rect in [layout.screen, layout.reference] {
                assert!(rect.is_positive());
                assert!(rect.left() >= -0.01 && rect.right() <= width + 0.01);
                assert!(rect.width() <= SETTINGS_PREVIEW_WIDTH + 0.01);
                assert!(rect.height() <= SETTINGS_PREVIEW_HEIGHT + 0.01);
                assert!((rect.width() / rect.height() - WINDOW_ASPECT_RATIO).abs() < 0.01);
            }
            assert!(!layout.screen.intersects(layout.reference));
            if expected_side_by_side {
                assert!((layout.screen.top() - layout.reference.top()).abs() < 0.01);
                assert!(layout.reference.left() > layout.screen.right());
            } else {
                assert!(layout.reference.top() > layout.screen.bottom());
            }
            assert!(!output.shapes.is_empty());
        }

        let context = egui::Context::default();
        let mut swatch = Rect::NOTHING;
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(520.0);
            swatch = fallback_color_swatch(ui, Color32::from_rgb(12, 34, 56));
        });
        assert!(swatch.is_positive());
        assert!((swatch.width() / swatch.height() - WINDOW_ASPECT_RATIO).abs() < 0.01);
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn clearing_logs_requires_explicit_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig::default(),
            Vec::new(),
            ConfigStore::new(directory.path()),
            true,
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
        let mut app = SwitcherApp::new(AppConfig::default(), Vec::new(), store.clone(), true);
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
            true,
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
            true,
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
                    SettingsTab::VirtualCamera,
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
