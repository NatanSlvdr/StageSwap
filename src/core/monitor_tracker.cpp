#include "asc/core/monitor_tracker.hpp"

#include <algorithm>
#include <cmath>
#include <stdexcept>

namespace asc {

MonitorTracker::MonitorTracker(const MonitorTrackerSettings settings) { configure(settings); }

void MonitorTracker::configure(const MonitorTrackerSettings settings) {
    if (!std::isfinite(settings.match_threshold) || settings.match_threshold < 0.0 || settings.match_threshold > 1.0)
        throw std::invalid_argument("monitor match threshold must be between zero and one");
    settings_ = settings;
    pending_key_.clear();
    pending_confirmations_ = 0;
}

void MonitorTracker::select(RuntimeMonitorDescriptor monitor) {
    tracked_ = std::move(monitor);
    pending_key_.clear();
    pending_confirmations_ = 0;
}

bool MonitorTracker::valid_score(const MonitorScore& score) {
    return score.capture_valid && !score.monitor.gdi_display_name.empty() &&
           std::isfinite(score.similarity) && score.similarity >= 0.0 && score.similarity <= 1.0;
}

MonitorTrackingResult MonitorTracker::apply_scan(const std::vector<MonitorScore>& scores, const TimePoint) {
    MonitorTrackingResult result;
    result.tracked = tracked_;
    const auto best = std::max_element(scores.begin(), scores.end(), [](const auto& left, const auto& right) {
        const auto left_score = valid_score(left) ? left.similarity : -1.0;
        const auto right_score = valid_score(right) ? right.similarity : -1.0;
        return left_score < right_score;
    });
    if (best == scores.end() || !valid_score(*best) || best->similarity < settings_.match_threshold) {
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.scan_state = DetectionState::reference_missing;
        result.message = "reference not found; retaining the runtime-selected monitor";
        return result;
    }

    result.scan_state = DetectionState::matching;
    result.best_similarity = best->similarity;
    const auto key = best->monitor.runtime_key();
    if (tracked_ && tracked_->runtime_key() == key) {
        tracked_ = best->monitor; // Refresh geometry and handle.
        result.tracked = tracked_;
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.message = "reference confirmed on selected monitor";
        return result;
    }

    if (pending_key_ == key) ++pending_confirmations_;
    else {
        pending_key_ = key;
        pending_confirmations_ = 1;
    }
    if (pending_confirmations_ < 2) {
        result.confirmation_pending = true;
        result.message = "candidate monitor awaiting immediate confirmation scan";
        return result;
    }

    tracked_ = best->monitor;
    result.tracked = tracked_;
    result.changed = true;
    result.message = "selected highest-scoring monitor after two scans";
    pending_key_.clear();
    pending_confirmations_ = 0;
    return result;
}

} // namespace asc
