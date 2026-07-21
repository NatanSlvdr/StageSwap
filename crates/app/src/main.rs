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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsTab {
    General,
    Sources,
    Detection,
    Output,
    Logs,
}

struct PreviewTexture {
    sequence: u64,
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
    Broadcast,
    Camera,
    Capture,
    Check,
    Error,
    Image,
    Layers,
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
    settings_draft: AppConfig,
    runtime: RuntimeHandle,
    store: ConfigStore,
    load_warnings: Vec<String>,
    show_settings: bool,
    settings_tab: SettingsTab,
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
            settings_draft: config.clone(),
            config,
            store,
            load_warnings,
            show_settings: false,
            settings_tab: SettingsTab::General,
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
        }
    }

    fn send(&self, command: Command) {
        let _ = self.runtime.send(command);
    }

    fn set_mode(&mut self, mode: OutputMode) {
        self.config.output_mode = mode;
        self.settings_draft.output_mode = mode;
        self.send(Command::SetMode(mode));
    }

    fn apply_settings(&mut self) {
        self.send(Command::UpdateSettings(Box::new(self.config.clone())));
        if let Err(error) = save_config(&self.store, &self.config) {
            self.load_warnings
                .push(format!("Could not save settings: {error}"));
        }
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

    fn dashboard(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
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
        let workspace_height = ui.available_height().max(160.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(preview_width, workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.preview_workspace(ui, &snapshot, preview_width, workspace_height),
            );

            ui.separator();
            let controls_width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(controls_width, workspace_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.controls_workspace(ui, &snapshot, controls_width, workspace_height),
            );
        });
        self.settings_window(context);
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
        ui.set_width(width);
        let body_height = (height - FOOTER_HEIGHT - FOOTER_GAP).max(80.0);
        ui.allocate_ui_with_layout(
            egui::vec2(width, body_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(width, body_height));
                self.preview_grid(ui, snapshot, width, body_height);
            },
        );
        ui.add_space(FOOTER_GAP);
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
                egui::ScrollArea::vertical()
                    .id_salt("dashboard-controls")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.controls_body(ui, snapshot));
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
            self.settings_draft = self.config.clone();
            self.send(Command::RefreshVideoDevices);
            self.send(Command::Rescan);
            self.show_settings = true;
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
        selected_source_group(ui, snapshot.actual_output);
        detection_state_group(ui, snapshot.detection);
        screen_mix_group(ui, snapshot.transition.screen_mix);
        ui.add_space(4.0);

        let automation_running =
            matches!(snapshot.run_state, RunState::Running | RunState::Starting);
        let (run_icon, run_label) = if automation_running {
            (UiIcon::Stop, "Stop automation")
        } else {
            (UiIcon::Play, "Start automation")
        };
        if icon_button(
            ui,
            run_icon,
            run_label,
            egui::vec2(ui.available_width(), 36.0),
            false,
            true,
        )
        .clicked()
        {
            if automation_running {
                self.send(Command::Stop);
            } else {
                self.send(Command::Start);
            }
        }

        ui.add_space(2.0);
        icon_text(ui, UiIcon::Broadcast, "OUTPUT MODE", Color32::GRAY, false);
        for (mode, icon, label) in [
            (OutputMode::Automatic, UiIcon::Automatic, "Automatic"),
            (OutputMode::ForceCamera, UiIcon::Camera, "Webcam"),
            (OutputMode::ForceScreen, UiIcon::Monitor, "Screen"),
        ] {
            if icon_button(
                ui,
                icon,
                label,
                egui::vec2(ui.available_width(), 30.0),
                snapshot.mode == mode,
                false,
            )
            .clicked()
            {
                self.set_mode(mode);
            }
        }
        ui.add_space(2.0);
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

    fn settings_window(&mut self, context: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .default_size([760.0, 520.0])
            .show(context, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (SettingsTab::General, "General"),
                        (SettingsTab::Sources, "Sources"),
                        (SettingsTab::Detection, "Detection"),
                        (SettingsTab::Output, "Output"),
                        (SettingsTab::Logs, "Logs"),
                    ] {
                        ui.selectable_value(&mut self.settings_tab, tab, label);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| match self.settings_tab {
                    SettingsTab::General => {
                        ui.checkbox(
                            &mut self.settings_draft.start_automatically,
                            "Start automation on launch",
                        );
                        ui.checkbox(&mut self.settings_draft.start_with_windows, "Start with Windows");
                        ui.checkbox(&mut self.settings_draft.start_minimized, "Start minimized");
                        ui.checkbox(&mut self.settings_draft.close_to_tray, "Close window to tray");
                        ui.checkbox(&mut self.settings_draft.confirm_exit, "Confirm before exit");
                        ui.checkbox(&mut self.settings_draft.show_notifications, "Show status notifications");
                    }
                    SettingsTab::Sources => {
                        ui.heading("Webcam / video");
                        let snapshot = self.runtime.snapshot();
                        let selected_name = snapshot
                            .video_devices
                            .iter()
                            .find(|device| device.id == self.settings_draft.selected_video_device_id)
                            .map(|device| device.name.clone())
                            .unwrap_or_else(|| {
                                if self.settings_draft.selected_video_device_id.is_empty() {
                                    "No video input selected".into()
                                } else {
                                    format!(
                                        "Unavailable saved source — {}",
                                        self.settings_draft.selected_video_device_id
                                    )
                                }
                            });
                        egui::ComboBox::from_id_salt("webcam-selector")
                            .selected_text(selected_name)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.settings_draft.selected_video_device_id,
                                    String::new(),
                                    "No video input selected",
                                );
                                for device in snapshot.video_devices.iter() {
                                    ui.selectable_value(
                                        &mut self.settings_draft.selected_video_device_id,
                                        device.id.clone(),
                                        &device.name,
                                    );
                                }
                            });
                        ui.collapsing("Video source details", |ui| {
                            ui.monospace(if self.settings_draft.selected_video_device_id.is_empty() {
                                "No symbolic link selected"
                            } else {
                                &self.settings_draft.selected_video_device_id
                            });
                            ui.label("Fixed capture: RGB32 1280×720 at 30 fps");
                        });
                        restart_button(ui, &self.runtime, "Restart webcam", RestartTarget::Webcam);
                        ui.separator();
                        ui.heading("Screen capture");
                        ui.checkbox(&mut self.settings_draft.cursor_visible, "Include mouse cursor");
                        let selected_monitor = snapshot
                            .selected_monitor
                            .as_ref()
                            .map_or("No monitor selected", |monitor| monitor.label.as_str());
                        egui::ComboBox::from_id_salt("monitor-selector")
                            .selected_text(selected_monitor)
                            .show_ui(ui, |ui| {
                                for monitor in snapshot.monitors.iter() {
                                    let label = format!(
                                        "{} — {}×{} at ({}, {})",
                                        monitor.label,
                                        monitor.width,
                                        monitor.height,
                                        monitor.x,
                                        monitor.y
                                    );
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
                        restart_button(
                            ui,
                            &self.runtime,
                            "Restart screen capture",
                            RestartTarget::ScreenCapture,
                        );
                    }
                    SettingsTab::Detection => {
                        ui.heading("Reference");
                        ui.label("Capture or import the visual that keeps the webcam active in Automatic mode.");
                        ui.horizontal(|ui| {
                            if ui.button("Set current screen").clicked() {
                                self.send(Command::CaptureReference);
                            }
                            if ui.button("Import image…").clicked() {
                                self.import_reference_dialog();
                            }
                        });
                        ui.label(format!("Local reference: {}", self.settings_draft.reference_image_path));
                        ui.separator();
                        ui.heading("Similarity");
                        ui.label("Reference similarity threshold");
                        ui.add(
                            egui::Slider::new(&mut self.settings_draft.similarity_threshold, 0.50..=1.0)
                                .fixed_decimals(3),
                        );
                        ui.label("Fixed behavior: every 250 ms; 5 matches; 3 mismatches; full monitor scan every 30 seconds; a new monitor must win twice.");
                        if ui.button("Rescan screens now").clicked() {
                            self.send(Command::Rescan);
                        }
                    }
                    SettingsTab::Output => {
                        ui.heading("Fixed frame pipeline");
                        ui.label("CPU BGRA composition at 1280×720 and 30 fps with aspect-fit scaling and black letterboxing.");
                        ui.label("Switches use a reversible 500 ms fade with the live screen frame.");
                        restart_button(
                            ui,
                            &self.runtime,
                            "Restart virtual camera",
                            RestartTarget::VirtualCamera,
                        );
                        ui.separator();
                        ui.heading("Safe fallback");
                        let mut color = bgra_color(self.settings_draft.placeholder_color_bgra);
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            self.settings_draft.placeholder_color_bgra =
                                u32::from_le_bytes([color.b(), color.g(), color.r(), color.a()]);
                        }
                        ui.label("Placeholder color");
                        restart_button(
                            ui,
                            &self.runtime,
                            "Restart all components",
                            RestartTarget::All,
                        );
                    }
                    SettingsTab::Logs => {
                        ui.heading("Logging");
                        ui.label("Logs are retained locally for 14 days.");
                        ui.label(format!(
                            "Directory: {}",
                            self.log.directory().display()
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Open log folder").clicked() {
                                self.open_log_directory();
                            }
                            if ui.button("Export logs…").clicked() {
                                self.export_logs();
                            }
                            if ui.button("Clear logs").clicked() {
                                self.clear_logs();
                            }
                        });
                        ui.separator();
                        ui.label(format!("Configuration: {}", self.store.config_path().display()));
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Apply settings").clicked() {
                        apply = true;
                    }
                });
            });
        if apply {
            self.config = self.settings_draft.clone();
            self.apply_settings();
            self.show_settings = false;
        } else {
            self.show_settings = open && !cancel;
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
                ui.label("The virtual camera will remain registered and show its placeholder when the publisher stops.");
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
        self.preview(ui, kind, frame, [width, height], actual_output);
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
    ) {
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
        let contour_width = match contour {
            PreviewContour::Live => 3.0,
            PreviewContour::Active | PreviewContour::Neutral => 1.0 + 2.0 * active_amount,
        };
        let fps_reading = kind.shows_fps().then(|| {
            self.fps_trackers
                .entry(kind)
                .or_default()
                .observe(frame, Instant::now())
        });
        let inner_size = (available - egui::vec2(6.0, 6.0)).max(egui::vec2(1.0, 1.0));
        let preview = egui::Frame::new()
            .fill(Color32::from_rgb(12, 14, 18))
            .stroke(Stroke::new(contour_width, contour_color))
            .corner_radius(8)
            .inner_margin(3)
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    inner_size,
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        if let Some(frame) = frame {
                            let texture =
                                self.textures
                                    .entry(kind.key())
                                    .or_insert_with(|| PreviewTexture {
                                        sequence: 0,
                                        texture: ui.ctx().load_texture(
                                            kind.key(),
                                            frame_image(frame),
                                            TextureOptions::LINEAR,
                                        ),
                                    });
                            if texture.sequence != frame.sequence {
                                texture
                                    .texture
                                    .set(frame_image(frame), TextureOptions::LINEAR);
                                texture.sequence = frame.sequence;
                            }
                            ui.add(
                                egui::Image::new((texture.texture.id(), inner_size))
                                    .maintain_aspect_ratio(true),
                            );
                        } else {
                            ui.label(RichText::new("No frame").color(Color32::DARK_GRAY));
                        }
                    },
                );
            });
        if let Some(reading) = fps_reading {
            paint_fps_overlay(ui, preview.response.rect, reading);
        }
    }
}

impl eframe::App for SwitcherApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = self.runtime.snapshot();
        if !snapshot.selected_video_device_id.is_empty()
            && snapshot.selected_video_device_id != self.config.selected_video_device_id
        {
            self.config.selected_video_device_id = snapshot.selected_video_device_id.clone();
            if self.settings_draft.selected_video_device_id.is_empty() {
                self.settings_draft.selected_video_device_id =
                    snapshot.selected_video_device_id.clone();
            }
            if let Err(error) = save_config(&self.store, &self.config) {
                self.load_warnings.push(format!(
                    "Could not save automatic webcam selection: {error}"
                ));
            }
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
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        context.request_repaint_after(std::time::Duration::from_millis(33));
        self.dashboard(&context, ui);
        self.exit_confirmation(&context);
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
    });
    indicator_group(ui, icon, label, None, &choices);
}

#[derive(Clone, Copy)]
struct IndicatorChoice {
    icon: UiIcon,
    label: &'static str,
    current: bool,
    tone: IndicatorTone,
}

#[derive(Clone, Copy)]
enum IndicatorTone {
    Green,
    Amber,
    Red,
}

fn selected_source_group(ui: &mut egui::Ui, current: Source) {
    let choices = [
        IndicatorChoice {
            icon: UiIcon::Camera,
            label: "Webcam",
            current: current == Source::Camera,
            tone: IndicatorTone::Green,
        },
        IndicatorChoice {
            icon: UiIcon::Monitor,
            label: "Screen",
            current: current == Source::Screen,
            tone: IndicatorTone::Green,
        },
        IndicatorChoice {
            icon: UiIcon::Image,
            label: "Placeholder",
            current: current == Source::Placeholder,
            tone: IndicatorTone::Red,
        },
    ];
    indicator_group(ui, UiIcon::Target, "Selected", None, &choices);
}

fn detection_state_group(ui: &mut egui::Ui, current: DetectionState) {
    let choices = [
        IndicatorChoice {
            icon: UiIcon::Question,
            label: "Unknown",
            current: current == DetectionState::Unknown,
            tone: IndicatorTone::Amber,
        },
        IndicatorChoice {
            icon: UiIcon::Check,
            label: "Matching",
            current: current == DetectionState::Matching,
            tone: IndicatorTone::Green,
        },
        IndicatorChoice {
            icon: UiIcon::Error,
            label: "Not matching",
            current: current == DetectionState::NotMatching,
            tone: IndicatorTone::Red,
        },
        IndicatorChoice {
            icon: UiIcon::Unavailable,
            label: "Reference missing",
            current: current == DetectionState::ReferenceMissing,
            tone: IndicatorTone::Red,
        },
    ];
    indicator_group(ui, UiIcon::Target, "Detection", None, &choices);
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
        },
        IndicatorChoice {
            icon: UiIcon::Layers,
            label: "Crossfading",
            current: active == 1,
            tone: IndicatorTone::Amber,
        },
        IndicatorChoice {
            icon: UiIcon::Monitor,
            label: "Screen only",
            current: active == 2,
            tone: IndicatorTone::Green,
        },
    ];
    let percentage = format!("{}%", (screen_mix * 100.0).round());
    indicator_group(
        ui,
        UiIcon::Layers,
        "Screen mix",
        Some(&percentage),
        &choices,
    );
}

fn indicator_group(
    ui: &mut egui::Ui,
    icon: UiIcon,
    label: &'static str,
    value: Option<&str>,
    choices: &[IndicatorChoice],
) {
    indicator_heading(ui, icon, label, value);
    ui.add_space(2.0);
    let gap = 4.0;
    let width = ((ui.available_width() - gap * (choices.len() as f32 - 1.0))
        / choices.len() as f32)
        .max(32.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.horizontal(|ui| {
            for choice in choices {
                indicator_chip(ui, label, *choice, width);
            }
        });
    });
    ui.add_space(6.0);
}

fn indicator_heading(ui: &mut egui::Ui, icon: UiIcon, label: &'static str, value: Option<&str>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), Sense::hover());
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

fn indicator_chip(ui: &mut egui::Ui, group: &'static str, choice: IndicatorChoice, width: f32) {
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
    draw_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), egui::vec2(15.0, 15.0)),
        choice.icon,
        icon_color,
        1.55,
    );
    response.on_hover_text(choice.label);
}

fn indicator_palette(tone: IndicatorTone, active_amount: f32) -> (Color32, Color32, Color32) {
    let inactive_fill = Color32::from_rgb(62, 31, 35);
    let inactive_stroke = Color32::from_rgb(138, 58, 64);
    let inactive_text = Color32::from_rgb(210, 125, 130);
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
        DeviceState::Initializing => UiIcon::Refresh,
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
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    let visuals = ui.style().interact(&response);
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if emphasized && !response.hovered() {
        Color32::from_rgb(38, 42, 50)
    } else {
        visuals.bg_fill
    };
    let stroke = if selected {
        Stroke::new(1.0, ui.visuals().selection.stroke.color)
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
    let content_width = icon_size + 7.0 + galley.size().x;
    let left = rect.center().x - content_width / 2.0;
    let icon_rect = Rect::from_min_size(
        Pos2::new(left, rect.center().y - icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );
    draw_icon(ui.painter(), icon_rect, icon, color, 1.45);
    ui.painter().galley(
        Pos2::new(
            icon_rect.right() + 7.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_icon(painter: &egui::Painter, rect: Rect, icon: UiIcon, color: Color32, width: f32) {
    let stroke = Stroke::new(width, color);
    let center = rect.center();
    let x = |fraction: f32| egui::lerp(rect.left()..=rect.right(), fraction);
    let y = |fraction: f32| egui::lerp(rect.top()..=rect.bottom(), fraction);
    match icon {
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

fn frame_image(frame: &Frame) -> egui::ColorImage {
    let mut rgba = Vec::with_capacity(frame.pixels().len());
    for pixel in frame.pixels().chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    egui::ColorImage::from_rgba_unmultiplied(
        [frame.size.width as usize, frame.size.height as usize],
        &rgba,
    )
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
        let image = frame_image(&frame);
        assert_eq!(image.size, [2, 1]);
        assert_eq!(image.as_raw(), &[1, 2, 3, 255, 10, 20, 30, 255]);
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
        assert_eq!(
            device_state_icon(DeviceState::Initializing),
            UiIcon::Refresh
        );
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
    }

    #[test]
    fn responsive_dashboard_and_all_settings_tabs_render() {
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
                for tab in [
                    SettingsTab::General,
                    SettingsTab::Sources,
                    SettingsTab::Detection,
                    SettingsTab::Output,
                    SettingsTab::Logs,
                ] {
                    app.settings_tab = tab;
                    app.show_settings = true;
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
                        let context = ui.ctx().clone();
                        app.dashboard(&context, ui);
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
