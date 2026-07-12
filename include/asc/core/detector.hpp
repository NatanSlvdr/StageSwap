#pragma once

#include "asc/core/types.hpp"

#include <cstdint>

namespace asc {

struct DetectorSettings {
    double threshold{0.98};
    std::uint32_t matches_required{5};
    std::uint32_t mismatches_required{3};
};

class DebouncedDetector {
public:
    explicit DebouncedDetector(DetectorSettings settings = {});
    void configure(DetectorSettings settings);
    [[nodiscard]] DetectionSnapshot update(double similarity, bool capture_valid, TimePoint now);
    [[nodiscard]] const DetectionSnapshot& snapshot() const noexcept { return snapshot_; }
    void reset(DetectionState state = DetectionState::unknown);

private:
    DetectorSettings settings_;
    DetectionSnapshot snapshot_;
};

} // namespace asc
