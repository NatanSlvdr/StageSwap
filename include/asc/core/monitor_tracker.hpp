#pragma once

#include "asc/core/types.hpp"

#include <cstddef>
#include <optional>
#include <string>
#include <vector>

namespace asc {

struct MonitorTrackerSettings {
    double match_threshold{0.98};
};

struct MonitorTrackingResult {
    std::optional<RuntimeMonitorDescriptor> tracked;
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
    [[nodiscard]] MonitorTrackingResult apply_scan(const std::vector<MonitorScore>& scores, TimePoint now);
    void select(RuntimeMonitorDescriptor monitor);
    [[nodiscard]] const std::optional<RuntimeMonitorDescriptor>& tracked() const noexcept { return tracked_; }

private:
    [[nodiscard]] static bool valid_score(const MonitorScore& score);

    MonitorTrackerSettings settings_;
    std::optional<RuntimeMonitorDescriptor> tracked_;
    std::string pending_key_;
    std::uint32_t pending_confirmations_{0};
};

} // namespace asc
