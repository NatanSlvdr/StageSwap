#![cfg_attr(not(windows), forbid(unsafe_code))]

use asc_core::{Frame, MonitorDescriptor};
use std::sync::Arc;

#[cfg(windows)]
mod pipe;
#[cfg(windows)]
pub use pipe::FramePublisher;
#[cfg(windows)]
mod virtual_camera;
#[cfg(windows)]
pub use virtual_camera::{VirtualCameraController, frame_pipe_name, remove_virtual_camera};
#[cfg(windows)]
mod video_input;
#[cfg(windows)]
pub use video_input::{MediaFoundationVideoInput, enumerate_video_devices};
#[cfg(windows)]
mod deployment;
#[cfg(windows)]
pub use deployment::{configure_startup, portable_startup, previous_install_present};
#[cfg(windows)]
mod notification;
#[cfg(windows)]
pub use notification::{notify_warning, show_error_dialog};
#[cfg(windows)]
mod shell_ui;
#[cfg(windows)]
pub use shell_ui::{open_directory, pick_log_export_path, pick_reference_image};
#[cfg(windows)]
mod screen_input;
#[cfg(windows)]
pub use deployment::save_config_atomic;
#[cfg(windows)]
pub use screen_input::WindowsGraphicsScreenInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_virtual: bool,
}

pub trait VideoInput: Send {
    fn enumerate(&self) -> Result<Vec<InputDevice>, String>;
    fn start(&mut self, device_id: &str) -> Result<(), String>;
    fn stop(&mut self);
    fn latest_frame(&self) -> Option<Arc<Frame>>;
}

pub trait ScreenInput: Send {
    fn enumerate(&self) -> Result<Vec<MonitorDescriptor>, String>;
    fn start(&mut self, monitor: &MonitorDescriptor, cursor_visible: bool) -> Result<(), String>;
    fn stop(&mut self);
    fn latest_frame(&self) -> Option<Arc<Frame>>;
}

pub fn choose_video_device(
    saved_id: &str,
    devices: &[InputDevice],
    saved_opened: bool,
) -> Option<String> {
    if saved_opened && !saved_id.is_empty() {
        return Some(saved_id.into());
    }
    let mut physical = devices.iter().filter(|device| !device.is_virtual);
    let only = physical.next()?;
    if physical.next().is_none() {
        Some(only.id.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(id: &str, is_virtual: bool) -> InputDevice {
        InputDevice {
            id: id.into(),
            name: id.into(),
            is_virtual,
        }
    }
    #[test]
    fn selection_is_saved_then_unique_physical() {
        let devices = [device("virtual", true), device("physical", false)];
        assert_eq!(
            choose_video_device("saved", &devices, true).as_deref(),
            Some("saved")
        );
        assert_eq!(
            choose_video_device("missing", &devices, false).as_deref(),
            Some("physical")
        );
        let ambiguous = [device("one", false), device("two", false)];
        assert_eq!(choose_video_device("", &ambiguous, false), None);
    }
}

#[cfg(all(test, windows))]
mod interactive_windows_tests {
    use super::*;
    use std::collections::HashSet;
    use std::thread;
    use std::time::{Duration, Instant};

    fn wait_for_sequences(input: &dyn ScreenInput, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut sequences = HashSet::new();
        while sequences.len() < count && Instant::now() < deadline {
            if let Some(frame) = input.latest_frame() {
                sequences.insert(frame.sequence);
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            sequences.len(),
            count,
            "capture did not produce {count} frames"
        );
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn screen_capture_produces_300_frames_with_cursor_on_and_off_and_stops_cleanly() {
        let mut input = WindowsGraphicsScreenInput::default();
        let monitor = input.enumerate().unwrap().into_iter().next().unwrap();
        for cursor_visible in [true, false] {
            input.start(&monitor, cursor_visible).unwrap();
            wait_for_sequences(&input, 300);
            input.stop();
            assert!(input.latest_frame().is_none());
        }
    }

    #[test]
    #[ignore = "requires a physical webcam"]
    fn webcam_enumeration_excludes_virtual_camera_and_restarts_without_stale_frame() {
        let devices = enumerate_video_devices().unwrap();
        assert!(devices.iter().all(|device| !device.is_virtual));
        let device = devices.first().expect("a physical webcam is required");
        let mut input = MediaFoundationVideoInput::default();
        for _ in 0..2 {
            input.start(&device.id).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while input.latest_frame().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(input.latest_frame().is_some());
            input.stop();
            assert!(input.latest_frame().is_none());
        }
    }

    #[test]
    #[ignore = "requires the portable COM source to be installed"]
    fn virtual_camera_can_restart_manually() {
        let pipe = frame_pipe_name().unwrap();
        let mut camera = VirtualCameraController::start(pipe, 0xff17_1719).unwrap();
        assert!(camera.is_running());
        camera.restart().unwrap();
        assert!(camera.is_running());
    }
}
