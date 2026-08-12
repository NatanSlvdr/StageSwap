use crate::{DetectionState, OutputMode, Source, SourceAvailability};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Decision {
    pub automatic_target: Source,
    pub desired_output: Source,
    pub manual_override: bool,
    pub reason: &'static str,
}

fn available(requested: Source, availability: SourceAvailability) -> Source {
    match requested {
        Source::Camera if availability.camera_ready => Source::Camera,
        Source::Screen if availability.screen_ready => Source::Screen,
        Source::Screen if availability.camera_ready => Source::Camera,
        _ => Source::Placeholder,
    }
}

pub fn decide(
    mode: OutputMode,
    detection: DetectionState,
    availability: SourceAvailability,
) -> Decision {
    let automatic_target = match detection {
        DetectionState::NotMatching => Source::Screen,
        DetectionState::Unknown | DetectionState::Matching | DetectionState::ReferenceMissing => {
            Source::Camera
        }
    };
    let (requested, manual_override, reason) = match mode {
        OutputMode::Automatic if detection == DetectionState::ReferenceMissing => {
            (Source::Camera, false, "reference unavailable")
        }
        OutputMode::Automatic => (automatic_target, false, "automatic detection"),
        OutputMode::ForceCamera => (Source::Camera, true, "camera forced"),
        OutputMode::ForceScreen => (Source::Screen, true, "screen forced"),
    };
    Decision {
        automatic_target,
        desired_output: available(requested, availability),
        manual_override,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_uses_safe_fallbacks() {
        assert_eq!(
            decide(
                OutputMode::Automatic,
                DetectionState::ReferenceMissing,
                SourceAvailability {
                    camera_ready: true,
                    screen_ready: true
                }
            )
            .desired_output,
            Source::Camera
        );
        assert_eq!(
            decide(
                OutputMode::ForceScreen,
                DetectionState::NotMatching,
                SourceAvailability {
                    camera_ready: true,
                    screen_ready: false
                }
            )
            .desired_output,
            Source::Camera
        );
        assert_eq!(
            decide(
                OutputMode::ForceCamera,
                DetectionState::Matching,
                SourceAvailability {
                    camera_ready: false,
                    screen_ready: true
                }
            )
            .desired_output,
            Source::Placeholder
        );
    }

    #[test]
    fn contract_exhaustive_modes_detections_and_availability_have_safe_outputs() {
        let modes = [
            OutputMode::Automatic,
            OutputMode::ForceCamera,
            OutputMode::ForceScreen,
        ];
        let detections = [
            DetectionState::Unknown,
            DetectionState::Matching,
            DetectionState::NotMatching,
            DetectionState::ReferenceMissing,
        ];
        let availability = [
            SourceAvailability {
                camera_ready: false,
                screen_ready: false,
            },
            SourceAvailability {
                camera_ready: true,
                screen_ready: false,
            },
            SourceAvailability {
                camera_ready: false,
                screen_ready: true,
            },
            SourceAvailability {
                camera_ready: true,
                screen_ready: true,
            },
        ];

        for mode in modes {
            for detection in detections {
                for sources in availability {
                    let decision = decide(mode, detection, sources);
                    let expected_automatic = if detection == DetectionState::NotMatching {
                        Source::Screen
                    } else {
                        Source::Camera
                    };
                    assert_eq!(decision.automatic_target, expected_automatic);
                    assert_eq!(
                        decision.manual_override,
                        !matches!(mode, OutputMode::Automatic)
                    );
                    let requested = match mode {
                        OutputMode::Automatic => expected_automatic,
                        OutputMode::ForceCamera => Source::Camera,
                        OutputMode::ForceScreen => Source::Screen,
                    };
                    let expected_output = match requested {
                        Source::Camera if sources.camera_ready => Source::Camera,
                        Source::Screen if sources.screen_ready => Source::Screen,
                        Source::Screen if sources.camera_ready => Source::Camera,
                        _ => Source::Placeholder,
                    };
                    assert_eq!(decision.desired_output, expected_output);
                    assert!(!decision.reason.is_empty());
                }
            }
        }
    }
}
