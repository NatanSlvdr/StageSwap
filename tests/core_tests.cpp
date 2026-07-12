#include "asc/core/decision_engine.hpp"
#include "asc/core/config.hpp"
#include "asc/core/controller.hpp"
#include "asc/core/detector.hpp"
#include "asc/core/image.hpp"
#include "asc/core/monitor_tracker.hpp"
#include "asc/core/transition.hpp"

#include <chrono>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <fstream>
#include <string_view>

namespace {

int failures = 0;

void check(const bool condition, const std::string_view message) {
    if (!condition) {
        ++failures;
        std::cerr << "FAIL: " << message << '\n';
    }
}

asc::MonitorIdentity monitor(std::string id, int x = 0) {
    asc::MonitorIdentity value;
    value.device_path = "path-" + id;
    value.hardware_id = std::move(id);
    value.manufacturer = "Example";
    value.model = "Panel";
    value.resolution = {1920, 1080};
    value.desktop_x = x;
    return value;
}

void detector_test() {
    asc::DebouncedDetector detector({.threshold = 0.98, .matches_required = 5, .mismatches_required = 3});
    const auto now = asc::Clock::now();
    for (int i = 0; i < 4; ++i) check(detector.update(0.99, true, now).state == asc::DetectionState::unknown, "match debounce holds");
    check(detector.update(0.99, true, now).state == asc::DetectionState::matching, "fifth match confirms");
    for (int i = 0; i < 2; ++i) check(detector.update(0.1, true, now).state == asc::DetectionState::matching, "mismatch debounce holds");
    check(detector.update(0.1, true, now).state == asc::DetectionState::not_matching, "third mismatch confirms loss");
    check(detector.update(0.0, false, now).state == asc::DetectionState::reference_missing, "capture failure is missing");
}

void decision_test() {
    asc::DecisionEngine engine;
    const asc::SourceAvailability ready{true, true, true};
    check(engine.decide(asc::OutputMode::automatic, asc::DetectionState::matching, asc::Source::screen, ready).desired_output == asc::Source::camera,
          "match chooses camera");
    check(engine.decide(asc::OutputMode::automatic, asc::DetectionState::not_matching, asc::Source::camera, ready).desired_output == asc::Source::screen,
          "loss chooses screen");
    check(engine.decide(asc::OutputMode::force_camera, asc::DetectionState::not_matching, asc::Source::screen, ready).desired_output == asc::Source::camera,
          "manual camera override wins");
    check(engine.decide(asc::OutputMode::automatic, asc::DetectionState::reference_missing, asc::Source::screen, ready).desired_output == asc::Source::camera,
          "missing reference defaults to camera");
    check(engine.decide(asc::OutputMode::force_screen, asc::DetectionState::matching, asc::Source::camera, {true, false, true}).desired_output == asc::Source::camera,
          "unavailable screen falls safely to camera");
}

void transition_test() {
    using namespace std::chrono_literals;
    const auto start = asc::Clock::now();
    asc::TransitionController transition(500ms);
    [[maybe_unused]] const auto initial = transition.tick(start);
    check(transition.request(asc::Source::screen, start).active, "screen fade starts");
    auto state = transition.tick(start + 300ms);
    check(std::abs(state.screen_mix - 0.6) < 0.001, "fade reaches sixty percent");
    state = transition.request(asc::Source::camera, start + 300ms);
    check(state.reversed && std::abs(state.screen_mix - 0.6) < 0.001, "reversal preserves blend");
    state = transition.tick(start + 450ms);
    check(std::abs(state.screen_mix - 0.3) < 0.001, "reverse continues smoothly");
    state = transition.tick(start + 600ms);
    check(!state.active && state.logical_source == asc::Source::camera && state.screen_mix == 0.0, "reverse completes at camera");
}

void image_test() {
    asc::GrayImage a{{160, 90}, std::vector<std::uint8_t>(160 * 90)};
    for (std::uint32_t y = 0; y < 90; ++y) {
        for (std::uint32_t x = 0; x < 160; ++x) a.pixels[y * 160 + x] = static_cast<std::uint8_t>((x + y) % 256);
    }
    auto b = a;
    check(asc::image_similarity(a, b) > 0.9999, "identical images match");
    for (auto& pixel : b.pixels) pixel = static_cast<std::uint8_t>(std::min(255, static_cast<int>(pixel) + 2));
    check(asc::image_similarity(a, b) > 0.98, "minor brightness change tolerated");
    for (auto& pixel : b.pixels) pixel = static_cast<std::uint8_t>(255 - pixel);
    check(asc::image_similarity(a, b) < 0.5, "different image rejected");
}

void monitor_test() {
    asc::MonitorTracker tracker({.match_threshold = 0.98, .reassignment_margin = 0.01, .confirmations_required = 3});
    const auto a = monitor("A");
    const auto b = monitor("B", 1920);
    tracker.restore_preferred(a);
    const std::vector<asc::MonitorScore> scores{{a, 0.12, true}, {b, 0.993, true}};
    check(!tracker.apply_scan(scores).changed, "first monitor scan does not reassign");
    check(!tracker.apply_scan(scores).changed, "second monitor scan does not reassign");
    const auto third = tracker.apply_scan(scores);
    check(third.changed && third.tracked && third.tracked->hardware_id == "B", "third scan reassigns");

    const auto c = monitor("C", 3840);
    const auto duplicate = tracker.apply_scan({{b, 0.991, true}, {c, 0.992, true}});
    check(!duplicate.changed && duplicate.tracked->hardware_id == "B", "duplicate match retains current monitor");
    const auto missing = tracker.apply_scan({{b, 0.5, true}, {c, 0.4, true}});
    check(missing.scan_state == asc::DetectionState::reference_missing && missing.tracked->hardware_id == "B", "missing reference retains monitor");

    asc::MonitorTracker stronger({.match_threshold = 0.98, .reassignment_margin = 0.01, .confirmations_required = 3});
    stronger.restore_preferred(a);
    const std::vector<asc::MonitorScore> stronger_scores{{a, 0.981, true}, {b, 0.999, true}};
    [[maybe_unused]] const auto stronger_first = stronger.apply_scan(stronger_scores);
    [[maybe_unused]] const auto stronger_second = stronger.apply_scan(stronger_scores);
    check(stronger.apply_scan(stronger_scores).changed && stronger.tracked()->hardware_id == "B",
          "clearly stronger match can replace a weak-but-valid current monitor");
}

void config_test() {
    asc::AppConfig original;
    original.selected_video_device_id = "camera\\id\"one";
    original.detector.threshold = 0.975;
    original.output_mode = asc::OutputMode::force_camera;
    original.log_level = asc::LogLevel::warning;
    original.video_auto_reconnect = false;
    original.video_reconnect_interval = std::chrono::seconds{7};
    original.last_tracked_monitor = monitor("DISPLAY-A");
    std::string error;
    const auto parsed = asc::ConfigStore::parse(asc::ConfigStore::serialize(original), error);
    check(parsed.has_value(), "serialized configuration parses");
    check(parsed && parsed->selected_video_device_id == original.selected_video_device_id, "configuration strings round trip");
    check(parsed && parsed->output_mode == asc::OutputMode::force_camera, "configuration enum round trips");
    check(parsed && parsed->log_level == asc::LogLevel::warning && !parsed->video_auto_reconnect && parsed->video_reconnect_interval == std::chrono::seconds{7},
          "logging and reconnect policy round trip");
    check(parsed && parsed->last_tracked_monitor && parsed->last_tracked_monitor->hardware_id == "DISPLAY-A", "monitor identity round trips");
    check(!asc::ConfigStore::parse("{\"schema_version\":1,\"detection_interval_ms\":1}", error), "invalid critical range rejected");
    check(!asc::ConfigStore::parse("{\"schema_version\":1,\"video_reconnect_interval_seconds\":0}", error), "invalid reconnect interval rejected");

    const auto directory = std::filesystem::temp_directory_path() / "asc-config-test";
    std::filesystem::remove_all(directory);
    asc::ConfigStore store(directory);
    store.save(original);
    auto second = original; second.selected_video_device_id = "second-camera"; store.save(second);
    { std::ofstream corrupt(store.config_path(), std::ios::trunc); corrupt << "not json"; }
    const auto recovered = store.load();
    check(recovered.used_backup && recovered.config.selected_video_device_id == original.selected_video_device_id,
          "invalid primary configuration falls back to last valid backup");
    check(std::filesystem::exists(directory / "config.invalid.json"), "invalid configuration is preserved");
    store.save(recovered.config);
    { std::string backup_error; std::ifstream backup(directory / "config.backup.json"); const std::string backup_json{std::istreambuf_iterator<char>(backup), std::istreambuf_iterator<char>()};
      const auto still_valid = asc::ConfigStore::parse(backup_json, backup_error);
      check(still_valid.has_value(), "saving recovered settings does not overwrite a valid backup with an invalid primary"); }
    std::filesystem::remove_all(directory);
}

void event_log_test() {
    const auto directory = std::filesystem::temp_directory_path() / "asc-event-log-test";
    std::filesystem::remove_all(directory);
    asc::EventLog log(directory, 14, 2);
    log.write(asc::LogLevel::info, "test", "ONE", "first");
    log.write(asc::LogLevel::warning, "test", "TWO", "second");
    log.write(asc::LogLevel::error, "test", "THREE", "third");
    check(log.recent().size() == 2, "recent event list is bounded");
    const auto exported = directory / "export.txt";
    log.export_to(exported);
    std::ifstream input(exported);
    const std::string contents{std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>()};
    check(contents.find("\"event_code\":\"THREE\"") != std::string::npos, "structured log exports JSON Lines");
    log.clear();
    check(log.recent().empty(), "clearing logs clears recent events");
    std::filesystem::remove_all(directory);
}

void controller_test() {
    namespace fs = std::filesystem;
    const auto directory = fs::temp_directory_path() / "asc-controller-test";
    fs::create_directories(directory);
    asc::EventLog log(directory, 1);
    asc::AppConfig config;
    config.detector.matches_required = 1;
    config.detector.mismatches_required = 1;
    asc::AppController controller(config, log);
    controller.begin_start();
    controller.finish_start(true, true, true);
    const auto now = asc::Clock::now();
    controller.on_similarity(0.1, true, now);
    check(controller.status().automatic_target == asc::Source::screen, "controller targets screen on loss");
    [[maybe_unused]] const auto missing_scan = controller.on_monitor_scan({}, now);
    check(controller.status().automatic_target == asc::Source::camera, "full-scan missing state applies safe camera behavior");
    controller.set_mode(asc::OutputMode::force_camera, now);
    check(controller.status().transition.target == asc::Source::camera, "controller override targets camera");
    controller.stop();
    check(controller.status().run_state == asc::RunState::stopped && controller.status().transition.target == asc::Source::camera,
          "stopping retains safe camera target");
    controller.set_component_state(asc::Source::screen, asc::DeviceState::recovering, now);
    controller.set_component_state(asc::Source::screen, asc::DeviceState::ready, now);
    check(controller.status().transition.target == asc::Source::camera, "background recovery cannot expose screen while stopped");

    asc::AppController scan_controller(config, log);
    scan_controller.begin_start(); scan_controller.finish_start(true, true, true);
    scan_controller.on_similarity(0.1, true, now);
    [[maybe_unused]] const auto found_elsewhere = scan_controller.on_monitor_scan({{monitor("scan-match"), 0.995, true}}, now);
    check(scan_controller.status().automatic_target == asc::Source::camera,
          "full scan match immediately hides a reference found on another monitor");
    fs::remove_all(directory);
}

} // namespace

int main() {
    detector_test();
    decision_test();
    transition_test();
    image_test();
    monitor_test();
    config_test();
    event_log_test();
    controller_test();
    if (failures != 0) {
        std::cerr << failures << " test(s) failed\n";
        return EXIT_FAILURE;
    }
    std::cout << "All core tests passed\n";
    return EXIT_SUCCESS;
}
