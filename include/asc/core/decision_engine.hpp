#pragma once

#include "asc/core/types.hpp"

namespace asc {

struct DecisionSettings {
    MissingReferenceBehavior missing_behavior{MissingReferenceBehavior::use_camera};
};

class DecisionEngine {
public:
    explicit DecisionEngine(DecisionSettings settings = {}) : settings_(settings) {}
    void configure(DecisionSettings settings) { settings_ = settings; }
    [[nodiscard]] Decision decide(OutputMode mode, DetectionState detection, Source current,
                                  const SourceAvailability& availability) const;

private:
    [[nodiscard]] static Source safe_available(Source requested, const SourceAvailability& availability);
    DecisionSettings settings_;
};

} // namespace asc
