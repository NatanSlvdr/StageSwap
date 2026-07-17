use asc_core::{Command, OutputMode};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    _icon: TrayIcon,
    show: MenuId,
    automatic: MenuId,
    camera: MenuId,
    screen: MenuId,
    exit: MenuId,
}

pub enum TrayAction {
    Show,
    Command(Command),
    Exit,
}

impl Tray {
    pub fn new() -> Result<Self, String> {
        let menu = Menu::new();
        let show = MenuItem::new("Open Automatic Screen Camera", true, None);
        let automatic = MenuItem::new("Automatic mode", true, None);
        let camera = MenuItem::new("Force webcam", true, None);
        let screen = MenuItem::new("Force screen", true, None);
        let exit = MenuItem::new("Exit", true, None);
        menu.append_items(&[&show, &automatic, &camera, &screen, &exit])
            .map_err(|error| format!("could not create tray menu: {error}"))?;
        let icon = Icon::from_rgba(icon_pixels(), 32, 32)
            .map_err(|error| format!("could not create tray icon: {error}"))?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Automatic Screen Camera")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()
            .map_err(|error| format!("could not create tray icon: {error}"))?;
        Ok(Self {
            _icon: tray,
            show: show.id().clone(),
            automatic: automatic.id().clone(),
            camera: camera.id().clone(),
            screen: screen.id().clone(),
            exit: exit.id().clone(),
        })
    }

    pub fn poll(&self) -> Option<TrayAction> {
        let event = MenuEvent::receiver().try_recv().ok()?;
        let id = event.id();
        if id == &self.show {
            Some(TrayAction::Show)
        } else if id == &self.automatic {
            Some(TrayAction::Command(Command::SetMode(OutputMode::Automatic)))
        } else if id == &self.camera {
            Some(TrayAction::Command(Command::SetMode(
                OutputMode::ForceCamera,
            )))
        } else if id == &self.screen {
            Some(TrayAction::Command(Command::SetMode(
                OutputMode::ForceScreen,
            )))
        } else if id == &self.exit {
            Some(TrayAction::Exit)
        } else {
            None
        }
    }
}

fn icon_pixels() -> Vec<u8> {
    let mut pixels = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let offset = (y * 32 + x) * 4;
            let inside = (4..28).contains(&x) && (7..25).contains(&y);
            pixels[offset..offset + 4].copy_from_slice(if inside {
                &[58, 135, 255, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows notification area"]
    fn tray_icon_and_all_retained_menu_actions_build() {
        let tray = Tray::new().expect("tray should build");
        assert_ne!(tray.show, tray.automatic);
        assert_ne!(tray.automatic, tray.camera);
        assert_ne!(tray.camera, tray.screen);
        assert_ne!(tray.screen, tray.exit);
    }
}
