use crate::{
    AppConfig, DetectionState, DeviceState, Frame, MonitorDescriptor, OutputLayout, OutputMode,
    Source, SourceAvailability, TransitionState, VideoDeviceChoice,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RunState {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RestartTarget {
    Webcam,
    ScreenCapture,
    VirtualCamera,
    All,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ComponentLifecycle {
    #[default]
    Stopped,
    Starting,
    WaitingForFirstFrame,
    Ready,
    Stale,
    Restarting,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebcamFailureKind {
    AccessDenied,
    PrivacyDisabled,
    DeviceBusy,
    DeviceInvalidated,
    UnsupportedFormat,
    IncompatibleTypeChange,
    DriverFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenFailureKind {
    UnsupportedCapture,
    MissingSelectedMonitor,
    Closed,
    UnsupportedHdr,
    CaptureFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentFailureKind {
    Webcam(WebcamFailureKind),
    Screen(ScreenFailureKind),
    Publisher,
    VirtualCamera,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComponentStatus {
    pub lifecycle: ComponentLifecycle,
    pub state_since: Option<Instant>,
    pub last_success_at: Option<Instant>,
    pub first_frame_deadline: Option<Instant>,
    pub last_failure: Option<String>,
    pub last_failure_kind: Option<ComponentFailureKind>,
    pub consecutive_restart_failures: u32,
    pub next_permitted_retry: Option<Instant>,
}

impl ComponentStatus {
    pub fn transition(&mut self, lifecycle: ComponentLifecycle, now: Instant) {
        if self.lifecycle != lifecycle {
            self.lifecycle = lifecycle;
            self.state_since = Some(now);
        }
    }

    pub fn waiting_for_first_frame(&mut self, now: Instant, deadline: Instant) {
        self.transition(ComponentLifecycle::WaitingForFirstFrame, now);
        self.first_frame_deadline = Some(deadline);
        self.last_failure = None;
        self.last_failure_kind = None;
    }

    pub fn mark_ready(&mut self, now: Instant) {
        self.transition(ComponentLifecycle::Ready, now);
        self.last_success_at = Some(now);
        self.first_frame_deadline = None;
        self.last_failure = None;
        self.last_failure_kind = None;
        self.consecutive_restart_failures = 0;
        self.next_permitted_retry = None;
    }

    pub fn mark_failed(&mut self, now: Instant, failure: impl Into<String>) {
        self.transition(ComponentLifecycle::Failed, now);
        self.first_frame_deadline = None;
        self.last_failure = Some(failure.into());
        self.last_failure_kind = None;
    }

    pub fn mark_failed_with_kind(
        &mut self,
        now: Instant,
        kind: ComponentFailureKind,
        failure: impl Into<String>,
    ) {
        self.mark_failed(now, failure);
        self.last_failure_kind = Some(kind);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Start,
    Stop,
    ToggleDisco,
    SetMode(OutputMode),
    UpdateSettings(Box<AppConfig>),
    ReloadSettings(Box<AppConfig>),
    CaptureReference,
    CaptureReferenceCandidate,
    ConfirmReferenceCandidate,
    DiscardReferenceCandidate,
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
    pub reference_candidate: Option<Arc<Frame>>,
}

#[derive(Clone, Debug, Default)]
pub struct AppSnapshot {
    pub run_state: RunState,
    pub disco_enabled: bool,
    pub mode: OutputMode,
    pub detection: DetectionState,
    pub automatic_target: Source,
    pub actual_output: Source,
    pub output_layout: OutputLayout,
    pub still_image_pip_active: bool,
    pub still_image_pip_mix: f64,
    pub transition: TransitionState,
    pub availability: SourceAvailability,
    pub webcam_state: DeviceState,
    pub screen_state: DeviceState,
    pub virtual_camera_state: DeviceState,
    pub webcam_component: ComponentStatus,
    pub screen_component: ComponentStatus,
    pub publisher_component: ComponentStatus,
    pub virtual_camera_component: ComponentStatus,
    pub webcam_fps: Option<u32>,
    pub screen_fps: Option<u32>,
    pub output_fps: Option<u32>,
    pub output_deadline_misses: u64,
    pub warning: Option<String>,
    pub recent_activity: Arc<[String]>,
    pub recent_activity_first_id: u64,
    pub previews: PreviewFrames,
    pub video_devices: Arc<[VideoDeviceChoice]>,
    pub selected_video_device_id: String,
    pub webcam_native_format: Option<String>,
    pub webcam_output_format: Option<String>,
    pub monitors: Arc<[MonitorDescriptor]>,
    pub selected_monitor: Option<MonitorDescriptor>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn flow_component_readiness_clears_deadline_and_restart_backoff() {
        let now = Instant::now();
        let mut status = ComponentStatus::default();
        status.waiting_for_first_frame(now, now + Duration::from_secs(3));
        status.consecutive_restart_failures = 2;
        status.next_permitted_retry = Some(now + Duration::from_secs(10));

        status.mark_ready(now + Duration::from_millis(50));

        assert_eq!(status.lifecycle, ComponentLifecycle::Ready);
        assert_eq!(
            status.last_success_at,
            Some(now + Duration::from_millis(50))
        );
        assert_eq!(status.first_frame_deadline, None);
        assert_eq!(status.consecutive_restart_failures, 0);
        assert_eq!(status.next_permitted_retry, None);
    }

    #[test]
    fn flow_component_failure_keeps_diagnostic_context() {
        let now = Instant::now();
        let mut status = ComponentStatus::default();
        status.waiting_for_first_frame(now, now + Duration::from_secs(2));
        status.mark_failed(now + Duration::from_secs(2), "first frame timed out");

        assert_eq!(status.lifecycle, ComponentLifecycle::Failed);
        assert_eq!(status.first_frame_deadline, None);
        assert_eq!(
            status.last_failure.as_deref(),
            Some("first frame timed out")
        );
    }
}
