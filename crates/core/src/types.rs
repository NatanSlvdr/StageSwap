use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Automatic,
    ForceCamera,
    ForceScreen,
    ForcePip,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Source {
    #[default]
    Camera,
    Screen,
    Placeholder,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutputLayout {
    #[default]
    Camera,
    Screen,
    WebcamMainScreenPip,
    ScreenMainWebcamPip,
    Placeholder,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DetectionState {
    #[default]
    Unknown,
    Matching,
    NotMatching,
    ReferenceMissing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeviceState {
    #[default]
    Unavailable,
    Initializing,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MonitorDescriptor {
    pub display_name: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorScore {
    pub monitor: MonitorDescriptor,
    pub similarity: f64,
    pub capture_valid: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VideoDeviceChoice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceAvailability {
    pub camera_ready: bool,
    pub screen_ready: bool,
}
