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
pub mod sha256;
pub mod stillness;
pub mod transition;
pub mod types;

pub use config::{
    AdminProfileStatus, AdminProfileStore, AdminRestoreOutcome, AppConfig, ConfigLoad, ConfigStore,
    StillImagePipLayout, UpdateChannel,
};
pub use decision::{Decision, decide};
pub use detector::{DebouncedDetector, DetectorSettings};
pub use frame::{
    CAPTURE_FRAME_POOL_CAPACITY, FRAME_STALE_AFTER, Frame, FrameBufferPool, FrameCompositor,
    FrameError, FrameMetadata, PIPELINE_FPS, PIPELINE_SIZE, PipComposition,
    STILL_IMAGE_PIP_CORNER_RADIUS, STILL_IMAGE_PIP_MARGIN, STILL_IMAGE_PIP_SIZE,
    aspect_fit_bgra_into, off_frame, off_frame_pixels,
};
pub use image::{
    GrayImage, ImageError, bgra_to_gray, image_similarity, resize_bgra_to_gray, resize_bilinear,
};
pub use ipc::{
    CacheError, FRAME_EXPIRY, FrameHeader, HEADER_LEN, HeaderError, MAX_FRAME_BYTES,
    SharedFrameCache, is_expected_pipe_disconnect,
};
pub use monitor::{MonitorTracker, MonitorTrackerSettings, MonitorTrackingResult};
pub use pacer::FramePacer;
pub use runtime::{
    AppSnapshot, Command, ComponentFailureKind, ComponentLifecycle, ComponentStatus, PreviewFrames,
    RestartTarget, RunState, ScreenFailureKind, WebcamFailureKind,
};
pub use sha256::{hex as hex_digest, sha256};
pub use stillness::StillImageDetector;
pub use transition::{TransitionController, TransitionState};
pub use types::*;
