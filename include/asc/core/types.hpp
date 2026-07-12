#pragma once

#include <chrono>
#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace asc {

using Clock = std::chrono::steady_clock;
using TimePoint = Clock::time_point;

enum class OutputMode { automatic, force_camera, force_screen };
enum class Source { camera, screen, placeholder };
enum class DetectionState { unknown, matching, not_matching, reference_missing, ambiguous };
enum class DeviceState { unavailable, initializing, ready, recovering, failed };
enum class MissingReferenceBehavior { use_camera, keep_current, use_last_screen, use_placeholder };
enum class ScalingMode { fit, fill, stretch };

struct Size {
    std::uint32_t width{0};
    std::uint32_t height{0};
    friend bool operator==(const Size&, const Size&) = default;
};

struct MonitorIdentity {
    std::string device_path;
    std::string hardware_id;
    std::string manufacturer;
    std::string model;
    std::string serial;
    std::string adapter_id;
    Size resolution;
    std::uint32_t orientation_degrees{0};
    std::uint32_t refresh_rate_millihz{0};
    std::int32_t desktop_x{0};
    std::int32_t desktop_y{0};

    [[nodiscard]] std::string stable_key() const {
        if (!hardware_id.empty()) return hardware_id + "|" + serial;
        if (!device_path.empty()) return device_path;
        return manufacturer + "|" + model + "|" + std::to_string(desktop_x) + "|" + std::to_string(desktop_y);
    }
};

struct MonitorScore {
    MonitorIdentity monitor;
    double similarity{0.0};
    bool capture_valid{false};
};

struct DetectionSnapshot {
    DetectionState state{DetectionState::unknown};
    double similarity{0.0};
    std::uint32_t consecutive_matches{0};
    std::uint32_t consecutive_mismatches{0};
    TimePoint measured_at{};
};

struct SourceAvailability {
    bool camera_ready{false};
    bool screen_ready{false};
    bool placeholder_ready{true};
};

struct Decision {
    Source automatic_target{Source::camera};
    Source desired_output{Source::camera};
    bool manual_override{false};
    std::string reason;
};

} // namespace asc
