#include "asc/core/decision_engine.hpp"
#include "asc/core/config.hpp"
#include "asc/core/controller.hpp"
#include "asc/core/detector.hpp"
#include "asc/core/image.hpp"
#include "asc/core/monitor_tracker.hpp"
#include "asc/core/pixel_conversion.hpp"
#include "asc/core/shared_frame_validation.hpp"
#include "asc/core/transition.hpp"
#include "asc/core/video_format.hpp"

#include <array>
#include <atomic>
#include <barrier>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <fstream>
#include <limits>
#include <string_view>
#include <thread>

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

void pixel_conversion_test() {
    const auto solid_bgra = [](const std::uint8_t blue, const std::uint8_t green, const std::uint8_t red) {
        std::array<std::uint8_t, 16> pixels{};
        for (std::size_t offset = 0; offset < pixels.size(); offset += 4) {
            pixels[offset] = blue;
            pixels[offset + 1] = green;
            pixels[offset + 2] = red;
            pixels[offset + 3] = 255;
        }
        return pixels;
    };

    const auto black = solid_bgra(0, 0, 0);
    std::array<std::uint8_t, 6> nv12{};
    check(asc::bgra_to_nv12(black, {2, 2}, 8, nv12, 2), "black BGRA converts to NV12");
    check(nv12 == std::array<std::uint8_t, 6>{16, 16, 16, 16, 128, 128},
          "black conversion has studio-range luma and neutral chroma");

    const auto white = solid_bgra(255, 255, 255);
    check(asc::bgra_to_nv12(white, {2, 2}, 8, nv12, 2), "white BGRA converts to NV12");
    check(nv12 == std::array<std::uint8_t, 6>{235, 235, 235, 235, 127, 128},
          "white conversion preserves the production BT.709 integer result");

    const auto red = solid_bgra(0, 0, 255);
    check(asc::bgra_to_nv12(red, {2, 2}, 8, nv12, 2), "color BGRA converts to NV12");
    check(nv12 == std::array<std::uint8_t, 6>{63, 63, 63, 63, 102, 240},
          "red conversion has the expected BT.709 luma and chroma");

    std::array<std::uint8_t, 20> padded_bgra{};
    std::copy(black.begin(), black.begin() + 8, padded_bgra.begin());
    std::copy(black.begin() + 8, black.end(), padded_bgra.begin() + 12);
    std::array<std::uint8_t, 12> padded_nv12{};
    padded_nv12.fill(0xcc);
    check(asc::bgra_to_nv12(padded_bgra, {2, 2}, 12, padded_nv12, 4),
          "conversion accepts valid padded strides");
    check(padded_nv12 == std::array<std::uint8_t, 12>{16, 16, 0xcc, 0xcc, 16, 16, 0xcc, 0xcc,
                                                     128, 128, 0xcc, 0xcc},
          "conversion leaves destination row padding untouched");

    const auto rejects_without_writing = [&](const std::span<const std::uint8_t> source, const asc::Size size,
                                             const std::size_t source_stride, const std::size_t output_stride) {
        std::array<std::uint8_t, 6> output{};
        output.fill(0xcc);
        const bool converted = asc::bgra_to_nv12(source, size, source_stride, output, output_stride);
        return !converted && std::all_of(output.begin(), output.end(), [](const auto byte) { return byte == 0xcc; });
    };
    check(rejects_without_writing(black, {2, 2}, 7, 2), "conversion rejects a short BGRA stride before writing");
    check(rejects_without_writing(std::span<const std::uint8_t>(black).first(15), {2, 2}, 8, 2),
          "conversion rejects a truncated BGRA buffer before writing");
    check(rejects_without_writing(black, {2, 2}, 8, 1), "conversion rejects a short NV12 stride before writing");
    check(rejects_without_writing(black, {2, 1}, 8, 2), "conversion rejects odd NV12 dimensions before writing");

    std::array<std::uint8_t, 5> short_nv12{};
    short_nv12.fill(0xcc);
    check(!asc::bgra_to_nv12(black, {2, 2}, 8, short_nv12, 2) &&
              std::all_of(short_nv12.begin(), short_nv12.end(), [](const auto byte) { return byte == 0xcc; }),
          "conversion rejects a truncated NV12 buffer before writing");
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

bool valid_source(const asc::Source source) {
    switch (source) {
    case asc::Source::camera:
    case asc::Source::screen:
    case asc::Source::placeholder:
        return true;
    }
    return false;
}

bool valid_mode(const asc::OutputMode mode) {
    switch (mode) {
    case asc::OutputMode::automatic:
    case asc::OutputMode::force_camera:
    case asc::OutputMode::force_screen:
        return true;
    }
    return false;
}

bool valid_detection_state(const asc::DetectionState state) {
    switch (state) {
    case asc::DetectionState::unknown:
    case asc::DetectionState::matching:
    case asc::DetectionState::not_matching:
    case asc::DetectionState::reference_missing:
    case asc::DetectionState::ambiguous:
        return true;
    }
    return false;
}

bool valid_device_state(const asc::DeviceState state) {
    switch (state) {
    case asc::DeviceState::unavailable:
    case asc::DeviceState::initializing:
    case asc::DeviceState::ready:
    case asc::DeviceState::recovering:
    case asc::DeviceState::failed:
        return true;
    }
    return false;
}

bool valid_run_state(const asc::RunState state) {
    switch (state) {
    case asc::RunState::stopped:
    case asc::RunState::starting:
    case asc::RunState::running:
    case asc::RunState::recovering:
    case asc::RunState::stopping:
    case asc::RunState::error:
        return true;
    }
    return false;
}

bool coherent_status_snapshot(const asc::AppStatus& status) {
    const auto& transition = status.transition;
    const bool transition_is_valid =
        valid_source(transition.logical_source) && valid_source(transition.target) &&
        std::isfinite(transition.screen_mix) && transition.screen_mix >= 0.0 && transition.screen_mix <= 1.0 &&
        transition.remaining >= std::chrono::milliseconds{0} &&
        transition.remaining <= std::chrono::milliseconds{2000} &&
        (!transition.active || transition.target != asc::Source::placeholder) &&
        (transition.active || status.actual_output == transition.logical_source);
    const bool detection_is_valid =
        valid_detection_state(status.detection.state) && std::isfinite(status.detection.similarity) &&
        status.detection.similarity >= 0.0 && status.detection.similarity <= 1.0 &&
        (status.detection.consecutive_matches == 0 || status.detection.consecutive_mismatches == 0);
    const bool components_are_coherent =
        valid_device_state(status.video_input) && valid_device_state(status.screen_capture) &&
        valid_device_state(status.virtual_camera) &&
        status.availability.camera_ready == (status.video_input == asc::DeviceState::ready) &&
        status.availability.screen_ready == (status.screen_capture == asc::DeviceState::ready) &&
        status.availability.placeholder_ready;
    return valid_run_state(status.run_state) && valid_mode(status.mode) &&
           valid_source(status.automatic_target) && valid_source(status.actual_output) &&
           transition_is_valid && detection_is_valid && components_are_coherent;
}

bool coherent_config_snapshot(const asc::AppConfig& config) {
    if (config.selected_video_device_id == "stress-base") {
        return config.detector.threshold == 0.93 && config.detector.matches_required == 2 &&
               config.fade_duration == std::chrono::milliseconds{61} && config.output_size == asc::Size{1280, 720};
    }
    if (config.selected_video_device_id == "stress-a") {
        return config.detector.threshold == 0.91 && config.detector.matches_required == 3 &&
               config.fade_duration == std::chrono::milliseconds{79} && config.output_size == asc::Size{1600, 900};
    }
    if (config.selected_video_device_id == "stress-b") {
        return config.detector.threshold == 0.99 && config.detector.matches_required == 7 &&
               config.fade_duration == std::chrono::milliseconds{137} && config.output_size == asc::Size{1920, 1080};
    }
    return false;
}

void controller_concurrency_test() {
    namespace fs = std::filesystem;
    using namespace std::chrono_literals;

    const auto directory = fs::temp_directory_path() / "asc-controller-concurrency-test";
    fs::remove_all(directory);
    fs::create_directories(directory);
    asc::EventLog log(directory, 1, 8);

    asc::AppConfig base;
    base.selected_video_device_id = "stress-base";
    base.detector = {.threshold = 0.93, .matches_required = 2, .mismatches_required = 2};
    base.fade_duration = 61ms;
    base.output_size = {1280, 720};

    auto config_a = base;
    config_a.selected_video_device_id = "stress-a";
    config_a.detector = {.threshold = 0.91, .matches_required = 3, .mismatches_required = 1};
    config_a.fade_duration = 79ms;
    config_a.output_size = {1600, 900};
    config_a.output_mode = asc::OutputMode::automatic;
    config_a.missing_behavior = asc::MissingReferenceBehavior::use_camera;

    auto config_b = base;
    config_b.selected_video_device_id = "stress-b";
    config_b.detector = {.threshold = 0.99, .matches_required = 7, .mismatches_required = 5};
    config_b.fade_duration = 137ms;
    config_b.output_size = {1920, 1080};
    config_b.output_mode = asc::OutputMode::force_screen;
    config_b.missing_behavior = asc::MissingReferenceBehavior::use_placeholder;

    asc::AppController controller(base, log);
    controller.begin_start();
    controller.finish_start(true, true, true);
    const auto fixed_now = asc::Clock::now();
    constexpr int iterations = 250;
    std::barrier round_start{4};
    std::barrier round_finish{4};
    std::atomic<int> invalid_snapshots{0};

    const auto mode_and_detection_writer = [&] {
        for (int iteration = 0; iteration < iterations; ++iteration) {
            round_start.arrive_and_wait();
            const auto mode = iteration % 3 == 0 ? asc::OutputMode::automatic :
                              iteration % 3 == 1 ? asc::OutputMode::force_camera : asc::OutputMode::force_screen;
            controller.set_mode(mode, fixed_now);
            controller.on_similarity(iteration % 2 == 0 ? 0.1 : 0.999, iteration % 11 != 0, fixed_now);
            controller.tick(fixed_now);
            round_finish.arrive_and_wait();
        }
    };
    const auto configuration_and_component_writer = [&] {
        for (int iteration = 0; iteration < iterations; ++iteration) {
            round_start.arrive_and_wait();
            controller.reconfigure(iteration % 2 == 0 ? config_a : config_b, fixed_now);
            controller.set_component_state(asc::Source::camera,
                                           iteration % 5 == 0 ? asc::DeviceState::recovering : asc::DeviceState::ready,
                                           fixed_now);
            controller.set_component_state(asc::Source::screen,
                                           iteration % 7 == 0 ? asc::DeviceState::failed : asc::DeviceState::ready,
                                           fixed_now);
            controller.set_virtual_camera_state(iteration % 13 == 0 ? asc::DeviceState::failed : asc::DeviceState::ready);
            controller.set_tracked_monitor(monitor(iteration % 2 == 0 ? "stress-a" : "stress-b"));
            round_finish.arrive_and_wait();
        }
    };
    const auto snapshot_reader = [&] {
        for (int iteration = 0; iteration < iterations; ++iteration) {
            round_start.arrive_and_wait();
            for (int sample = 0; sample < 4; ++sample) {
                if (!coherent_status_snapshot(controller.status())) invalid_snapshots.fetch_add(1, std::memory_order_relaxed);
                if (!coherent_config_snapshot(controller.config())) invalid_snapshots.fetch_add(1, std::memory_order_relaxed);
            }
            round_finish.arrive_and_wait();
        }
    };

    std::thread mode_thread(mode_and_detection_writer);
    std::thread configuration_thread(configuration_and_component_writer);
    std::thread first_reader(snapshot_reader);
    std::thread second_reader(snapshot_reader);
    mode_thread.join();
    configuration_thread.join();
    first_reader.join();
    second_reader.join();

    check(invalid_snapshots.load(std::memory_order_relaxed) == 0,
          "concurrent controller snapshots remain coherent during state and configuration changes");

    const auto finish_time = fixed_now + 10s;
    controller.reconfigure(config_a, finish_time);
    controller.set_component_state(asc::Source::camera, asc::DeviceState::ready, finish_time);
    controller.set_component_state(asc::Source::screen, asc::DeviceState::ready, finish_time);
    controller.set_virtual_camera_state(asc::DeviceState::ready);
    controller.set_mode(asc::OutputMode::automatic, finish_time);
    controller.on_similarity(0.1, true, finish_time);
    controller.tick(finish_time + 1s);
    const auto final_status = controller.status();
    check(final_status.run_state == asc::RunState::running &&
              final_status.automatic_target == asc::Source::screen &&
              final_status.actual_output == asc::Source::screen && !final_status.transition.active,
          "controller reaches a deterministic state after concurrent mutations complete");
    fs::remove_all(directory);
}

void video_format_test() {
    const std::vector<asc::CaptureFormatCandidate> formats{
        {{1920, 1080}, 5, 1, 0, 0},
        {{1280, 720}, 30, 1, 3, 1},
        {{1920, 1080}, 30'000, 1001, 3, 2},
        {{1920, 1080}, 30, 1, 0, 3},
        {{0, 0}, 30, 1, 0, 4},
    };
    const auto ranked = asc::rank_capture_formats(formats, {1920, 1080}, 30);
    check(ranked.size() == 4, "format negotiation rejects malformed native formats");
    check(!ranked.empty() && ranked.front().native_index == 3,
          "format negotiation prefers exact size, cadence, and efficient subtype");
    check(ranked.size() > 1 && ranked[1].native_index == 2,
          "format negotiation treats 29.97 fps as a close fallback");
    check(ranked.size() > 2 && ranked[2].native_index == 1,
          "format negotiation prefers a smaller usable stream to exact resolution at very low fps");
}

void shared_frame_validation_test() {
    using enum asc::SharedFrameMetadataStatus;
    constexpr std::uint64_t maximum_frame_bytes = 1920ull * 1080 * 4;
    const auto validate = [](const std::uint32_t width, const std::uint32_t height,
                             const std::uint32_t stride, const std::uint32_t frame_bytes) {
        return asc::validate_shared_frame_metadata(width, height, stride, frame_bytes, maximum_frame_bytes);
    };

    check(validate(1920, 1080, 7680, 8'294'400) == frame, "valid IPC frame metadata is accepted");
    check(validate(1, 1, 4, 4) == frame, "minimum-size IPC frame metadata is accepted");
    check(validate(0, 0, 0, 0) == invalidation, "zeroed IPC invalidation packet is accepted");

    check(validate(0, 1080, 0, 1) == invalid && validate(1920, 0, 7680, 1) == invalid,
          "zero IPC frame dimensions are rejected");
    check(validate(1920, 1080, 7679, 8'294'400) == invalid,
          "incorrect IPC frame stride is rejected");
    check(validate(1920, 1080, 7680, 8'294'399) == invalid,
          "incorrect IPC frame length is rejected");
    check(validate(1921, 1080, 7684, 8'298'720) == invalid,
          "IPC frames larger than the configured capacity are rejected");
    check(validate(std::numeric_limits<std::uint32_t>::max(), 1, 0,
                   std::numeric_limits<std::uint32_t>::max()) == invalid,
          "overflowing IPC frame stride is rejected");
    check(validate(1'073'741'823, 2, 4'294'967'292u, 4'294'967'288u) == invalid,
          "overflowing IPC frame length is rejected");
    check(validate(1, 0, 0, 0) == invalid && validate(0, 1, 0, 0) == invalid &&
              validate(0, 0, 4, 0) == invalid,
          "non-zero IPC invalidation metadata is rejected");
}

} // namespace

int main() {
    detector_test();
    decision_test();
    transition_test();
    image_test();
    pixel_conversion_test();
    monitor_test();
    config_test();
    event_log_test();
    controller_test();
    controller_concurrency_test();
    video_format_test();
    shared_frame_validation_test();
    if (failures != 0) {
        std::cerr << failures << " test(s) failed\n";
        return EXIT_FAILURE;
    }
    std::cout << "All core tests passed\n";
    return EXIT_SUCCESS;
}
