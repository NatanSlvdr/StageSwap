#pragma once

#include "asc/core/types.hpp"

#include <chrono>
#include <cstdint>
#include <span>
#include <vector>

namespace asc {

inline constexpr Size pipeline_size{1280, 720};
inline constexpr std::uint32_t pipeline_fps = 30;
inline constexpr auto frame_stale_after = std::chrono::seconds{1};

enum class FrameFreshness { live, retained, stale };

struct Frame {
    std::vector<std::uint8_t> bgra;
    Size size;
    std::uint32_t stride{0};
    TimePoint received_at{};
    std::int64_t presentation_time_100ns{0};
    std::uint64_t sequence{0};
    FrameFreshness freshness{FrameFreshness::live};

    [[nodiscard]] bool valid() const noexcept;
    [[nodiscard]] bool fresh(TimePoint now, std::chrono::milliseconds maximum_age = frame_stale_after) const noexcept;
};

[[nodiscard]] Frame make_placeholder(Size size, std::uint32_t color_bgra, std::int64_t timestamp_100ns = 0);
[[nodiscard]] Frame aspect_fit_bgra(const Frame& source, Size output = pipeline_size);
[[nodiscard]] Frame blend_bgra(const Frame& camera, const Frame& screen, double screen_mix,
                               std::uint32_t placeholder_color_bgra, Size output = pipeline_size,
                               std::int64_t timestamp_100ns = 0);

} // namespace asc
