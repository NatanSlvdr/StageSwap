#pragma once

#include "asc/core/detector.hpp"
#include "asc/core/event_log.hpp"
#include "asc/core/monitor_tracker.hpp"
#include "asc/core/types.hpp"

#include <chrono>
#include <filesystem>
#include <optional>
#include <string>
#include <vector>

namespace asc {

struct AppConfig {
    std::uint32_t schema_version{1};
    std::string selected_video_device_id;
    Size preferred_input_size{1920, 1080};
    std::uint32_t preferred_input_fps{30};
    std::string reference_image_path;
    DetectorSettings detector;
    std::chrono::milliseconds detection_interval{250};
    std::chrono::seconds full_scan_interval{30};
    MonitorTrackerSettings monitor_tracker;
    Size output_size{1920, 1080};
    std::uint32_t output_fps{30};
    std::chrono::milliseconds fade_duration{500};
    bool cursor_visible{false};
    bool start_with_windows{false};
    bool start_minimized{true};
    bool start_automatically{true};
    bool close_to_tray{true};
    bool show_notifications{true};
    std::string interface_language{"en-US"};
    bool confirm_exit{true};
    OutputMode output_mode{OutputMode::automatic};
    MissingReferenceBehavior missing_behavior{MissingReferenceBehavior::use_camera};
    ScalingMode camera_scaling{ScalingMode::fit};
    ScalingMode screen_scaling{ScalingMode::fit};
    std::uint32_t placeholder_color_bgra{0xff171719u};
    std::uint32_t log_retention_days{14};
    bool diagnostic_logging{false};
    LogLevel log_level{LogLevel::info};
    bool video_auto_reconnect{true};
    std::chrono::seconds video_reconnect_interval{2};
    std::optional<MonitorIdentity> last_tracked_monitor;
};

struct ConfigLoadResult {
    AppConfig config;
    bool used_backup{false};
    std::vector<std::string> warnings;
};

class ConfigStore {
public:
    explicit ConfigStore(std::filesystem::path directory);
    [[nodiscard]] ConfigLoadResult load() const;
    void save(const AppConfig& config) const;
    [[nodiscard]] const std::filesystem::path& directory() const noexcept { return directory_; }
    [[nodiscard]] std::filesystem::path config_path() const;
    [[nodiscard]] std::filesystem::path reference_path() const;
    [[nodiscard]] std::filesystem::path comparison_path() const;

    [[nodiscard]] static std::string serialize(const AppConfig& config);
    [[nodiscard]] static std::optional<AppConfig> parse(std::string_view json, std::string& error);

private:
    std::filesystem::path directory_;
};

} // namespace asc
