#pragma once

#include "asc/core/types.hpp"
#include <windows.h>

#include <string>
#include <vector>

namespace asc::win {

struct VideoFormat { asc::Size size; std::uint32_t numerator{0}; std::uint32_t denominator{1}; std::string subtype; };
struct VideoDevice { std::string name; std::string identifier; std::vector<VideoFormat> formats; bool connected{true}; };
struct MonitorDevice { HMONITOR handle{nullptr}; asc::MonitorIdentity identity; std::wstring display_name; };

[[nodiscard]] std::vector<VideoDevice> enumerate_video_devices();
[[nodiscard]] std::vector<MonitorDevice> enumerate_monitors();

} // namespace asc::win
