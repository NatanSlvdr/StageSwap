#pragma once

#include "asc/core/types.hpp"

#include <filesystem>
#include <optional>
#include <string>
#include <vector>

namespace asc {

struct AppConfig {
    std::uint32_t schema_version{2};
    std::string selected_video_device_id;
    std::string reference_image_path;
    double similarity_threshold{0.98};
    bool cursor_visible{false};
    bool start_with_windows{false};
    bool start_minimized{true};
    bool start_automatically{true};
    bool close_to_tray{true};
    bool show_notifications{true};
    std::string interface_language{"en-US"};
    bool confirm_exit{true};
    OutputMode output_mode{OutputMode::automatic};
    std::uint32_t placeholder_color_bgra{0xff171719u};
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
