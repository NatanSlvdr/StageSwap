#include "asc/core/image.hpp"

#include <algorithm>
#include <cmath>
#include <stdexcept>

namespace asc {

bool GrayImage::valid() const noexcept {
    return size.width > 0 && size.height > 0 &&
           pixels.size() == static_cast<std::size_t>(size.width) * size.height;
}

std::uint8_t GrayImage::at(const std::uint32_t x, const std::uint32_t y) const {
    return pixels.at(static_cast<std::size_t>(y) * size.width + x);
}

GrayImage bgra_to_gray(const std::span<const std::uint8_t> bgra, const Size size, const std::size_t row_pitch) {
    if (size.width == 0 || size.height == 0 || row_pitch < static_cast<std::size_t>(size.width) * 4 ||
        bgra.size() < row_pitch * size.height) {
        throw std::invalid_argument("invalid BGRA image layout");
    }
    GrayImage result{size, std::vector<std::uint8_t>(static_cast<std::size_t>(size.width) * size.height)};
    for (std::uint32_t y = 0; y < size.height; ++y) {
        for (std::uint32_t x = 0; x < size.width; ++x) {
            const auto offset = static_cast<std::size_t>(y) * row_pitch + static_cast<std::size_t>(x) * 4;
            const auto b = static_cast<double>(bgra[offset]);
            const auto g = static_cast<double>(bgra[offset + 1]);
            const auto r = static_cast<double>(bgra[offset + 2]);
            result.pixels[static_cast<std::size_t>(y) * size.width + x] =
                static_cast<std::uint8_t>(std::clamp(std::lround(0.0722 * b + 0.7152 * g + 0.2126 * r), 0L, 255L));
        }
    }
    return result;
}

GrayImage resize_bilinear(const GrayImage& source, const Size target) {
    if (!source.valid() || target.width == 0 || target.height == 0) {
        throw std::invalid_argument("invalid resize dimensions");
    }
    GrayImage output{target, std::vector<std::uint8_t>(static_cast<std::size_t>(target.width) * target.height)};
    const double sx = static_cast<double>(source.size.width) / target.width;
    const double sy = static_cast<double>(source.size.height) / target.height;
    for (std::uint32_t y = 0; y < target.height; ++y) {
        const double source_y = (static_cast<double>(y) + 0.5) * sy - 0.5;
        const auto y0 = static_cast<std::uint32_t>(std::clamp(std::floor(source_y), 0.0, static_cast<double>(source.size.height - 1)));
        const auto y1 = std::min(y0 + 1, source.size.height - 1);
        const double fy = std::clamp(source_y - std::floor(source_y), 0.0, 1.0);
        for (std::uint32_t x = 0; x < target.width; ++x) {
            const double source_x = (static_cast<double>(x) + 0.5) * sx - 0.5;
            const auto x0 = static_cast<std::uint32_t>(std::clamp(std::floor(source_x), 0.0, static_cast<double>(source.size.width - 1)));
            const auto x1 = std::min(x0 + 1, source.size.width - 1);
            const double fx = std::clamp(source_x - std::floor(source_x), 0.0, 1.0);
            const double top = source.at(x0, y0) * (1.0 - fx) + source.at(x1, y0) * fx;
            const double bottom = source.at(x0, y1) * (1.0 - fx) + source.at(x1, y1) * fx;
            output.pixels[static_cast<std::size_t>(y) * target.width + x] =
                static_cast<std::uint8_t>(std::clamp(std::lround(top * (1.0 - fy) + bottom * fy), 0L, 255L));
        }
    }
    return output;
}

double image_similarity(const GrayImage& reference, const GrayImage& candidate) {
    if (!reference.valid() || !candidate.valid() || reference.size != candidate.size) {
        return 0.0;
    }
    const auto border_x = reference.size.width >= 80 ? reference.size.width / 80 : 0;
    const auto border_y = reference.size.height >= 45 ? reference.size.height / 45 : 0;
    const auto x_end = reference.size.width - border_x;
    const auto y_end = reference.size.height - border_y;
    double sum_a = 0.0;
    double sum_b = 0.0;
    double absolute_error = 0.0;
    std::size_t count = 0;
    for (std::uint32_t y = border_y; y < y_end; ++y) {
        for (std::uint32_t x = border_x; x < x_end; ++x) {
            const auto a = static_cast<double>(reference.at(x, y));
            const auto b = static_cast<double>(candidate.at(x, y));
            sum_a += a;
            sum_b += b;
            absolute_error += std::abs(a - b);
            ++count;
        }
    }
    if (count == 0) return 0.0;
    const double mean_a = sum_a / static_cast<double>(count);
    const double mean_b = sum_b / static_cast<double>(count);
    double variance_a = 0.0;
    double variance_b = 0.0;
    double covariance = 0.0;
    for (std::uint32_t y = border_y; y < y_end; ++y) {
        for (std::uint32_t x = border_x; x < x_end; ++x) {
            const double da = static_cast<double>(reference.at(x, y)) - mean_a;
            const double db = static_cast<double>(candidate.at(x, y)) - mean_b;
            variance_a += da * da;
            variance_b += db * db;
            covariance += da * db;
        }
    }
    const double divisor = static_cast<double>(count);
    variance_a /= divisor;
    variance_b /= divisor;
    covariance /= divisor;
    constexpr double c1 = 6.5025;  // (0.01 * 255)^2
    constexpr double c2 = 58.5225; // (0.03 * 255)^2
    const double ssim = ((2.0 * mean_a * mean_b + c1) * (2.0 * covariance + c2)) /
                        ((mean_a * mean_a + mean_b * mean_b + c1) * (variance_a + variance_b + c2));
    const double pixel_score = 1.0 - absolute_error / (divisor * 255.0);
    return std::clamp(0.8 * std::clamp(ssim, 0.0, 1.0) + 0.2 * pixel_score, 0.0, 1.0);
}

} // namespace asc

