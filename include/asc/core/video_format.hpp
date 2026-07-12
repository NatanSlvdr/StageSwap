#pragma once

#include "asc/core/types.hpp"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>
#include <tuple>
#include <vector>

namespace asc {

// Platform capture backends assign subtype_rank (lower is better) while this
// portable policy ranks resolution and cadence independently of Media Foundation.
struct CaptureFormatCandidate {
    Size size;
    std::uint32_t frame_rate_numerator{0};
    std::uint32_t frame_rate_denominator{1};
    std::uint32_t subtype_rank{0};
    std::size_t native_index{0};
};

namespace detail {

inline std::uint64_t relative_distance(const std::uint64_t actual, const std::uint64_t preferred) noexcept {
    if (preferred == 0) return actual == 0 ? 0 : 10'000;
    const auto difference = actual > preferred ? actual - preferred : preferred - actual;
    constexpr auto maximum_component = std::numeric_limits<std::uint64_t>::max() / 8;
    if (difference > maximum_component / 10'000) return maximum_component;
    return (difference * 10'000) / preferred;
}

inline auto capture_format_rank(const CaptureFormatCandidate& format, const Size preferred_size,
                                const std::uint32_t preferred_fps) noexcept {
    const auto width_distance = relative_distance(format.size.width, preferred_size.width);
    const auto height_distance = relative_distance(format.size.height, preferred_size.height);
    const auto preferred_rate = static_cast<std::uint64_t>(preferred_fps) * format.frame_rate_denominator;
    const auto rate_distance = relative_distance(format.frame_rate_numerator, preferred_rate);
    const auto aspect_left = static_cast<std::uint64_t>(format.size.width) * preferred_size.height;
    const auto aspect_right = static_cast<std::uint64_t>(preferred_size.width) * format.size.height;
    const auto aspect_distance = relative_distance(aspect_left, aspect_right);

    // Cadence gets twice the weight of each dimension: a slightly smaller 30 fps
    // stream is preferable to an exact-resolution stream with an unusably low fps.
    const auto total_distance = width_distance + height_distance + (rate_distance * 2) + aspect_distance;
    return std::tuple{total_distance, width_distance + height_distance, rate_distance,
                      aspect_distance, format.subtype_rank, format.native_index};
}

} // namespace detail

[[nodiscard]] inline std::vector<CaptureFormatCandidate> rank_capture_formats(
    const std::span<const CaptureFormatCandidate> formats, const Size preferred_size,
    const std::uint32_t preferred_fps) {
    std::vector<CaptureFormatCandidate> ranked;
    ranked.reserve(formats.size());
    for (const auto& format : formats) {
        if (format.size.width == 0 || format.size.height == 0 ||
            format.frame_rate_numerator == 0 || format.frame_rate_denominator == 0) continue;
        ranked.push_back(format);
    }
    std::stable_sort(ranked.begin(), ranked.end(), [&](const auto& left, const auto& right) {
        return detail::capture_format_rank(left, preferred_size, preferred_fps) <
               detail::capture_format_rank(right, preferred_size, preferred_fps);
    });
    return ranked;
}

} // namespace asc
