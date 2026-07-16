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
enum class DetectionState { unknown, matching, not_matching, reference_missing };
enum class DeviceState { unavailable, initializing, ready, recovering, failed };

struct Size {
    std::uint32_t width{0};
    std::uint32_t height{0};
    friend bool operator==(const Size&, const Size&) = default;
};

struct MonitorGeometry {
    std::int32_t x{0};
    std::int32_t y{0};
    std::uint32_t width{0};
    std::uint32_t height{0};
    friend bool operator==(const MonitorGeometry&, const MonitorGeometry&) = default;
};

// Runtime-only monitor metadata. It intentionally contains no EDID or other
// persisted hardware identity; the GDI name is valid only for this session.
struct RuntimeMonitorDescriptor {
    std::string gdi_display_name;
    std::string label;
    MonitorGeometry geometry;
    std::uintptr_t native_handle{0};

    [[nodiscard]] std::string runtime_key() const { return gdi_display_name; }
    friend bool operator==(const RuntimeMonitorDescriptor&, const RuntimeMonitorDescriptor&) = default;
};

struct MonitorScore {
    RuntimeMonitorDescriptor monitor;
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
