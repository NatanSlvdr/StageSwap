#include "asc/core/detector.hpp"

#include <algorithm>
#include <stdexcept>

namespace asc {

DebouncedDetector::DebouncedDetector(const DetectorSettings settings) : settings_(settings) {
    if (settings_.threshold < 0.0 || settings_.threshold > 1.0 ||
        settings_.matches_required == 0 || settings_.mismatches_required == 0) {
        throw std::invalid_argument("invalid detector settings");
    }
}

void DebouncedDetector::configure(const DetectorSettings settings) {
    if (settings.threshold < 0.0 || settings.threshold > 1.0 || settings.matches_required == 0 || settings.mismatches_required == 0)
        throw std::invalid_argument("invalid detector settings");
    settings_ = settings;
    snapshot_.consecutive_matches = 0;
    snapshot_.consecutive_mismatches = 0;
}

DetectionSnapshot DebouncedDetector::update(const double similarity, const bool capture_valid, const TimePoint now) {
    snapshot_.similarity = std::clamp(similarity, 0.0, 1.0);
    snapshot_.measured_at = now;
    if (!capture_valid) {
        snapshot_.state = DetectionState::reference_missing;
        snapshot_.consecutive_matches = 0;
        snapshot_.consecutive_mismatches = 0;
        return snapshot_;
    }
    if (snapshot_.similarity >= settings_.threshold) {
        snapshot_.consecutive_mismatches = 0;
        ++snapshot_.consecutive_matches;
        if (snapshot_.consecutive_matches >= settings_.matches_required) {
            snapshot_.state = DetectionState::matching;
        }
    } else {
        snapshot_.consecutive_matches = 0;
        ++snapshot_.consecutive_mismatches;
        if (snapshot_.consecutive_mismatches >= settings_.mismatches_required) {
            snapshot_.state = DetectionState::not_matching;
        }
    }
    return snapshot_;
}

void DebouncedDetector::reset(const DetectionState state) {
    snapshot_ = {};
    snapshot_.state = state;
}

} // namespace asc
