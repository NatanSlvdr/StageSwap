#![cfg_attr(windows, windows_subsystem = "windows")]

use asc_app::RuntimeHandle;
use asc_core::{
    AppConfig, AppSnapshot, Command, ConfigStore, DeviceState, Frame, OutputMode, RestartTarget,
    RunState,
};
use eframe::egui::{self, Color32, RichText, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;

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
    let store = ConfigStore::new(local_data_directory());
    let loaded = store.load();
    let start_visible = !loaded.config.start_minimized;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Automatic Screen Camera")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([820.0, 600.0])
            .with_visible(start_visible),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Automatic Screen Camera",
        options,
        Box::new(move |context| {
            context.egui_ctx.set_visuals(egui::Visuals::dark());
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
    show_previews: bool,
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
            show_previews: false,
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
        }
    }

    fn send(&self, command: Command) {
        let _ = self.runtime.try_send(command);
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
                ui.label(RichText::new("Virtual camera control center").color(Color32::GRAY));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.settings_draft = self.config.clone();
                    self.send(Command::RefreshVideoDevices);
                    self.send(Command::Rescan);
                    self.show_settings = true;
                }
                if ui.button("4 previews").clicked() {
                    self.show_previews = true;
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
        ui.columns(2, |columns| {
            columns[0].vertical(|ui| {
                ui.label(RichText::new("LIVE OUTPUT").small().color(Color32::GRAY));
                self.preview(
                    ui,
                    "final",
                    snapshot.previews.final_output.as_deref(),
                    [640.0, 360.0],
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    for (mode, label) in [
                        (OutputMode::Automatic, "Automatic"),
                        (OutputMode::ForceCamera, "Force webcam"),
                        (OutputMode::ForceScreen, "Force screen"),
                    ] {
                        if ui.selectable_label(snapshot.mode == mode, label).clicked() {
                            self.set_mode(mode);
                        }
                    }
                });
            });
            columns[1].vertical(|ui| {
                ui.heading("Status");
                status_row(ui, "Webcam", snapshot.webcam_state);
                status_row(ui, "Screen capture", snapshot.screen_state);
                status_row(ui, "Virtual camera", snapshot.virtual_camera_state);
                ui.separator();
                ui.label(format!("Selected output: {:?}", snapshot.actual_output));
                ui.label(format!("Detection: {:?}", snapshot.detection));
                ui.label(format!(
                    "Transition: {:>3}% screen",
                    (snapshot.transition.screen_mix * 100.0).round()
                ));
                ui.add_space(12.0);
                ui.horizontal(|ui| match snapshot.run_state {
                    RunState::Running | RunState::Starting => {
                        if ui.button("Stop automation").clicked() {
                            self.send(Command::Stop);
                        }
                    }
                    _ => {
                        if ui.button("Start automation").clicked() {
                            self.send(Command::Start);
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Capture reference").clicked() {
                        self.send(Command::CaptureReference);
                    }
                    if ui.button("Rescan screens").clicked() {
                        self.send(Command::Rescan);
                    }
                });
                ui.add_space(12.0);
                ui.heading("Recent activity");
                if snapshot.recent_activity.is_empty() {
                    ui.label(RichText::new("No activity yet").color(Color32::GRAY));
                }
                for activity in snapshot.recent_activity.iter().rev().take(7) {
                    ui.label(format!("• {activity}"));
                }
                ui.add_space(8.0);
                ui.collapsing("Component controls", |ui| {
                    restart_button(ui, &self.runtime, "Restart webcam", RestartTarget::Webcam);
                    restart_button(
                        ui,
                        &self.runtime,
                        "Restart screen capture",
                        RestartTarget::ScreenCapture,
                    );
                    restart_button(
                        ui,
                        &self.runtime,
                        "Restart virtual camera",
                        RestartTarget::VirtualCamera,
                    );
                    restart_button(ui, &self.runtime, "Restart all", RestartTarget::All);
                });
            });
        });
        self.settings_window(context);
        self.previews_window(context, &snapshot);
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
                        if ui.button("Open four previews").clicked() {
                            self.show_previews = true;
                        }
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

    fn previews_window(&mut self, context: &egui::Context, snapshot: &AppSnapshot) {
        if !self.show_previews {
            return;
        }
        let mut open = self.show_previews;
        egui::Window::new("Four previews")
            .open(&mut open)
            .default_size([920.0, 620.0])
            .show(context, |ui| {
                egui::Grid::new("preview-grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        self.preview_cell(
                            ui,
                            "webcam",
                            "Webcam",
                            snapshot.previews.webcam.as_deref(),
                        );
                        self.preview_cell(
                            ui,
                            "screen",
                            "Screen",
                            snapshot.previews.screen.as_deref(),
                        );
                        ui.end_row();
                        self.preview_cell(
                            ui,
                            "final-grid",
                            "Final output",
                            snapshot.previews.final_output.as_deref(),
                        );
                        self.preview_cell(
                            ui,
                            "reference",
                            "Saved reference (detection only)",
                            snapshot.previews.reference.as_deref(),
                        );
                        ui.end_row();
                    });
            });
        self.show_previews = open;
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
    ) {
        ui.vertical(|ui| {
            ui.strong(title);
            self.preview(ui, key, frame, [420.0, 236.0]);
        });
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
                ui.set_min_size(available);
                if let Some(frame) = frame {
                    let texture = self.textures.entry(key).or_insert_with(|| PreviewTexture {
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
                    ui.centered_and_justified(|ui| {
                        ui.add(
                            egui::Image::new((texture.texture.id(), available))
                                .maintain_aspect_ratio(true),
                        );
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("No frame").color(Color32::DARK_GRAY));
                    });
                }
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
    fn dashboard_banner_all_settings_tabs_and_preview_window_render() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = SwitcherApp::new(
            AppConfig {
                start_automatically: false,
                ..AppConfig::default()
            },
            vec!["Test warning banner".into()],
            ConfigStore::new(directory.path()),
        );
        app.show_previews = true;
        let context = egui::Context::default();
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
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1120.0, 760.0),
                    )),
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
                    "{tab:?} at {dpi_scale}× produced no UI shapes"
                );
            }
        }
    }
}
