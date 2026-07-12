#include "asc/core/controller.hpp"

#include <iomanip>
#include <sstream>

namespace asc {
namespace {
const char* source_name(const Source source) {
    switch (source) { case Source::camera: return "camera"; case Source::screen: return "screen"; case Source::placeholder: return "placeholder"; }
    return "unknown";
}
const char* mode_name(const OutputMode mode) {
    switch (mode) { case OutputMode::automatic: return "automatic"; case OutputMode::force_camera: return "force camera"; case OutputMode::force_screen: return "force screen"; }
    return "unknown";
}
}

AppController::AppController(AppConfig config, EventLog& log)
    : config_(std::move(config)), log_(log), detector_(config_.detector), monitor_tracker_(config_.monitor_tracker),
      decision_engine_({config_.missing_behavior}), transition_(config_.fade_duration) {
    status_.mode = config_.output_mode;
    if (config_.last_tracked_monitor) {
        monitor_tracker_.restore_preferred(*config_.last_tracked_monitor);
        status_.tracked_monitor = config_.last_tracked_monitor;
    }
}

void AppController::begin_start() {
    std::scoped_lock lock(mutex_);
    if (status_.run_state != RunState::stopped && status_.run_state != RunState::error) return;
    status_.run_state = RunState::starting;
    status_.warning.clear();
    status_.video_input = DeviceState::initializing;
    status_.screen_capture = DeviceState::initializing;
    status_.virtual_camera = DeviceState::initializing;
    log_.write(LogLevel::info, "lifecycle", "APPLICATION_STARTING", "Application components are starting");
}

void AppController::finish_start(const bool video_ready, const bool screen_ready, const bool virtual_camera_ready) {
    std::scoped_lock lock(mutex_);
    status_.video_input = video_ready ? DeviceState::ready : DeviceState::recovering;
    status_.screen_capture = screen_ready ? DeviceState::ready : DeviceState::recovering;
    status_.virtual_camera = virtual_camera_ready ? DeviceState::ready : DeviceState::failed;
    status_.availability = {video_ready, screen_ready, true};
    status_.run_state = virtual_camera_ready ? RunState::running : RunState::error;
    if (!virtual_camera_ready) status_.warning = "Virtual camera could not be initialized";
    else if (!video_ready) status_.warning = "camera is unavailable; safe placeholder is active";
    else if (!screen_ready) status_.warning = "screen capture is unavailable; safe fallback is active";
    log_.write(virtual_camera_ready ? LogLevel::info : LogLevel::error, "lifecycle",
               virtual_camera_ready ? "APPLICATION_STARTED" : "APPLICATION_START_FAILED",
               virtual_camera_ready ? "Application started" : status_.warning);
    evaluate(Clock::now());
}

void AppController::stop() {
    std::scoped_lock lock(mutex_);
    status_.run_state = RunState::stopping;
    status_.transition = transition_.request(
        status_.availability.camera_ready ? Source::camera : Source::placeholder, Clock::now());
    detector_.reset();
    status_.detection = detector_.snapshot();
    status_.run_state = RunState::stopped;
    log_.write(LogLevel::info, "lifecycle", "APPLICATION_STOPPED", "Automation stopped; safe output retained");
}

void AppController::set_mode(const OutputMode mode, const TimePoint now) {
    std::scoped_lock lock(mutex_);
    const auto previous = status_.mode;
    status_.mode = mode;
    config_.output_mode = mode;
    log_.write(LogLevel::info, "decision", mode == OutputMode::automatic && previous != OutputMode::automatic ? "MANUAL_OVERRIDE_DISABLED" :
               mode == OutputMode::automatic ? "AUTOMATIC_MODE_RESTORED" : "MANUAL_OVERRIDE_ENABLED",
               std::string("Output mode changed to ") + mode_name(mode),
               std::string("{\"previous\":\"") + mode_name(previous) + "\"}");
    evaluate(now);
}

void AppController::set_component_state(const Source source, const DeviceState state, const TimePoint now) {
    std::scoped_lock lock(mutex_);
    if (source == Source::camera) {
        status_.video_input = state;
        status_.availability.camera_ready = state == DeviceState::ready;
    } else if (source == Source::screen) {
        status_.screen_capture = state;
        status_.availability.screen_ready = state == DeviceState::ready;
    }
    log_.write(state == DeviceState::failed ? LogLevel::error : LogLevel::info, "recovery", "COMPONENT_STATE_CHANGED",
               std::string(source_name(source)) + " state changed");
    const std::string component = source_name(source);
    if (state == DeviceState::failed || state == DeviceState::recovering)
        status_.warning = component + (state == DeviceState::failed ? " failed; safe fallback is active" : " is recovering; safe fallback is active");
    else if (state == DeviceState::ready && status_.warning.starts_with(component)) status_.warning.clear();
    evaluate(now);
}

void AppController::set_virtual_camera_state(const DeviceState state) {
    std::scoped_lock lock(mutex_);
    status_.virtual_camera = state;
    if (state == DeviceState::failed) {
        status_.warning = "Virtual camera failed; recovery required";
        status_.run_state = RunState::error;
    } else if (state == DeviceState::ready) {
        if (status_.warning.starts_with("Virtual camera")) status_.warning.clear();
        if (status_.run_state == RunState::error) status_.run_state = RunState::running;
    }
}

void AppController::log_detection_change(const DetectionState previous, const DetectionSnapshot& next) {
    if (previous == next.state) return;
    std::ostringstream details;
    details << "{\"similarity\":" << std::fixed << std::setprecision(5) << next.similarity << '}';
    if (next.state == DetectionState::matching || next.state == DetectionState::not_matching)
        log_.write(LogLevel::info, "detector", "SIMILARITY_THRESHOLD_CROSSED",
                   next.state == DetectionState::matching ? "Similarity crossed into matching state" : "Similarity crossed into mismatching state",
                   details.str());
    if (next.state == DetectionState::matching)
        log_.write(LogLevel::info, "detector", "REFERENCE_DETECTED", "Reference image detected", details.str());
    else if (next.state == DetectionState::not_matching)
        log_.write(LogLevel::info, "detector", "REFERENCE_LOST", "Reference image no longer detected", details.str());
    else if (next.state == DetectionState::reference_missing)
        log_.write(LogLevel::warning, "detector", "REFERENCE_NOT_FOUND", "Reference image or tracked capture unavailable", details.str());
}

void AppController::on_similarity(const double similarity, const bool capture_valid, const TimePoint now) {
    std::scoped_lock lock(mutex_);
    const auto previous = status_.detection.state;
    status_.detection = detector_.update(similarity, capture_valid, now);
    if (scan_safety_state_) {
        if (status_.detection.state == DetectionState::matching) scan_safety_state_.reset();
        else status_.detection.state = *scan_safety_state_;
    }
    log_detection_change(previous, status_.detection);
    evaluate(now);
}

MonitorTrackingResult AppController::on_monitor_scan(const std::vector<MonitorScore>& scores, const TimePoint now) {
    std::scoped_lock lock(mutex_);
    const auto previous_detection = status_.detection.state;
    auto result = monitor_tracker_.apply_scan(scores);
    status_.tracked_monitor = result.tracked;
    status_.last_full_scan = now;
    if (result.scan_state == DetectionState::reference_missing || result.scan_state == DetectionState::ambiguous) {
        scan_safety_state_ = result.scan_state;
        status_.detection.state = result.scan_state;
    } else {
        scan_safety_state_.reset();
        if (result.scan_state == DetectionState::matching) {
            status_.detection.state = DetectionState::matching;
            status_.detection.similarity = result.best_similarity;
            status_.detection.measured_at = now;
        }
    }
    log_detection_change(previous_detection, status_.detection);
    if (result.changed) {
        log_.write(LogLevel::info, "monitors", "TRACKED_MONITOR_CHANGED", result.message,
                   std::string("{\"similarity\":") + std::to_string(result.best_similarity) + "}");
    } else if (result.scan_state == DetectionState::ambiguous) {
        status_.warning = result.message;
        log_.write(LogLevel::warning, "monitors", "DUPLICATE_REFERENCE", result.message);
    } else if (result.scan_state == DetectionState::reference_missing) {
        status_.warning = result.message;
        log_.write(LogLevel::warning, "monitors", "REFERENCE_NOT_FOUND", result.message);
    } else if (status_.warning.starts_with("reference") || status_.warning.starts_with("duplicate") ||
               status_.warning.starts_with("new match") || status_.warning.starts_with("candidate monitor")) {
        status_.warning.clear();
    }
    evaluate(now);
    return result;
}

void AppController::evaluate(const TimePoint now) {
    const auto decision = decision_engine_.decide(status_.mode, status_.detection.state, status_.actual_output, status_.availability);
    status_.automatic_target = decision.automatic_target;
    if (status_.run_state == RunState::stopped || status_.run_state == RunState::stopping) {
        status_.transition = transition_.request(status_.availability.camera_ready ? Source::camera : Source::placeholder, now);
        if (!status_.transition.active) status_.actual_output = status_.transition.logical_source;
        return;
    }
    const auto before = transition_.state();
    status_.transition = transition_.request(decision.desired_output, now);
    if (status_.transition.active && (!before.active || before.target != status_.transition.target)) {
        log_.write(LogLevel::info, "compositor", status_.transition.reversed ? "FADE_REVERSED" : "FADE_STARTED",
                   std::string(status_.transition.reversed ? "Fade reversed toward " : "Fading to ") + source_name(decision.desired_output),
                   std::string("{\"duration_ms\":") + std::to_string(config_.fade_duration.count()) + "}");
    }
    if (!status_.transition.active) status_.actual_output = status_.transition.logical_source;
}

void AppController::tick(const TimePoint now) {
    std::scoped_lock lock(mutex_);
    const bool was_active = status_.transition.active;
    status_.transition = transition_.tick(now);
    if (!status_.transition.active) {
        status_.actual_output = status_.transition.logical_source;
        if (was_active) log_.write(LogLevel::info, "compositor", "FADE_COMPLETED",
                                   std::string("Fade completed; ") + source_name(status_.actual_output) + " active");
    }
}

void AppController::reconfigure(const AppConfig& config, const TimePoint now) {
    std::scoped_lock lock(mutex_);
    config_ = config;
    detector_.configure(config_.detector);
    monitor_tracker_.configure(config_.monitor_tracker);
    decision_engine_.configure({config_.missing_behavior});
    transition_.set_duration(config_.fade_duration);
    status_.mode = config_.output_mode;
    evaluate(now);
    log_.write(LogLevel::info, "configuration", "CONFIGURATION_APPLIED", "Configuration changes applied");
}

void AppController::set_tracked_monitor(MonitorIdentity monitor) {
    std::scoped_lock lock(mutex_);
    monitor_tracker_.restore_preferred(monitor);
    status_.tracked_monitor = std::move(monitor);
    log_.write(LogLevel::info, "monitors", "TRACKED_MONITOR_SELECTED", "Tracked monitor selected by user");
}

AppStatus AppController::status() const { std::scoped_lock lock(mutex_); return status_; }
AppConfig AppController::config() const { std::scoped_lock lock(mutex_); return config_; }

} // namespace asc
