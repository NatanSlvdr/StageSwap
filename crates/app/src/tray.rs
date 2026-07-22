use image::{Rgba, RgbaImage, imageops::FilterType};
use stageswap_core::{AppSnapshot, OutputMode, RunState};
use tray_icon::menu::{
    CheckMenuItem, Icon as MenuIcon, IconMenuItem, Menu, MenuEvent, MenuId, PredefinedMenuItem,
    Submenu,
};
use tray_icon::{Icon as TrayImage, TrayIcon, TrayIconBuilder};

use crate::app_icon;

pub struct Tray {
    _icon: TrayIcon,
    show: MenuId,
    automation: IconMenuItem,
    start_icon: MenuIcon,
    stop_icon: MenuIcon,
    automatic: CheckMenuItem,
    camera: CheckMenuItem,
    screen: CheckMenuItem,
    settings: MenuId,
    exit: MenuId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayAction {
    Show,
    OpenSettings,
    ToggleAutomation,
    SetMode(OutputMode),
    Exit,
}

impl Tray {
    pub fn new() -> Result<Self, String> {
        let menu = Menu::new();
        let show = IconMenuItem::new("Open StageSwap", true, Some(app_menu_icon()?), None);
        let start_icon = action_menu_icon(MenuGlyph::Start)?;
        let stop_icon = action_menu_icon(MenuGlyph::Stop)?;
        let automation =
            IconMenuItem::new("Start automation", true, Some(start_icon.clone()), None);
        let automatic = CheckMenuItem::new("Automatic", true, true, None);
        let camera = CheckMenuItem::new("Webcam only", true, false, None);
        let screen = CheckMenuItem::new("Screen only", true, false, None);
        let output_mode = Submenu::with_items("Output mode", true, &[&automatic, &camera, &screen])
            .map_err(|error| format!("could not create tray output-mode menu: {error}"))?;
        output_mode.set_icon(Some(action_menu_icon(MenuGlyph::OutputMode)?));
        let settings = IconMenuItem::new(
            "Settings",
            true,
            Some(action_menu_icon(MenuGlyph::Settings)?),
            None,
        );
        let exit = IconMenuItem::new("Exit", true, Some(action_menu_icon(MenuGlyph::Exit)?), None);
        let first_separator = PredefinedMenuItem::separator();
        let second_separator = PredefinedMenuItem::separator();
        let third_separator = PredefinedMenuItem::separator();
        menu.append_items(&[
            &show,
            &first_separator,
            &automation,
            &output_mode,
            &second_separator,
            &settings,
            &third_separator,
            &exit,
        ])
        .map_err(|error| format!("could not create tray menu: {error}"))?;

        let app_icon = app_icon::load(Some(32))?;
        let icon = TrayImage::from_rgba(app_icon.rgba, app_icon.width, app_icon.height)
            .map_err(|error| format!("could not create tray icon: {error}"))?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("StageSwap")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|error| format!("could not create tray icon: {error}"))?;
        Ok(Self {
            _icon: icon,
            show: show.id().clone(),
            automation,
            start_icon,
            stop_icon,
            automatic,
            camera,
            screen,
            settings: settings.id().clone(),
            exit: exit.id().clone(),
        })
    }

    pub fn sync(&self, snapshot: &AppSnapshot) {
        let (automation_text, automation_enabled) = automation_menu_state(snapshot.run_state);
        if self.automation.text() != automation_text {
            self.automation.set_text(automation_text);
            let icon = if matches!(
                snapshot.run_state,
                RunState::Running | RunState::Starting | RunState::Stopping
            ) {
                self.stop_icon.clone()
            } else {
                self.start_icon.clone()
            };
            self.automation.set_icon(Some(icon));
        }
        if self.automation.is_enabled() != automation_enabled {
            self.automation.set_enabled(automation_enabled);
        }
        set_checked(&self.automatic, snapshot.mode == OutputMode::Automatic);
        set_checked(&self.camera, snapshot.mode == OutputMode::ForceCamera);
        set_checked(&self.screen, snapshot.mode == OutputMode::ForceScreen);
    }

    pub fn poll(&self) -> Option<TrayAction> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id();
            if id == &self.show {
                return Some(TrayAction::Show);
            }
            if id == self.automation.id() {
                return Some(TrayAction::ToggleAutomation);
            }
            if id == self.automatic.id() {
                return Some(TrayAction::SetMode(OutputMode::Automatic));
            }
            if id == self.camera.id() {
                return Some(TrayAction::SetMode(OutputMode::ForceCamera));
            }
            if id == self.screen.id() {
                return Some(TrayAction::SetMode(OutputMode::ForceScreen));
            }
            if id == &self.settings {
                return Some(TrayAction::OpenSettings);
            }
            if id == &self.exit {
                return Some(TrayAction::Exit);
            }
        }
        None
    }
}

#[derive(Clone, Copy)]
enum MenuGlyph {
    Start,
    Stop,
    OutputMode,
    Settings,
    Exit,
}

fn app_menu_icon() -> Result<MenuIcon, String> {
    let icon = app_icon::load(Some(32))?;
    MenuIcon::from_rgba(icon.rgba, icon.width, icon.height)
        .map_err(|error| format!("could not create app menu icon: {error}"))
}

fn action_menu_icon(glyph: MenuGlyph) -> Result<MenuIcon, String> {
    const SOURCE_SIZE: u32 = 64;
    const MENU_SIZE: u32 = 32;
    let (color, contains): ([u8; 4], fn(f32, f32) -> bool) = match glyph {
        MenuGlyph::Start => ([48, 174, 105, 255], start_glyph),
        MenuGlyph::Stop => ([220, 76, 76, 255], stop_glyph),
        MenuGlyph::OutputMode => ([64, 118, 216, 255], output_mode_glyph),
        MenuGlyph::Settings => ([64, 118, 216, 255], settings_glyph),
        MenuGlyph::Exit => ([220, 76, 76, 255], exit_glyph),
    };
    let mut source = RgbaImage::new(SOURCE_SIZE, SOURCE_SIZE);
    for y in 0..SOURCE_SIZE {
        for x in 0..SOURCE_SIZE {
            if contains(x as f32 + 0.5, y as f32 + 0.5) {
                source.put_pixel(x, y, Rgba(color));
            }
        }
    }
    let icon = image::imageops::resize(&source, MENU_SIZE, MENU_SIZE, FilterType::Lanczos3);
    MenuIcon::from_rgba(icon.into_raw(), MENU_SIZE, MENU_SIZE)
        .map_err(|error| format!("could not create tray menu action icon: {error}"))
}

fn start_glyph(x: f32, y: f32) -> bool {
    (16.0..=51.0).contains(&x) && (y - 32.0).abs() <= (x - 16.0) * 0.56
}

fn stop_glyph(x: f32, y: f32) -> bool {
    let dx = (18.0 - x).max(0.0).max(x - 46.0);
    let dy = (18.0 - y).max(0.0).max(y - 46.0);
    dx * dx + dy * dy <= 16.0
}

fn output_mode_glyph(x: f32, y: f32) -> bool {
    let upper_shaft = (13.0..=49.0).contains(&x) && (y - 21.0).abs() <= 3.0;
    let upper_head = (39.0..=53.0).contains(&x) && (y - 21.0).abs() <= 53.0 - x;
    let lower_shaft = (15.0..=51.0).contains(&x) && (y - 43.0).abs() <= 3.0;
    let lower_head = (11.0..=25.0).contains(&x) && (y - 43.0).abs() <= x - 11.0;
    upper_shaft || upper_head || lower_shaft || lower_head
}

fn settings_glyph(x: f32, y: f32) -> bool {
    let dx = x - 32.0;
    let dy = y - 32.0;
    let radius = dx.hypot(dy);
    let ring = (13.0..=20.0).contains(&radius);
    let spokes = (0..8).any(|index| {
        let angle = index as f32 * std::f32::consts::FRAC_PI_4;
        let along = dx * angle.cos() + dy * angle.sin();
        let across = -dx * angle.sin() + dy * angle.cos();
        (18.0..=27.0).contains(&along) && across.abs() <= 3.5
    });
    ring || spokes
}

fn exit_glyph(x: f32, y: f32) -> bool {
    let door = distance_to_segment(x, y, 14.0, 13.0, 14.0, 51.0) <= 3.0
        || distance_to_segment(x, y, 14.0, 13.0, 38.0, 13.0) <= 3.0
        || distance_to_segment(x, y, 14.0, 51.0, 38.0, 51.0) <= 3.0;
    let arrow = distance_to_segment(x, y, 27.0, 32.0, 53.0, 32.0) <= 3.0
        || distance_to_segment(x, y, 43.0, 22.0, 53.0, 32.0) <= 3.0
        || distance_to_segment(x, y, 43.0, 42.0, 53.0, 32.0) <= 3.0;
    door || arrow
}

fn distance_to_segment(x: f32, y: f32, start_x: f32, start_y: f32, end_x: f32, end_y: f32) -> f32 {
    let segment_x = end_x - start_x;
    let segment_y = end_y - start_y;
    let length_squared = segment_x * segment_x + segment_y * segment_y;
    let amount =
        (((x - start_x) * segment_x + (y - start_y) * segment_y) / length_squared).clamp(0.0, 1.0);
    let nearest_x = start_x + amount * segment_x;
    let nearest_y = start_y + amount * segment_y;
    (x - nearest_x).hypot(y - nearest_y)
}

fn automation_menu_state(run_state: RunState) -> (&'static str, bool) {
    match run_state {
        RunState::Running | RunState::Starting => ("Stop automation", true),
        RunState::Stopping => ("Stopping automation…", false),
        RunState::Stopped | RunState::Error => ("Start automation", true),
    }
}

fn set_checked(item: &CheckMenuItem, checked: bool) {
    if item.is_checked() != checked {
        item.set_checked(checked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_menu_tracks_run_state() {
        assert_eq!(
            automation_menu_state(RunState::Starting),
            ("Stop automation", true)
        );
        assert_eq!(
            automation_menu_state(RunState::Running),
            ("Stop automation", true)
        );
        assert_eq!(
            automation_menu_state(RunState::Stopping),
            ("Stopping automation…", false)
        );
        assert_eq!(
            automation_menu_state(RunState::Stopped),
            ("Start automation", true)
        );
        assert_eq!(
            automation_menu_state(RunState::Error),
            ("Start automation", true)
        );
    }

    #[test]
    #[ignore = "requires an interactive Windows notification area"]
    fn tray_icon_and_all_retained_menu_actions_build() {
        let tray = Tray::new().expect("tray should build");
        assert_ne!(tray.show, *tray.automation.id());
        assert_ne!(tray.automatic.id(), tray.camera.id());
        assert_ne!(tray.camera.id(), tray.screen.id());
        assert_ne!(tray.settings, tray.exit);
    }
}
