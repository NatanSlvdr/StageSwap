#pragma once

#include "asc/core/types.hpp"

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
    DetectionState scan_state{DetectionState::reference_missing};
    double best_similarity{0.0};
    bool changed{false};
    std::string message;
};

class MonitorTracker {
public:
    explicit MonitorTracker(MonitorTrackerSettings settings = {});
    void configure(MonitorTrackerSettings settings);
    void restore_preferred(MonitorIdentity identity);
    [[nodiscard]] MonitorTrackingResult apply_scan(const std::vector<MonitorScore>& scores);
    [[nodiscard]] const std::optional<MonitorIdentity>& tracked() const noexcept { return tracked_; }

private:
    [[nodiscard]] int identity_affinity(const MonitorIdentity& candidate) const;
    [[nodiscard]] static bool same_monitor(const MonitorIdentity& a, const MonitorIdentity& b);

    MonitorTrackerSettings settings_;
    std::optional<MonitorIdentity> tracked_;
    std::string pending_key_;
    std::uint32_t pending_confirmations_{0};
};

} // namespace asc
