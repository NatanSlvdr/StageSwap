use crate::ScreenInput;
use asc_core::{Frame, MonitorDescriptor, Size};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame as CaptureFrame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

#[derive(Default)]
struct Shared {
    latest: Mutex<Option<Arc<Frame>>>,
    sequence: AtomicU64,
}

struct CaptureHandler {
    shared: Arc<Shared>,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = Arc<Shared>;
    type Error = String;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            shared: context.flags,
            scratch: Vec::new(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut CaptureFrame,
        _control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let timestamp = frame.timestamp().map_or(0, |time| time.Duration);
        let width = frame.width();
        let height = frame.height();
        let buffer = frame.buffer().map_err(|error| error.to_string())?;
        let pixels = buffer.as_nopadding_buffer(&mut self.scratch);
        let sequence = self.shared.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let frame = Frame::new(
            pixels.to_vec().into(),
            Size::new(width, height),
            width * 4,
            sequence,
            timestamp,
            Instant::now(),
        )
        .map_err(|error| format!("invalid screen frame: {error:?}"))?;
        *self
            .shared
            .latest
            .lock()
            .map_err(|_| "screen frame state is poisoned")? = Some(Arc::new(frame));
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        if let Ok(mut latest) = self.shared.latest.lock() {
            *latest = None;
        }
        Ok(())
    }
}

pub struct WindowsGraphicsScreenInput {
    shared: Arc<Shared>,
    control: Option<CaptureControl<CaptureHandler, String>>,
}

impl Default for WindowsGraphicsScreenInput {
    fn default() -> Self {
        Self {
            shared: Arc::new(Shared::default()),
            control: None,
        }
    }
}

impl ScreenInput for WindowsGraphicsScreenInput {
    fn enumerate(&self) -> Result<Vec<MonitorDescriptor>, String> {
        let primary = Monitor::primary().ok();
        let mut monitors = Monitor::enumerate()
            .map_err(|error| format!("could not enumerate monitors: {error}"))?;
        monitors.sort_by_key(|monitor| u8::from(Some(*monitor) != primary));
        monitors.into_iter().map(describe_monitor).collect()
    }

    fn start(
        &mut self,
        descriptor: &MonitorDescriptor,
        cursor_visible: bool,
    ) -> Result<(), String> {
        self.stop();
        let monitor = Monitor::enumerate()
            .map_err(|error| format!("could not enumerate monitors: {error}"))?
            .into_iter()
            .find(|monitor| {
                monitor
                    .device_name()
                    .is_ok_and(|name| name == descriptor.display_name)
            })
            .ok_or_else(|| format!("monitor {} is no longer available", descriptor.display_name))?;
        let cursor = if cursor_visible {
            CursorCaptureSettings::WithCursor
        } else {
            CursorCaptureSettings::WithoutCursor
        };
        let settings = Settings::new(
            monitor,
            cursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(std::time::Duration::from_millis(16)),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            Arc::clone(&self.shared),
        );
        self.control = Some(
            CaptureHandler::start_free_threaded(settings)
                .map_err(|error| format!("could not start Windows Graphics Capture: {error}"))?,
        );
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
        if let Ok(mut latest) = self.shared.latest.lock() {
            *latest = None;
        }
    }

    fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.shared
            .latest
            .lock()
            .ok()
            .and_then(|frame| frame.clone())
    }
}

impl Drop for WindowsGraphicsScreenInput {
    fn drop(&mut self) {
        self.stop();
    }
}

fn describe_monitor(monitor: Monitor) -> Result<MonitorDescriptor, String> {
    let display_name = monitor
        .device_name()
        .map_err(|error| format!("could not read monitor device name: {error}"))?;
    let label = monitor.name().unwrap_or_else(|_| display_name.clone());
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    // SAFETY: the wrapper exposes a live HMONITOR and info is writable.
    unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), (&raw mut info).cast()) }
        .ok()
        .map_err(|error| format!("could not read monitor bounds: {error}"))?;
    Ok(MonitorDescriptor {
        display_name,
        label,
        x: info.rcMonitor.left,
        y: info.rcMonitor.top,
        width: (info.rcMonitor.right - info.rcMonitor.left) as u32,
        height: (info.rcMonitor.bottom - info.rcMonitor.top) as u32,
    })
}
