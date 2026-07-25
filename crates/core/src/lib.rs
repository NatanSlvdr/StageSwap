#![forbid(unsafe_code)]

pub mod config;
pub mod decision;
pub mod detector;
pub mod frame;
pub mod image;
pub mod ipc;
pub mod monitor;
pub mod pacer;
pub mod runtime;
pub mod transition;
pub mod types;

pub use config::{
    AdminProfileStatus, AdminProfileStore, AdminRestoreOutcome, AppConfig, ConfigLoad, ConfigStore,
};
pub use decision::{Decision, decide};
pub use detector::{DebouncedDetector, DetectorSettings};
pub use frame::{
    FRAME_STALE_AFTER, Frame, FrameCompositor, FrameError, FrameMetadata, PIPELINE_FPS,
    PIPELINE_SIZE, off_frame, off_frame_pixels,
};
pub use image::{
    GrayImage, ImageError, bgra_to_gray, image_similarity, resize_bgra_to_gray, resize_bilinear,
};
pub use ipc::{
    CacheError, FRAME_EXPIRY, FrameHeader, HEADER_LEN, HeaderError, MAX_FRAME_BYTES,
    SharedFrameCache,
};
pub use monitor::{MonitorTracker, MonitorTrackerSettings, MonitorTrackingResult};
pub use pacer::FramePacer;
pub use runtime::{AppSnapshot, Command, PreviewFrames, RestartTarget, RunState};
pub use transition::{TransitionController, TransitionState};
pub use types::*;
