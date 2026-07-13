#include "asc/core/monitor_tracker.hpp"

#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <unordered_set>

namespace asc {
namespace {
constexpr std::size_t maximum_history_entries = 256;
}

MonitorTracker::MonitorTracker(const MonitorTrackerSettings settings) : settings_(settings) {
    if (!std::isfinite(settings_.match_threshold) || settings_.match_threshold < 0.0 || settings_.match_threshold > 1.0 ||
        !std::isfinite(settings_.reassignment_margin) || settings_.reassignment_margin < 0.0 || settings_.reassignment_margin > 1.0 ||
        settings_.confirmations_required == 0) {
        throw std::invalid_argument("invalid monitor tracker settings");
    }
}

void MonitorTracker::configure(const MonitorTrackerSettings settings) {
    if (!std::isfinite(settings.match_threshold) || settings.match_threshold < 0.0 || settings.match_threshold > 1.0 ||
        !std::isfinite(settings.reassignment_margin) || settings.reassignment_margin < 0.0 || settings.reassignment_margin > 1.0 ||
        settings.confirmations_required == 0)
        throw std::invalid_argument("invalid monitor tracker settings");
    settings_ = settings;
    pending_key_.clear();
    pending_confirmations_ = 0;
}

void MonitorTracker::restore_preferred(MonitorIdentity identity) {
    tracked_ = std::move(identity);
    mark_previously_tracked(*tracked_);
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

bool MonitorTracker::valid_score(const MonitorScore& score) {
    return score.capture_valid && std::isfinite(score.similarity) && score.similarity >= 0.0 && score.similarity <= 1.0;
}

void MonitorTracker::mark_previously_tracked(const MonitorIdentity& identity) {
    const auto key = identity.stable_key();
    auto& remembered = history_[key];
    remembered.identity = identity;
    remembered.previously_tracked = true;
    for (auto& observation : current_observations_) {
        if (observation.identity.stable_key() == key) observation.previously_tracked = true;
    }
}

void MonitorTracker::update_observations(const std::vector<MonitorScore>& scores, const TimePoint now) {
    current_observations_.clear();
    current_observations_.reserve(scores.size());
    std::unordered_set<std::string> current_keys;
    current_keys.reserve(scores.size());
    for (const auto& score : scores) {
        const auto key = score.monitor.stable_key();
        auto& observation = history_[key];
        observation.identity = score.monitor;
        observation.last_scanned_at = now;
        observation.capture_valid = valid_score(score);
        if (observation.capture_valid) {
            observation.last_similarity = score.similarity;
            if (score.similarity >= settings_.match_threshold) observation.last_reference_detected_at = now;
        }
        if (tracked_ && same_monitor(score.monitor, *tracked_)) observation.previously_tracked = true;
        if (current_keys.insert(key).second) current_observations_.push_back(observation);
        else {
            const auto current = std::find_if(current_observations_.begin(), current_observations_.end(), [&](const MonitorObservation& item) {
                return item.identity.stable_key() == key;
            });
            if (current != current_observations_.end()) *current = observation;
        }
    }

    while (history_.size() > maximum_history_entries) {
        const auto oldest = std::min_element(history_.begin(), history_.end(), [this](const auto& a, const auto& b) {
            const bool a_is_tracked = tracked_ && same_monitor(a.second.identity, *tracked_);
            const bool b_is_tracked = tracked_ && same_monitor(b.second.identity, *tracked_);
            if (a_is_tracked != b_is_tracked) return !a_is_tracked;
            return a.second.last_scanned_at < b.second.last_scanned_at;
        });
        if (oldest == history_.end() || (tracked_ && same_monitor(oldest->second.identity, *tracked_) && history_.size() == 1)) break;
        history_.erase(oldest);
    }
}

MonitorTrackingResult MonitorTracker::apply_scan(const std::vector<MonitorScore>& scores, const TimePoint now) {
    update_observations(scores, now);
    MonitorTrackingResult result;
    result.tracked = tracked_;
    const auto finish = [&]() {
        result.observations = current_observations_;
        return result;
    };
    std::vector<const MonitorScore*> valid;
    for (const auto& score : scores) {
        if (valid_score(score) && score.similarity >= settings_.match_threshold) valid.push_back(&score);
    }
    if (valid.empty()) {
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.scan_state = DetectionState::reference_missing;
        result.message = "reference not found; retaining last tracked monitor";
        return finish();
    }

    // More than one above-threshold score means the reference is duplicated,
    // even when one score is substantially higher. If the currently tracked
    // physical monitor still has a valid match, retaining it takes precedence
    // over score ordering so repeated scans cannot migrate or oscillate.
    if (tracked_) {
        const auto current = std::find_if(valid.begin(), valid.end(), [&](const MonitorScore* score) {
            return same_monitor(score->monitor, *tracked_);
        });
        if (current != valid.end()) {
            const bool duplicate = valid.size() > 1;
            tracked_ = (*current)->monitor; // Refresh transient geometry and resolution.
            mark_previously_tracked(*tracked_);
            pending_key_.clear();
            pending_confirmations_ = 0;
            result.tracked = tracked_;
            result.best_similarity = (*current)->similarity;
            result.scan_state = duplicate ? DetectionState::ambiguous : DetectionState::matching;
            result.message = duplicate ? "duplicate reference detected; retained tracked monitor" :
                                         "reference confirmed on tracked monitor";
            return finish();
        }
    }

    std::sort(valid.begin(), valid.end(), [](const MonitorScore* a, const MonitorScore* b) { return a->similarity > b->similarity; });
    const bool multiple_matches = valid.size() > 1;
    const double highest_score = valid.front()->similarity;
    const auto near_end = std::find_if(valid.begin(), valid.end(), [&](const MonitorScore* score) {
        return highest_score - score->similarity > settings_.reassignment_margin;
    });
    if (tracked_) {
        const auto affinity = std::max_element(valid.begin(), near_end, [this](const MonitorScore* a, const MonitorScore* b) {
            return identity_affinity(a->monitor) < identity_affinity(b->monitor);
        });
        if (affinity != near_end) std::iter_swap(valid.begin(), affinity);
    }
    const auto& best = *valid.front();
    result.best_similarity = best.similarity;

    const bool close_competition = std::distance(valid.begin(), near_end) > 1;
    if (close_competition) {
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.scan_state = DetectionState::ambiguous;
        result.message = "duplicate reference matches are ambiguous; tracked monitor unchanged";
        return finish();
    }
    result.scan_state = multiple_matches ? DetectionState::ambiguous : DetectionState::matching;

    double current_score = -1.0;
    if (tracked_) {
        for (const auto& score : scores) {
            if (same_monitor(score.monitor, *tracked_) && valid_score(score)) current_score = score.similarity;
        }
    }
    if (current_score >= 0.0 && best.similarity < current_score + settings_.reassignment_margin) {
        pending_key_.clear();
        pending_confirmations_ = 0;
        result.scan_state = DetectionState::ambiguous;
        result.message = "new match does not clearly exceed current monitor; tracked monitor unchanged";
        return finish();
    }

    const auto key = best.monitor.stable_key();
    if (key == pending_key_) ++pending_confirmations_;
    else {
        pending_key_ = key;
        pending_confirmations_ = 1;
    }
    if (pending_confirmations_ < settings_.confirmations_required) {
        result.confirmation_pending = true;
        result.message = multiple_matches ? "duplicate reference detected; preferred candidate awaiting confirmation" :
                                            "candidate monitor awaiting confirmation";
        return finish();
    }

    tracked_ = best.monitor;
    mark_previously_tracked(*tracked_);
    result.tracked = tracked_;
    result.changed = true;
    result.message = multiple_matches ? "tracked monitor reassigned after repeated valid scans; duplicate reference remains" :
                                        "tracked monitor reassigned after repeated valid scans";
    pending_key_.clear();
    pending_confirmations_ = 0;
    return finish();
}

} // namespace asc
