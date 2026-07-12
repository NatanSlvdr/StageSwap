#pragma once

#include "asc/core/config.hpp"
#include "asc/core/decision_engine.hpp"
#include "asc/core/event_log.hpp"
#include "asc/core/monitor_tracker.hpp"
#include "asc/core/transition.hpp"

#include <mutex>
#include <optional>
#include <string>
#include <vector>

namespace asc {

enum class RunState { stopped, starting, running, recovering, stopping, error };

struct AppStatus {
    RunState run_state{RunState::stopped};
    OutputMode mode{OutputMode::automatic};
    DetectionSnapshot detection;
    Source automatic_target{Source::camera};
    Source actual_output{Source::camera};
    TransitionState transition;
    SourceAvailability availability;
    DeviceState video_input{DeviceState::unavailable};
    DeviceState screen_capture{DeviceState::unavailable};
    DeviceState virtual_camera{DeviceState::unavailable};
    std::optional<MonitorIdentity> tracked_monitor;
    TimePoint last_full_scan{};
    std::string warning;
};

// Serializes state changes from UI, detector, display watcher, and video workers.
// It never owns frames; platform adapters execute the returned desired state.
class AppController {
public:
    AppController(AppConfig config, EventLog& log);
    void begin_start();
    void finish_start(bool video_ready, bool screen_ready, bool virtual_camera_ready);
    void stop();
    void set_mode(OutputMode mode, TimePoint now);
    void set_component_state(Source source, DeviceState state, TimePoint now);
    void set_virtual_camera_state(DeviceState state);
    void on_similarity(double similarity, bool capture_valid, TimePoint now);
    [[nodiscard]] MonitorTrackingResult on_monitor_scan(const std::vector<MonitorScore>& scores, TimePoint now);
    void tick(TimePoint now);
    void reconfigure(const AppConfig& config, TimePoint now);
    void set_tracked_monitor(MonitorIdentity monitor);
    [[nodiscard]] AppStatus status() const;
    [[nodiscard]] AppConfig config() const;

private:
    void evaluate(TimePoint now);
    void log_detection_change(DetectionState previous, const DetectionSnapshot& next);
    mutable std::mutex mutex_;
    AppConfig config_;
    EventLog& log_;
    DebouncedDetector detector_;
    MonitorTracker monitor_tracker_;
    DecisionEngine decision_engine_;
    TransitionController transition_;
    AppStatus status_;
    std::optional<DetectionState> scan_safety_state_;
};

} // namespace asc
