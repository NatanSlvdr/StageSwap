#![cfg_attr(windows, windows_subsystem = "windows")]

use asc_app::RuntimeHandle;
use asc_core::{
    AppConfig, AppSnapshot, Command, ConfigStore, DeviceState, Frame, OutputMode, RestartTarget,
    RunState,
};
use eframe::egui::{self, Color32, RichText, Stroke, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;

const WINDOW_ASPECT_RATIO: f32 = 16.0 / 9.0;
const MIN_WINDOW_HEIGHT: f32 = 600.0;

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

struct SwitcherApp {
    config: AppConfig,
    settings_draft: AppConfig,
    runtime: RuntimeHandle,
    store: ConfigStore,
    load_warnings: Vec<String>,
    show_settings: bool,
    settings_tab: SettingsTab,
    textures: HashMap<&'static str, PreviewTexture>,
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
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading("Automatic Screen Camera");
                ui.label(RichText::new("Live switcher").color(Color32::GRAY));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.settings_draft = self.config.clone();
                    self.send(Command::RefreshVideoDevices);
                    self.send(Command::Rescan);
                    self.show_settings = true;
                }
            });
        });
        ui.add_space(10.0);
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
        let preview_column_height = (ui.available_height() - 170.0).max(160.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(preview_width, preview_column_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(preview_width);
                    ui.label(RichText::new("SIGNAL FLOW").small().color(Color32::GRAY));
                    let spacing = ui.spacing().item_spacing.y;
                    let cell_height = (ui.available_height() - spacing) / 2.0;
                    let label_height = ui.text_style_height(&egui::TextStyle::Small);
                    let cell_width = (ui.available_width() - spacing) / 2.0;
                    let maximum_preview_height = (cell_height - label_height - spacing).max(24.0);
                    let rendered_width = cell_width
                        .min(maximum_preview_height * WINDOW_ASPECT_RATIO)
                        .max(24.0 * WINDOW_ASPECT_RATIO);
                    let rendered_height = rendered_width / WINDOW_ASPECT_RATIO;
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(cell_width, cell_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                self.preview_cell(
                                    ui,
                                    "webcam",
                                    "WEBCAM",
                                    snapshot.previews.webcam.as_deref(),
                                    rendered_width,
                                    rendered_height,
                                );
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(cell_width, cell_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                self.preview_cell(
                                    ui,
                                    "screen",
                                    "SCREEN",
                                    snapshot.previews.screen.as_deref(),
                                    rendered_width,
                                    rendered_height,
                                );
                            },
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(cell_width, cell_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                self.preview_cell(
                                    ui,
                                    "reference",
                                    "REFERENCE",
                                    snapshot.previews.reference.as_deref(),
                                    rendered_width,
                                    rendered_height,
                                );
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(cell_width, cell_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                self.preview_cell(
                                    ui,
                                    "output",
                                    "OUTPUT",
                                    snapshot.previews.final_output.as_deref(),
                                    rendered_width,
                                    rendered_height,
                                );
                            },
                        );
                    });
                },
            );

            ui.separator();
            ui.vertical(|ui| {
                ui.label(RichText::new("CONTROL").small().color(Color32::GRAY));
                status_row(ui, "Webcam", snapshot.webcam_state);
                status_row(ui, "Screen", snapshot.screen_state);
                status_row(ui, "Output", snapshot.virtual_camera_state);
                ui.separator();
                detail_row(ui, "Selected", format!("{:?}", snapshot.actual_output));
                detail_row(ui, "Detection", format!("{:?}", snapshot.detection));
                detail_row(
                    ui,
                    "Screen mix",
                    format!("{}%", (snapshot.transition.screen_mix * 100.0).round()),
                );
                ui.add_space(4.0);

                let run_label = match snapshot.run_state {
                    RunState::Running | RunState::Starting => "Stop automation",
                    _ => "Start automation",
                };
                if ui
                    .add_sized(
                        [ui.available_width(), 36.0],
                        egui::Button::new(RichText::new(run_label).strong()),
                    )
                    .clicked()
                {
                    match snapshot.run_state {
                        RunState::Running | RunState::Starting => self.send(Command::Stop),
                        _ => self.send(Command::Start),
                    }
                }

                ui.label(RichText::new("OUTPUT MODE").small().color(Color32::GRAY));
                for (mode, label) in [
                    (OutputMode::Automatic, "Automatic"),
                    (OutputMode::ForceCamera, "Webcam"),
                    (OutputMode::ForceScreen, "Screen"),
                ] {
                    if ui
                        .add_sized(
                            [ui.available_width(), 30.0],
                            egui::Button::selectable(snapshot.mode == mode, label),
                        )
                        .clicked()
                    {
                        self.set_mode(mode);
                    }
                }
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Capture reference").clicked() {
                        self.send(Command::CaptureReference);
                    }
                    if ui.button("Rescan screens").clicked() {
                        self.send(Command::Rescan);
                    }
                });
            });
        });

        ui.add_space(6.0);
        self.console(ui, &snapshot);
        self.settings_window(context);
    }

    fn console(&self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        egui::Frame::new()
            .fill(Color32::from_rgb(7, 9, 12))
            .stroke(Stroke::new(1.0, Color32::from_rgb(42, 47, 55)))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").color(Color32::from_rgb(255, 95, 87)));
                    ui.label(RichText::new("●").color(Color32::from_rgb(254, 188, 46)));
                    ui.label(RichText::new("●").color(Color32::from_rgb(40, 200, 64)));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("runtime console")
                            .monospace()
                            .small()
                            .color(Color32::from_rgb(142, 150, 163)),
                    );
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("runtime-console")
                    .max_height(112.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if snapshot.recent_activity.is_empty() {
                            ui.label(
                                RichText::new("> waiting for runtime events…")
                                    .monospace()
                                    .color(Color32::from_rgb(112, 122, 136)),
                            );
                        }
                        for activity in snapshot.recent_activity.iter().rev().take(8).rev() {
                            ui.label(
                                RichText::new(format!("> {activity}"))
                                    .monospace()
                                    .color(Color32::from_rgb(190, 205, 190)),
                            );
                        }
                    });
            });
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
        key: &'static str,
        title: &str,
        frame: Option<&Frame>,
        width: f32,
        height: f32,
    ) {
        ui.label(
            RichText::new(title)
                .small()
                .strong()
                .color(Color32::LIGHT_GRAY),
        );
        self.preview(ui, key, frame, [width, height]);
    }

    fn preview(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        frame: Option<&Frame>,
        maximum: [f32; 2],
    ) {
        let available = Vec2::new(maximum[0].min(ui.available_width()), maximum[1]);
        egui::Frame::new()
            .fill(Color32::from_rgb(12, 14, 18))
            .corner_radius(8)
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    available,
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        if let Some(frame) = frame {
                            let texture =
                                self.textures.entry(key).or_insert_with(|| PreviewTexture {
                                    sequence: 0,
                                    texture: ui.ctx().load_texture(
                                        key,
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
                                egui::Image::new((texture.texture.id(), available))
                                    .maintain_aspect_ratio(true),
                            );
                        } else {
                            ui.label(RichText::new("No frame").color(Color32::DARK_GRAY));
                        }
                    },
                );
            });
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

fn status_row(ui: &mut egui::Ui, label: &str, state: DeviceState) {
    let color = match state {
        DeviceState::Ready => Color32::from_rgb(76, 205, 132),
        DeviceState::Initializing => Color32::from_rgb(245, 190, 75),
        DeviceState::Failed => Color32::from_rgb(235, 90, 90),
        DeviceState::Unavailable => Color32::GRAY,
    };
    ui.horizontal(|ui| {
        ui.colored_label(color, "●");
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("{state:?}"));
        });
    });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(Color32::GRAY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value);
        });
    });
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
    use std::time::Instant;

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
        for viewport in [egui::vec2(820.0, 600.0), egui::vec2(1120.0, 760.0)] {
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
