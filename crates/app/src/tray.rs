use stageswap_core::{AppSnapshot, OutputMode, RunState};
use tray_icon::menu::{
    CheckMenuItem, Icon as MenuIcon, IconMenuItem, Menu, MenuEvent, MenuId, PredefinedMenuItem,
    Submenu,
};
use tray_icon::{Icon as TrayImage, TrayIcon, TrayIconBuilder};

use crate::{
    app_icon,
    ui_icon::{self, UiIcon},
};

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
        let start_icon = action_menu_icon(UiIcon::Play, [48, 174, 105, 255])?;
        let stop_icon = action_menu_icon(UiIcon::Stop, [220, 76, 76, 255])?;
        let automation =
            IconMenuItem::new("Start automation", true, Some(start_icon.clone()), None);
        let automatic = CheckMenuItem::new("Automatic", true, true, None);
        let camera = CheckMenuItem::new("Webcam only", true, false, None);
        let screen = CheckMenuItem::new("Screen only", true, false, None);
        let output_mode = Submenu::with_items("Output mode", true, &[&automatic, &camera, &screen])
            .map_err(|error| format!("could not create tray output-mode menu: {error}"))?;
        output_mode.set_icon(Some(action_menu_icon(UiIcon::Route, [64, 118, 216, 255])?));
        let settings = IconMenuItem::new(
            "Settings",
            true,
            Some(action_menu_icon(UiIcon::Settings, [64, 118, 216, 255])?),
            None,
        );
        let exit = IconMenuItem::new(
            "Exit",
            true,
            Some(action_menu_icon(UiIcon::SignOut, [220, 76, 76, 255])?),
            None,
        );
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

fn app_menu_icon() -> Result<MenuIcon, String> {
    let icon = app_icon::load(Some(32))?;
    MenuIcon::from_rgba(icon.rgba, icon.width, icon.height)
        .map_err(|error| format!("could not create app menu icon: {error}"))
}

fn action_menu_icon(icon: UiIcon, color: [u8; 4]) -> Result<MenuIcon, String> {
    const MENU_SIZE: u32 = 32;
    let image = ui_icon::rasterize(icon, color, MENU_SIZE)?;
    MenuIcon::from_rgba(image.into_raw(), MENU_SIZE, MENU_SIZE)
        .map_err(|error| format!("could not create tray menu action icon: {error}"))
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
