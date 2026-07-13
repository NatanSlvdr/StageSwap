#pragma once

#include "asc/core/types.hpp"

#include <chrono>
#include <optional>
#include <stdexcept>

namespace asc {

// Single-threaded debounce helper for event storms such as the cluster of
// power, session, display, and device notifications emitted after resume.
class DeferredTrigger {
public:
    void schedule(const TimePoint now, const std::chrono::milliseconds delay) {
        if (delay < std::chrono::milliseconds{0}) throw std::invalid_argument("deferred trigger delay must not be negative");
        due_at_ = now + delay;
    }

    [[nodiscard]] bool consume_if_due(const TimePoint now) noexcept {
        if (!due_at_ || now < *due_at_) return false;
        due_at_.reset();
        return true;
    }

    void cancel() noexcept { due_at_.reset(); }
    [[nodiscard]] bool pending() const noexcept { return due_at_.has_value(); }
    [[nodiscard]] std::optional<TimePoint> due_at() const noexcept { return due_at_; }

private:
    std::optional<TimePoint> due_at_;
};

} // namespace asc
