#include "asc/core/transition.hpp"

#include <algorithm>
#include <cmath>
#include <stdexcept>

namespace asc {

TransitionController::TransitionController(const std::chrono::milliseconds duration) : duration_(duration) {
    set_duration(duration);
}

void TransitionController::set_duration(const std::chrono::milliseconds duration) {
    if (duration.count() < 0 || duration.count() > 2000) throw std::invalid_argument("fade duration outside 0..2000 ms");
    duration_ = duration;
}

void TransitionController::advance(const TimePoint now) {
    if (!initialized_) {
        initialized_ = true;
        last_update_ = now;
        return;
    }
    const auto elapsed = std::chrono::duration<double>(now - last_update_).count();
    last_update_ = now;
    if (!state_.active) return;
    const double duration_seconds = std::chrono::duration<double>(duration_).count();
    const double step = duration_seconds <= 0.0 ? 1.0 : elapsed / duration_seconds;
    const double goal = state_.target == Source::screen ? 1.0 : 0.0;
    if (goal > state_.screen_mix) state_.screen_mix = std::min(goal, state_.screen_mix + step);
    else state_.screen_mix = std::max(goal, state_.screen_mix - step);
    if (std::abs(state_.screen_mix - goal) < 1e-9) {
        state_.screen_mix = goal;
        state_.active = false;
        state_.logical_source = state_.target;
        state_.remaining = std::chrono::milliseconds{0};
    } else {
        const auto distance = std::abs(goal - state_.screen_mix);
        state_.remaining = std::chrono::milliseconds{static_cast<long long>(std::ceil(distance * static_cast<double>(duration_.count())))};
    }
}

TransitionState TransitionController::request(const Source target, const TimePoint now) {
    advance(now);
    if (target == Source::placeholder) {
        state_ = {Source::placeholder, Source::placeholder, 0.0, false, false, std::chrono::milliseconds{0}};
        return state_;
    }
    const bool was_active = state_.active;
    const auto old_target = state_.target;
    state_.target = target;
    const double goal = target == Source::screen ? 1.0 : 0.0;
    state_.reversed = was_active && old_target != target;
    state_.active = std::abs(state_.screen_mix - goal) > 1e-9;
    if (!state_.active) state_.logical_source = target;
    state_.remaining = std::chrono::milliseconds{
        static_cast<long long>(std::ceil(std::abs(goal - state_.screen_mix) * static_cast<double>(duration_.count())))};
    return state_;
}

TransitionState TransitionController::tick(const TimePoint now) {
    state_.reversed = false;
    advance(now);
    return state_;
}

} // namespace asc
