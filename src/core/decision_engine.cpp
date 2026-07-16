#include "asc/core/decision_engine.hpp"

namespace asc {

Source DecisionEngine::safe_available(const Source requested, const SourceAvailability& available) {
    if (requested == Source::camera && available.camera_ready) return Source::camera;
    if (requested == Source::screen && available.screen_ready) return Source::screen;
    if (available.camera_ready) return Source::camera; // Privacy-preserving fallback.
    return Source::placeholder;
}

Decision DecisionEngine::decide(const OutputMode mode, const DetectionState detection, const Source,
                                const SourceAvailability& availability) const {
    Source automatic = Source::camera;
    std::string reason;
    if (detection == DetectionState::matching) {
        automatic = Source::camera;
        reason = "reference detected";
    } else if (detection == DetectionState::not_matching) {
        automatic = Source::screen;
        reason = "reference absent";
    } else {
        automatic = Source::camera;
        reason = "reference unavailable";
    }

    Source requested = automatic;
    if (mode == OutputMode::force_camera) requested = Source::camera;
    if (mode == OutputMode::force_screen) requested = Source::screen;
    const auto desired = safe_available(requested, availability);
    if (desired != requested) reason += "; requested source unavailable, safe fallback selected";
    return {automatic, desired, mode != OutputMode::automatic, std::move(reason)};
}

} // namespace asc
