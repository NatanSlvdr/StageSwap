#include "status_presentation.hpp"

#include <algorithm>
#include <chrono>
#include <iomanip>
#include <iterator>
#include <sstream>
#include <string_view>

namespace asc::win {
namespace {

const char* run_name(const RunState state) {
    switch (state) {
    case RunState::stopped: return "Stopped";
    case RunState::starting: return "Starting";
    case RunState::running: return "Running";
    case RunState::recovering: return "Recovering";
    case RunState::stopping: return "Stopping";
    case RunState::error: return "Error";
    }
    return "Unknown";
}

const char* mode_name(const OutputMode mode) {
    switch (mode) {
    case OutputMode::automatic: return "Automatic";
    case OutputMode::force_camera: return "Webcam / video override";
    case OutputMode::force_screen: return "Screen override";
    }
    return "Unknown";
}

const char* detection_name(const DetectionState state) {
    switch (state) {
    case DetectionState::unknown: return "Unknown";
    case DetectionState::matching: return "Detected";
    case DetectionState::not_matching: return "Not detected";
    case DetectionState::reference_missing: return "Missing";
    case DetectionState::ambiguous: return "Ambiguous";
    }
    return "Unknown";
}

const char* device_name(const DeviceState state) {
    switch (state) {
    case DeviceState::unavailable: return "Unavailable";
    case DeviceState::initializing: return "Initializing";
    case DeviceState::ready: return "Ready";
    case DeviceState::recovering: return "Recovering";
    case DeviceState::failed: return "Failed";
    }
    return "Unknown";
}

std::string monitor_name(const std::optional<MonitorIdentity>& monitor) {
    if (!monitor) return "Not identified";
    if (!monitor->model.empty()) return monitor->model;
    if (!monitor->manufacturer.empty()) return monitor->manufacturer + " display";
    if (!monitor->device_path.empty()) return monitor->device_path;
    return "Unidentified display";
}

std::string video_name(const VideoSourcePresentation& video) {
    if (video.identifier.empty()) return "No video input selected";
    if (!video.display_name.empty()) return video.display_name;
    return "Unavailable saved source";
}

std::string source_name(const Source source, const VideoSourcePresentation& video,
                        const std::optional<MonitorIdentity>& monitor) {
    switch (source) {
    case Source::camera: return video_name(video);
    case Source::screen: return monitor_name(monitor);
    case Source::placeholder: return "Safe placeholder";
    }
    return "Unknown";
}

const char* source_kind(const Source source) {
    switch (source) {
    case Source::camera: return "Webcam / video";
    case Source::screen: return "Screen";
    case Source::placeholder: return "Safe placeholder";
    }
    return "Unknown";
}

std::string age_text(const TimePoint measured_at, const TimePoint now, const bool milliseconds) {
    if (measured_at == TimePoint{}) return "Never";
    const auto elapsed = std::max(Clock::duration::zero(), now - measured_at);
    if (milliseconds) {
        return std::to_string(std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count()) + " ms ago";
    }
    return std::to_string(std::chrono::duration_cast<std::chrono::seconds>(elapsed).count()) + " s ago";
}

std::string percent(const double value, const int precision = 1) {
    std::ostringstream text;
    text << std::fixed << std::setprecision(precision) << value * 100.0 << '%';
    return text.str();
}

std::string health_summary(const AppStatus& status) {
    const DeviceState states[]{status.video_input, status.screen_capture, status.virtual_camera};
    if (std::any_of(std::begin(states), std::end(states), [](const DeviceState value) { return value == DeviceState::failed; }))
        return "Action required";
    if (std::any_of(std::begin(states), std::end(states), [](const DeviceState value) {
            return value == DeviceState::initializing || value == DeviceState::recovering;
        })) return "Components recovering";
    if (std::all_of(std::begin(states), std::end(states), [](const DeviceState value) { return value == DeviceState::ready; }))
        return "All components ready";
    return "Some components unavailable";
}

std::string short_event_line(const LogEvent& event) {
    const auto full = format_event_summary(event);
    const auto code_end = full.find("] ");
    if (code_end == std::string::npos) return event.message;
    const auto time_end = full.find("  ");
    const auto time = time_end == std::string::npos ? std::string{} : full.substr(0, time_end);
    return time.empty() ? event.message : time + "  " + event.message;
}

const MonitorObservation* tracked_observation(const AppStatus& status) {
    if (!status.tracked_monitor) return nullptr;
    const auto key = status.tracked_monitor->stable_key();
    const auto found = std::find_if(status.monitor_observations.begin(), status.monitor_observations.end(),
                                    [&key](const MonitorObservation& observation) {
                                        return observation.identity.stable_key() == key;
                                    });
    return found == status.monitor_observations.end() ? nullptr : &*found;
}

} // namespace

DashboardPresentation build_dashboard_presentation(const AppStatus& status, const AppConfig& config,
                                                   const VideoSourcePresentation& video,
                                                   const std::vector<LogEvent>& events,
                                                   const TimePoint now) {
    DashboardPresentation result;
    result.manual_override = status.mode != OutputMode::automatic;
    result.warning_active = !status.warning.empty();
    result.run_label = run_name(status.run_state);
    result.mode_label = mode_name(status.mode);
    result.output_kind = source_kind(status.actual_output);
    result.output_name = source_name(status.actual_output, video, status.tracked_monitor);
    result.reference_label = detection_name(status.detection.state);
    result.display_label = monitor_name(status.tracked_monitor);
    result.health_label = health_summary(status);
    result.warning = status.warning;

    {
        std::ostringstream text;
        text << "Current output: " << result.output_kind << " — " << result.output_name
             << "\r\nAutomatic target: " << source_name(status.automatic_target, video, status.tracked_monitor)
             << "\r\nTransition: " << (status.transition.active ? "In progress" : "Idle")
             << "; " << static_cast<int>(status.transition.screen_mix * 100.0) << "% screen; "
             << status.transition.remaining.count() << " ms remaining";
        result.output_tooltip = text.str();
    }
    {
        std::ostringstream text;
        text << "Similarity: " << percent(status.detection.similarity)
             << "\r\nThreshold: " << percent(config.detector.threshold)
             << "\r\nConfirmations: " << status.detection.consecutive_matches << " matching / "
             << status.detection.consecutive_mismatches << " mismatching"
             << "\r\nLast detection: " << age_text(status.detection.measured_at, now, true);
        result.reference_tooltip = text.str();
    }
    {
        std::ostringstream text;
        text << "Display: " << result.display_label;
        if (status.tracked_monitor) {
            text << "\r\nResolution: " << status.tracked_monitor->resolution.width << " x "
                 << status.tracked_monitor->resolution.height
                 << "\r\nDesktop position: (" << status.tracked_monitor->desktop_x << ", "
                 << status.tracked_monitor->desktop_y << ')';
            if (const auto* observation = tracked_observation(status)) {
                text << "\r\nLatest capture: " << (observation->capture_valid ? "Available" : "Unavailable")
                     << "; similarity " << percent(observation->last_similarity)
                     << "\r\nLast scan: " << age_text(observation->last_scanned_at, now, false);
            }
        }
        text << "\r\nLast full scan: " << age_text(status.last_full_scan, now, false);
        result.display_tooltip = text.str();
    }
    {
        std::ostringstream text;
        text << "Video input: " << device_name(status.video_input)
             << "\r\nScreen capture: " << device_name(status.screen_capture)
             << "\r\nVirtual camera: " << device_name(status.virtual_camera);
        result.health_tooltip = text.str();
    }

    {
        std::ostringstream text;
        text << "Status: " << result.run_label << "\r\n"
             << "Mode: " << result.mode_label << "\r\n"
             << "Reference: " << result.reference_label << "    Similarity: " << percent(status.detection.similarity)
             << "    Threshold: " << percent(config.detector.threshold) << "\r\n"
             << "Confirmations: " << status.detection.consecutive_matches << " matching / "
             << status.detection.consecutive_mismatches << " mismatching\r\n"
             << "Tracked display: " << result.display_label;
        if (status.tracked_monitor)
            text << " — " << status.tracked_monitor->resolution.width << " x "
                 << status.tracked_monitor->resolution.height;
        text << "\r\nSelected video source: " << video_name(video)
             << "\r\nAutomatic target: " << source_name(status.automatic_target, video, status.tracked_monitor)
             << "    Actual output: " << result.output_name << "\r\n"
             << "Transition: " << (status.transition.active ? "In progress" : "Idle") << "    "
             << static_cast<int>(status.transition.screen_mix * 100.0) << "% screen    "
             << status.transition.remaining.count() << " ms remaining\r\n"
             << result.health_tooltip << "\r\n"
             << "Last detection: " << age_text(status.detection.measured_at, now, true)
             << "    Last full scan: " << age_text(status.last_full_scan, now, false);
        if (!status.warning.empty()) text << "\r\nWarning: " << status.warning;
        result.technical_details = text.str();
    }

    result.recent_activity.reserve(std::min<std::size_t>(3, events.size()));
    result.full_activity.reserve(events.size());
    for (std::size_t index = 0; index < events.size(); ++index) {
        if (index < 3) result.recent_activity.push_back(short_event_line(events[index]));
        result.full_activity.push_back(format_event_summary(events[index]));
    }
    return result;
}

} // namespace asc::win
