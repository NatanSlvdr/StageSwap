#pragma once

#include "asc/core/types.hpp"

namespace asc {

class DecisionEngine {
public:
    [[nodiscard]] Decision decide(OutputMode mode, DetectionState detection, Source current,
                                  const SourceAvailability& availability) const;

private:
    [[nodiscard]] static Source safe_available(Source requested, const SourceAvailability& availability);
};

} // namespace asc
