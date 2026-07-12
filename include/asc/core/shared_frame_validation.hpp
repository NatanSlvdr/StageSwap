#pragma once

#include <cstdint>
#include <limits>

namespace asc {

enum class SharedFrameMetadataStatus { invalid, invalidation, frame };

[[nodiscard]] constexpr SharedFrameMetadataStatus validate_shared_frame_metadata(
    const std::uint32_t width, const std::uint32_t height, const std::uint32_t stride,
    const std::uint32_t frame_bytes, const std::uint64_t maximum_frame_bytes) noexcept {
    if (frame_bytes == 0) {
        return width == 0 && height == 0 && stride == 0
            ? SharedFrameMetadataStatus::invalidation
            : SharedFrameMetadataStatus::invalid;
    }
    if (width == 0 || height == 0) return SharedFrameMetadataStatus::invalid;

    constexpr std::uint32_t bytes_per_pixel = 4;
    if (width > std::numeric_limits<std::uint32_t>::max() / bytes_per_pixel)
        return SharedFrameMetadataStatus::invalid;
    const auto expected_stride = width * bytes_per_pixel;
    if (stride != expected_stride) return SharedFrameMetadataStatus::invalid;

    if (height > std::numeric_limits<std::uint64_t>::max() / expected_stride)
        return SharedFrameMetadataStatus::invalid;
    const auto expected_bytes = static_cast<std::uint64_t>(expected_stride) * height;
    if (expected_bytes > maximum_frame_bytes || expected_bytes != frame_bytes)
        return SharedFrameMetadataStatus::invalid;
    return SharedFrameMetadataStatus::frame;
}

} // namespace asc
