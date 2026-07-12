#include "asc/core/monitor_tracker.hpp"

#include <algorithm>
#include <cmath>
#include <stdexcept>

namespace asc {

MonitorTracker::MonitorTracker(const MonitorTrackerSettings settings) : settings_(settings) {
    if (settings_.match_threshold < 0.0 || settings_.match_threshold > 1.0 ||
        settings_.reassignment_margin < 0.0 || settings_.confirmations_required == 0) {
        throw std::invalid_argument("invalid monitor tracker settings");
    }
}

void MonitorTracker::configure(const MonitorTrackerSettings settings) {
    if (settings.match_threshold < 0.0 || settings.match_threshold > 1.0 || settings.reassignment_margin < 0.0 || settings.confirmations_required == 0)
        throw std::invalid_argument("invalid monitor tracker settings");
    settings_ = settings;
    pending_key_.clear();
    pending_confirmations_ = 0;
}

void MonitorTracker::restore_preferred(MonitorIdentity identity) {
    tracked_ = std::move(identity);
    pending_key_.clear();
    pending_confirmations_ = 0;
}

bool MonitorTracker::same_monitor(const MonitorIdentity& a, const MonitorIdentity& b) {
    if (!a.hardware_id.empty() && !b.hardware_id.empty() && a.hardware_id == b.hardware_id) {
        return a.serial.empty() || b.serial.empty() || a.serial == b.serial;
    }
    return !a.device_path.empty() && a.device_path == b.device_path;
}

int MonitorTracker::identity_affinity(const MonitorIdentity& candidate) const {
    if (!tracked_) return 0;
    if (!tracked_->hardware_id.empty() && tracked_->hardware_id == candidate.hardware_id) return 600;
    if (!tracked_->device_path.empty() && tracked_->device_path == candidate.device_path) return 500;
    int score = 0;
    if (tracked_->manufacturer == candidate.manufacturer && tracked_->model == candidate.model) score += 300;
    if (tracked_->resolution == candidate.resolution && tracked_->orientation_degrees == candidate.orientation_degrees) score += 100;
    if (tracked_->desktop_x == candidate.desktop_x && tracked_->desktop_y == candidate.desktop_y) score += 50;
    return score;
}

MonitorTrackingResult MonitorTracker::apply_scan(const std::vector<MonitorScore>& scores) {
    MonitorTrackingResult result;
    result.tracked = tracked_;
    std::vector<const MonitorScore*> valid;
    for (const auto& score : scores) {
        if (score.capture_valid && score.similarity >= settings_.match_threshold) valid.push_back(&score);
    }
    if (valid.empty()) {
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.scan_state = DetectionState::reference_missing;
        result.message = "reference not found; retaining last tracked monitor";
        return result;
    }

    std::sort(valid.begin(), valid.end(), [](const MonitorScore* a, const MonitorScore* b) { return a->similarity > b->similarity; });
    const double highest_score = valid.front()->similarity;
    const auto near_end = std::find_if(valid.begin(), valid.end(), [&](const MonitorScore* score) {
        return highest_score - score->similarity > settings_.reassignment_margin;
    });
    if (tracked_) {
        const auto current = std::find_if(valid.begin(), near_end, [&](const MonitorScore* score) { return same_monitor(score->monitor, *tracked_); });
        if (current != near_end) std::iter_swap(valid.begin(), current);
        else {
            const auto affinity = std::max_element(valid.begin(), near_end, [this](const MonitorScore* a, const MonitorScore* b) {
                return identity_affinity(a->monitor) < identity_affinity(b->monitor);
            });
            if (affinity != near_end) std::iter_swap(valid.begin(), affinity);
        }
    }
    const auto& best = *valid.front();
    result.best_similarity = best.similarity;

    const bool duplicate = std::distance(valid.begin(), near_end) > 1;
    if (duplicate && (!tracked_ || !same_monitor(best.monitor, *tracked_))) {
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.scan_state = DetectionState::ambiguous;
        result.message = "duplicate reference matches are ambiguous; tracked monitor unchanged";
        return result;
    }
    result.scan_state = duplicate ? DetectionState::ambiguous : DetectionState::matching;

    if (tracked_ && same_monitor(best.monitor, *tracked_)) {
        tracked_ = best.monitor; // Refresh transient geometry and resolution.
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.tracked = tracked_;
        result.message = duplicate ? "duplicate reference detected; retained tracked monitor" : "reference confirmed on tracked monitor";
        return result;
    }

    double current_score = -1.0;
    if (tracked_) {
        for (const auto& score : scores) {
            if (same_monitor(score.monitor, *tracked_) && score.capture_valid) current_score = score.similarity;
        }
    }
    if (current_score >= 0.0 && best.similarity < current_score + settings_.reassignment_margin) {
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.scan_state = DetectionState::ambiguous;
        result.message = "new match does not clearly exceed current monitor; tracked monitor unchanged";
        return result;
    }

    const auto key = best.monitor.stable_key();
    if (key == pending_key_) ++pending_confirmations_;
    else {
        pending_key_ = key;
        pending_confirmations_ = 1;
    }
    if (pending_confirmations_ < settings_.confirmations_required) {
        result.message = "candidate monitor awaiting confirmation";
        return result;
    }

    tracked_ = best.monitor;
    result.tracked = tracked_;
    result.changed = true;
    result.message = "tracked monitor reassigned after repeated valid scans";
    pending_key_.clear();
    pending_confirmations_ = 0;
    return result;
}

} // namespace asc
