#pragma once

#include "asc/core/types.hpp"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>

namespace asc {

namespace detail {

[[nodiscard]] inline bool pixel_rows_size(const std::size_t row_count, const std::size_t row_stride,
                                          const std::size_t row_bytes, std::size_t& result) noexcept {
    if (row_count == 0 || row_bytes == 0 || row_stride < row_bytes) return false;
    const auto rows_before_last = row_count - 1;
    if (rows_before_last > (std::numeric_limits<std::size_t>::max() - row_bytes) / row_stride) return false;
    result = rows_before_last * row_stride + row_bytes;
    return true;
}

[[nodiscard]] inline int divide_floor_256(const int value) noexcept {
    return value >= 0 ? value / 256 : -((-value + 255) / 256);
}

[[nodiscard]] inline std::uint8_t clamp_pixel_byte(const int value) noexcept {
    return static_cast<std::uint8_t>(std::clamp(value, 0, 255));
}

} // namespace detail

// Converts BGRA8 to studio-range BT.709 NV12. The UV plane starts immediately
// after size.height full Y-stride rows. Invalid layouts are rejected before any
// destination byte is written.
[[nodiscard]] inline bool bgra_to_nv12(const std::span<const std::uint8_t> bgra, const Size size,
                                       const std::size_t bgra_stride, const std::span<std::uint8_t> nv12,
                                       const std::size_t nv12_stride) noexcept {
    if (size.width == 0 || size.height == 0 || (size.width & 1u) != 0 || (size.height & 1u) != 0) return false;

    const auto width = static_cast<std::size_t>(size.width);
    const auto height = static_cast<std::size_t>(size.height);
    if (width > std::numeric_limits<std::size_t>::max() / 4 ||
        height > std::numeric_limits<std::size_t>::max() - height / 2) return false;

    std::size_t required_bgra = 0;
    std::size_t required_nv12 = 0;
    if (!detail::pixel_rows_size(height, bgra_stride, width * 4, required_bgra) ||
        !detail::pixel_rows_size(height + height / 2, nv12_stride, width, required_nv12) ||
        bgra.size() < required_bgra || nv12.size() < required_nv12) return false;

    auto* const y_plane = nv12.data();
    auto* const uv_plane = nv12.data() + nv12_stride * height;
    for (std::size_t y = 0; y < height; ++y) {
        for (std::size_t x = 0; x < width; ++x) {
            const auto* const pixel = bgra.data() + y * bgra_stride + x * 4;
            const int b = pixel[0];
            const int g = pixel[1];
            const int r = pixel[2];
            y_plane[y * nv12_stride + x] = detail::clamp_pixel_byte(
                16 + detail::divide_floor_256(47 * r + 157 * g + 16 * b + 128));
        }
    }

    for (std::size_t y = 0; y < height; y += 2) {
        for (std::size_t x = 0; x < width; x += 2) {
            int sum_u = 0;
            int sum_v = 0;
            for (std::size_t dy = 0; dy < 2; ++dy) {
                for (std::size_t dx = 0; dx < 2; ++dx) {
                    const auto* const pixel = bgra.data() + (y + dy) * bgra_stride + (x + dx) * 4;
                    const int b = pixel[0];
                    const int g = pixel[1];
                    const int r = pixel[2];
                    sum_u += 128 + detail::divide_floor_256(-26 * r - 87 * g + 112 * b + 128);
                    sum_v += 128 + detail::divide_floor_256(112 * r - 102 * g - 10 * b + 128);
                }
            }
            const auto offset = (y / 2) * nv12_stride + x;
            uv_plane[offset] = detail::clamp_pixel_byte(sum_u / 4);
            uv_plane[offset + 1] = detail::clamp_pixel_byte(sum_v / 4);
        }
    }
    return true;
}

} // namespace asc
