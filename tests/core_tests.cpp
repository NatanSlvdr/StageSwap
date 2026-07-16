#include "asc/core/config.hpp"
#include "asc/core/decision_engine.hpp"
#include "asc/core/detector.hpp"
#include "asc/core/frame.hpp"
#include "asc/core/monitor_tracker.hpp"
#include "asc/core/transition.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <string_view>

namespace {

int failures = 0;
void check(const bool condition, const std::string_view message) {
    if (!condition) { ++failures; std::cerr << "FAIL: " << message << '\n'; }
}

asc::Frame solid(const asc::Size size, const std::uint32_t color, const asc::TimePoint received = asc::Clock::now()) {
    auto frame = asc::make_placeholder(size, color);
    frame.received_at = received;
    frame.freshness = asc::FrameFreshness::live;
    return frame;
}

asc::RuntimeMonitorDescriptor monitor(std::string name, const std::int32_t x = 0) {
    return {std::move(name), "Display", {x, 0, 1920, 1080}, static_cast<std::uintptr_t>(x + 1)};
}

void frame_test() {
    const auto now = asc::Clock::now();
    auto source = solid({4, 2}, 0xff204080u, now);
    const auto fitted = asc::aspect_fit_bgra(source, {4, 4});
    check(fitted.valid() && fitted.size == asc::Size{4, 4}, "aspect-fit produces the requested BGRA frame");
    bool black_letterbox = true;
    for (std::size_t offset = 0; offset < 16; offset += 4)
        black_letterbox = black_letterbox && fitted.bgra[offset] == 0 && fitted.bgra[offset + 1] == 0 &&
                          fitted.bgra[offset + 2] == 0 && fitted.bgra[offset + 3] == 0xff;
    check(black_letterbox, "aspect-fit uses black top letterboxing");
    check(fitted.bgra[16] == 0x80 && fitted.bgra[17] == 0x40 && fitted.bgra[18] == 0x20,
          "aspect-fit preserves source pixels in the fitted rectangle");

    const auto camera = solid({2, 2}, 0xff000000u, now);
    const auto screen = solid({2, 2}, 0xffffffffu, now);
    const auto blended = asc::blend_bgra(camera, screen, 0.5, 0xff112233u, {2, 2});
    check(blended.bgra[0] == 128 && blended.bgra[1] == 128 && blended.bgra[2] == 128,
          "CPU compositor blends camera and live screen frames");
    const auto placeholder = asc::blend_bgra({}, {}, 0.0, 0xff112233u, {2, 2});
    check(placeholder.bgra[0] == 0x33 && placeholder.bgra[1] == 0x22 && placeholder.bgra[2] == 0x11,
          "missing camera produces the configured placeholder");
    check(source.fresh(now + std::chrono::milliseconds{999}) && !source.fresh(now + std::chrono::milliseconds{1001}),
          "stale-frame rejection uses the fixed freshness window");
}

void detector_test() {
    asc::DebouncedDetector detector({0.98, 5, 3});
    const auto now = asc::Clock::now();
    for (int index = 0; index < 4; ++index) [[maybe_unused]] const auto snapshot = detector.update(0.99, true, now);
    check(detector.snapshot().state == asc::DetectionState::unknown, "four matches do not confirm detection");
    check(detector.update(0.99, true, now).state == asc::DetectionState::matching, "five matches confirm detection");
    [[maybe_unused]] const auto mismatch_one = detector.update(0.1, true, now);
    [[maybe_unused]] const auto mismatch_two = detector.update(0.1, true, now);
    check(detector.snapshot().state == asc::DetectionState::matching, "two mismatches retain the match");
    check(detector.update(0.1, true, now).state == asc::DetectionState::not_matching, "three mismatches confirm loss");
    check(detector.update(0.0, false, now).state == asc::DetectionState::reference_missing,
          "invalid capture selects the missing-reference safety state");
}

void transition_test() {
    using namespace std::chrono_literals;
    const auto start = asc::Clock::now();
    asc::TransitionController transition(500ms);
    [[maybe_unused]] const auto initial = transition.tick(start);
    [[maybe_unused]] const auto requested = transition.request(asc::Source::screen, start);
    check(std::abs(transition.tick(start + 300ms).screen_mix - 0.6) < 0.001, "fixed fade reaches sixty percent");
    const auto reversed = transition.request(asc::Source::camera, start + 300ms);
    check(reversed.reversed && std::abs(reversed.screen_mix - 0.6) < 0.001, "fade reverses without a discontinuity");
    check(std::abs(transition.tick(start + 450ms).screen_mix - 0.3) < 0.001, "reversed fade follows the same duration");
}

void monitor_test() {
    asc::MonitorTracker tracker({0.98});
    tracker.select(monitor("DISPLAY1"));
    const auto now = asc::Clock::now();
    const std::vector<asc::MonitorScore> scores{{monitor("DISPLAY1"), 0.2, true}, {monitor("DISPLAY2", 1920), 0.995, true}};
    const auto first = tracker.apply_scan(scores, now);
    check(first.confirmation_pending && first.tracked->gdi_display_name == "DISPLAY1", "first scan retains current monitor");
    const auto second = tracker.apply_scan(scores, now);
    check(second.changed && second.tracked->gdi_display_name == "DISPLAY2", "immediate second scan confirms best monitor");
    const auto missing = tracker.apply_scan({{monitor("DISPLAY1"), 0.2, true}}, now);
    check(missing.scan_state == asc::DetectionState::reference_missing &&
          missing.tracked->gdi_display_name == "DISPLAY2", "missing reference retains runtime selection");
}

void config_test() {
    asc::AppConfig original;
    original.selected_video_device_id = "camera\\id\"one";
    original.similarity_threshold = 0.975;
    original.output_mode = asc::OutputMode::force_screen;
    const auto json = asc::ConfigStore::serialize(original);
    std::string error;
    const auto parsed = asc::ConfigStore::parse(json, error);
    check(parsed && parsed->schema_version == 2 && parsed->selected_video_device_id == original.selected_video_device_id,
          "schema v2 round-trips retained values");
    check(json.find("output_width") == std::string::npos && json.find("reassignment_margin") == std::string::npos &&
          json.find("video_auto_reconnect") == std::string::npos && json.find("monitor_hardware_id") == std::string::npos,
          "schema v2 does not serialize removed settings");

    constexpr std::string_view v1 = R"({"schema_version":1,"selected_video_device_id":"legacy-camera","detection_threshold":0.91,"cursor_visible":true,"output_mode":"force_camera","output_width":1920,"fade_duration_ms":900,"monitor_hardware_id":"EDID"})";
    const auto migrated = asc::ConfigStore::parse(v1, error);
    check(migrated && migrated->schema_version == 2 && migrated->selected_video_device_id == "legacy-camera" &&
          std::abs(migrated->similarity_threshold - 0.91) < 0.0001 && migrated->cursor_visible &&
          migrated->output_mode == asc::OutputMode::force_camera,
          "v1 migration imports retained values and ignores removed fields");
    const auto migrated_json = asc::ConfigStore::serialize(*migrated);
    check(migrated_json.find("output_width") == std::string::npos && migrated_json.find("monitor_hardware_id") == std::string::npos,
          "next save after migration writes only schema v2");
}

void decision_test() {
    asc::DecisionEngine engine;
    const asc::SourceAvailability ready{true, true, true};
    check(engine.decide(asc::OutputMode::automatic, asc::DetectionState::reference_missing, asc::Source::screen, ready).desired_output == asc::Source::camera,
          "missing reference always selects webcam");
    check(engine.decide(asc::OutputMode::force_screen, asc::DetectionState::not_matching, asc::Source::camera, {true, false, true}).desired_output == asc::Source::camera,
          "unavailable screen falls back to webcam");
    check(engine.decide(asc::OutputMode::force_camera, asc::DetectionState::matching, asc::Source::screen, {false, true, true}).desired_output == asc::Source::placeholder,
          "unavailable webcam produces placeholder");
}

} // namespace

int main() {
    frame_test();
    detector_test();
    transition_test();
    monitor_test();
    config_test();
    decision_test();
    if (failures != 0) return EXIT_FAILURE;
    std::cout << "All core tests passed\n";
    return EXIT_SUCCESS;
}
