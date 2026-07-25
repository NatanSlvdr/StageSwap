use crate::{
    AppConfig, DetectionState, DeviceState, Frame, MonitorDescriptor, OutputMode, Source,
    SourceAvailability, TransitionState, VideoDeviceChoice,
};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RunState {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartTarget {
    Webcam,
    ScreenCapture,
    VirtualCamera,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Start,
    Stop,
    ToggleDisco,
    SetMode(OutputMode),
    UpdateSettings(Box<AppConfig>),
    CaptureReference,
    ImportReference(PathBuf),
    SelectMonitor(MonitorDescriptor),
    RefreshVideoDevices,
    Rescan,
    Restart(RestartTarget),
    Exit,
}

#[derive(Clone, Debug, Default)]
pub struct PreviewFrames {
    pub final_output: Option<Arc<Frame>>,
    pub webcam: Option<Arc<Frame>>,
    pub screen: Option<Arc<Frame>>,
    pub reference: Option<Arc<Frame>>,
}

#[derive(Clone, Debug, Default)]
pub struct AppSnapshot {
    pub run_state: RunState,
    pub disco_enabled: bool,
    pub mode: OutputMode,
    pub detection: DetectionState,
    pub automatic_target: Source,
    pub actual_output: Source,
    pub transition: TransitionState,
    pub availability: SourceAvailability,
    pub webcam_state: DeviceState,
    pub screen_state: DeviceState,
    pub virtual_camera_state: DeviceState,
    pub webcam_fps: Option<u32>,
    pub screen_fps: Option<u32>,
    pub output_fps: Option<u32>,
    pub warning: Option<String>,
    pub recent_activity: Arc<[String]>,
    pub previews: PreviewFrames,
    pub video_devices: Arc<[VideoDeviceChoice]>,
    pub selected_video_device_id: String,
    pub monitors: Arc<[MonitorDescriptor]>,
    pub selected_monitor: Option<MonitorDescriptor>,
}
