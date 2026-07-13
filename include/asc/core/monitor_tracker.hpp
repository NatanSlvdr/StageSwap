#pragma once

#include "asc/core/types.hpp"

#include <cstddef>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

namespace asc {

struct MonitorTrackerSettings {
    double match_threshold{0.98};
    double reassignment_margin{0.01};
    std::uint32_t confirmations_required{3};
};

struct MonitorTrackingResult {
    std::optional<MonitorIdentity> tracked;
    std::vector<MonitorObservation> observations;
    DetectionState scan_state{DetectionState::reference_missing};
    double best_similarity{0.0};
    bool changed{false};
    bool confirmation_pending{false};
    std::string message;
};

class MonitorTracker {
public:
    explicit MonitorTracker(MonitorTrackerSettings settings = {});
    void configure(MonitorTrackerSettings settings);
    void restore_preferred(MonitorIdentity identity);
    [[nodiscard]] MonitorTrackingResult apply_scan(const std::vector<MonitorScore>& scores, TimePoint now);
    [[nodiscard]] const std::optional<MonitorIdentity>& tracked() const noexcept { return tracked_; }
    [[nodiscard]] const std::vector<MonitorObservation>& observations() const noexcept { return current_observations_; }
    [[nodiscard]] std::size_t remembered_monitor_count() const noexcept { return history_.size(); }

private:
    [[nodiscard]] int identity_affinity(const MonitorIdentity& candidate) const;
    [[nodiscard]] static bool same_monitor(const MonitorIdentity& a, const MonitorIdentity& b);
    [[nodiscard]] static bool valid_score(const MonitorScore& score);
    void update_observations(const std::vector<MonitorScore>& scores, TimePoint now);
    void mark_previously_tracked(const MonitorIdentity& identity);

    MonitorTrackerSettings settings_;
    std::optional<MonitorIdentity> tracked_;
    std::unordered_map<std::string, MonitorObservation> history_;
    std::vector<MonitorObservation> current_observations_;
    std::string pending_key_;
    std::uint32_t pending_confirmations_{0};
};

} // namespace asc
