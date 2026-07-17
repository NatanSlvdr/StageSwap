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
    fn uses_safe_fallbacks() {
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
}
