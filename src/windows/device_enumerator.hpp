#pragma once

#include "asc/core/types.hpp"
#include <windows.h>

#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace asc::win {

struct VideoDevice { std::string name; std::string identifier; bool connected{true}; };
struct MonitorDevice { HMONITOR handle{nullptr}; asc::RuntimeMonitorDescriptor descriptor; };

[[nodiscard]] std::vector<VideoDevice> enumerate_video_devices();
[[nodiscard]] std::optional<std::string> find_video_device_name(std::string_view identifier);
[[nodiscard]] std::vector<MonitorDevice> enumerate_monitors();

} // namespace asc::win
