#pragma once

#include "asc/core/types.hpp"

#include <cstddef>
#include <cstdint>
#include <span>
#include <vector>

namespace asc {

struct GrayImage {
    Size size;
    std::vector<std::uint8_t> pixels;

    [[nodiscard]] bool valid() const noexcept;
    [[nodiscard]] std::uint8_t at(std::uint32_t x, std::uint32_t y) const;
};

[[nodiscard]] GrayImage bgra_to_gray(std::span<const std::uint8_t> bgra, Size size, std::size_t row_pitch);
[[nodiscard]] GrayImage resize_bilinear(const GrayImage& source, Size target);

// Returns [0, 1]. It combines global structural similarity with normalized
// pixel error, ignoring a narrow outer border where scaling artifacts cluster.
[[nodiscard]] double image_similarity(const GrayImage& reference, const GrayImage& candidate);

} // namespace asc

