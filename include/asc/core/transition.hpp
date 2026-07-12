#pragma once

#include "asc/core/types.hpp"

#include <chrono>

namespace asc {

struct TransitionState {
    Source logical_source{Source::camera};
    Source target{Source::camera};
    double screen_mix{0.0};
    bool active{false};
    bool reversed{false};
    std::chrono::milliseconds remaining{0};
};

class TransitionController {
public:
    explicit TransitionController(std::chrono::milliseconds duration = std::chrono::milliseconds{500});
    void set_duration(std::chrono::milliseconds duration);
    [[nodiscard]] TransitionState request(Source target, TimePoint now);
    [[nodiscard]] TransitionState tick(TimePoint now);
    [[nodiscard]] const TransitionState& state() const noexcept { return state_; }

private:
    void advance(TimePoint now);
    std::chrono::milliseconds duration_;
    TimePoint last_update_{};
    bool initialized_{false};
    TransitionState state_;
};

} // namespace asc

