#include "status_presentation.hpp"

#include <chrono>
#include <iostream>
#include <string_view>

namespace {

int failures = 0;

void check(const bool condition, const std::string_view message) {
    if (!condition) {
        ++failures;
        std::cerr << "FAIL: " << message << '\n';
    }
}

bool contains(const std::string& text, const std::string_view expected) {
    return text.find(expected) != std::string::npos;
}

asc::MonitorIdentity example_monitor() {
    asc::MonitorIdentity monitor;
    monitor.hardware_id = "MONITOR-1";
    monitor.model = "Studio Display";
    monitor.resolution = {3840, 2160};
    monitor.desktop_x = 1920;
    return monitor;
}

asc::AppStatus ready_status(const asc::TimePoint now) {
    asc::AppStatus status;
    status.run_state = asc::RunState::running;
    status.mode = asc::OutputMode::automatic;
    status.detection = {asc::DetectionState::matching, 0.942, 5, 0, now - std::chrono::milliseconds{84}};
    status.automatic_target = asc::Source::camera;
    status.actual_output = asc::Source::camera;
    status.video_input = asc::DeviceState::ready;
    status.screen_capture = asc::DeviceState::ready;
    status.virtual_camera = asc::DeviceState::ready;
    status.tracked_monitor = example_monitor();
    status.last_full_scan = now - std::chrono::seconds{12};
    status.monitor_observations.push_back({*status.tracked_monitor, 0.942, true, now - std::chrono::seconds{12}, now - std::chrono::seconds{4}, true});
    return status;
}

void automatic_presentation_test() {
    const auto now = asc::Clock::now();
    const auto status = ready_status(now);
    asc::AppConfig config;
    config.detector.threshold = 0.9;
    const asc::win::VideoSourcePresentation video{"camera-id", "Logitech Brio"};
    const std::vector<asc::LogEvent> events{
        {std::chrono::system_clock::now(), asc::LogLevel::info, "detector", "REFERENCE_DETECTED", "Reference detected", "{}"}
    };
    const auto view = asc::win::build_dashboard_presentation(status, config, video, events, now);
    check(view.run_label == "Running", "running state is visible");
    check(view.output_kind == "Webcam / video" && view.output_name == "Logitech Brio", "concrete output is prominent");
    check(view.reference_label == "Detected" && view.display_label == "Studio Display", "reference and display summaries are friendly");
    check(view.health_label == "All components ready", "healthy components collapse to one summary");
    check(contains(view.reference_tooltip, "Similarity: 94.2%") && contains(view.reference_tooltip, "Threshold: 90.0%"),
          "technical detection metrics move to the reference tooltip");
    check(contains(view.display_tooltip, "3840") && contains(view.display_tooltip, "Desktop position: (1920, 0)"),
          "display tooltip retains geometry");
    check(contains(view.technical_details, "Confirmations: 5 matching / 0 mismatching"),
          "expanded details retain confirmation counters");
    check(view.recent_activity.size() == 1 && contains(view.recent_activity.front(), "Reference detected"),
          "recent activity is human readable");
}

void override_and_transition_test() {
    const auto now = asc::Clock::now();
    auto status = ready_status(now);
    status.mode = asc::OutputMode::force_screen;
    status.automatic_target = asc::Source::camera;
    status.actual_output = asc::Source::screen;
    status.transition = {asc::Source::camera, asc::Source::screen, 0.64, true, false, std::chrono::milliseconds{180}};
    const auto view = asc::win::build_dashboard_presentation(status, asc::AppConfig{}, {"camera-id", "Camera"}, {}, now);
    check(view.manual_override, "manual override remains an explicit state");
    check(view.output_kind == "Screen" && view.output_name == "Studio Display", "screen output names the physical display");
    check(contains(view.output_tooltip, "In progress") && contains(view.output_tooltip, "64% screen") &&
          contains(view.output_tooltip, "180 ms remaining"), "transition telemetry remains available");
}

void warning_during_override_test() {
    const auto now = asc::Clock::now();
    auto status = ready_status(now);
    status.mode = asc::OutputMode::force_screen;
    status.warning = "screen capture is recovering";
    const auto view = asc::win::build_dashboard_presentation(
        status, asc::AppConfig{}, {"camera-id", "Camera"}, {}, now);
    const auto banners = asc::win::dashboard_banner_visibility(view);
    check(banners.show_warning && banners.show_override && banners.row_count == 2,
          "warning and manual override use two simultaneous dashboard rows");
}

void failure_and_placeholder_test() {
    const auto now = asc::Clock::now();
    auto status = ready_status(now);
    status.run_state = asc::RunState::error;
    status.actual_output = asc::Source::placeholder;
    status.video_input = asc::DeviceState::failed;
    status.screen_capture = asc::DeviceState::recovering;
    status.virtual_camera = asc::DeviceState::ready;
    status.detection.state = asc::DetectionState::reference_missing;
    status.warning = "camera is unavailable; safe placeholder is active";
    const auto view = asc::win::build_dashboard_presentation(status, asc::AppConfig{}, {"saved-camera", ""}, {}, now);
    check(view.warning_active && view.warning == status.warning, "critical warning remains visible");
    check(view.output_kind == "Safe placeholder" && view.output_name == "Safe placeholder", "privacy fallback is explicit");
    check(view.health_label == "Action required", "failed component cannot be hidden by the health summary");
    check(contains(view.health_tooltip, "Video input: Failed") && contains(view.health_tooltip, "Screen capture: Recovering"),
          "component states remain in health details");
    check(contains(view.technical_details, "Warning:"), "warning is duplicated in selectable diagnostics");
}

void unavailable_source_and_activity_limit_test() {
    const auto now = asc::Clock::now();
    auto status = ready_status(now);
    status.actual_output = asc::Source::camera;
    std::vector<asc::LogEvent> events;
    for (int index = 0; index < 6; ++index) {
        events.push_back({std::chrono::system_clock::now(), asc::LogLevel::info, "test", "EVENT",
                          "Event " + std::to_string(index), "{\"index\":" + std::to_string(index) + "}"});
    }
    const auto view = asc::win::build_dashboard_presentation(status, asc::AppConfig{}, {"saved-camera", ""}, events, now);
    check(view.output_name == "Unavailable saved source", "saved but disconnected source remains understandable");
    check(view.recent_activity.size() == 3, "dashboard limits activity to three rows");
    check(view.full_activity.size() == events.size() && contains(view.full_activity.front(), "{\"index\":0}"),
          "expanded activity retains every event and structured context");
}

void preview_size_and_reconnect_text_test() {
    check(asc::win::fit_preview_size({1920, 1080}, {400, 400}) == asc::Size{400, 225},
          "preview fitting preserves a landscape aspect ratio");
    check(asc::win::fit_preview_size({1200, 1800}, {400, 300}) == asc::Size{200, 300},
          "preview fitting preserves a portrait aspect ratio");
    check(asc::win::fit_preview_size({3840, 2160}, {4000, 3000}) == asc::Size{640, 360},
          "preview fitting caps large render targets");
    check(asc::win::fit_preview_size({1920, 1080}, {0, 300}) == asc::Size{},
          "preview fitting rejects empty bounds");
    check(contains(asc::win::unavailable_video_source_status(true), "retried automatically"),
          "enabled reconnect status describes automatic retries");
    check(contains(asc::win::unavailable_video_source_status(false), "reconnect is disabled"),
          "disabled reconnect status does not promise retries");
}

} // namespace

int main() {
    automatic_presentation_test();
    override_and_transition_test();
    warning_during_override_test();
    failure_and_placeholder_test();
    unavailable_source_and_activity_limit_test();
    preview_size_and_reconnect_text_test();
    if (failures == 0) std::cout << "All UI presentation tests passed\n";
    return failures == 0 ? 0 : 1;
}
