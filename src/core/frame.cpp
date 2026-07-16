#include "asc/core/frame.hpp"

#include <algorithm>
#include <cmath>
#include <limits>

namespace asc {
namespace {

bool storage_size(const Size size, const std::uint32_t stride, std::size_t& bytes) noexcept {
    if (size.width == 0 || size.height == 0 || stride < size.width * 4u) return false;
    if (size.height > std::numeric_limits<std::size_t>::max() / stride) return false;
    bytes = static_cast<std::size_t>(stride) * size.height;
    return true;
}

} // namespace

bool Frame::valid() const noexcept {
    std::size_t required = 0;
    return storage_size(size, stride, required) && bgra.size() >= required;
}

bool Frame::fresh(const TimePoint now, const std::chrono::milliseconds maximum_age) const noexcept {
    return valid() && freshness != FrameFreshness::stale && received_at != TimePoint{} &&
           now >= received_at && now - received_at <= maximum_age;
}

Frame make_placeholder(const Size size, const std::uint32_t color_bgra, const std::int64_t timestamp_100ns) {
    Frame output;
    output.size = size;
    output.stride = size.width * 4u;
    output.received_at = Clock::now();
    output.presentation_time_100ns = timestamp_100ns;
    output.freshness = FrameFreshness::retained;
    output.bgra.resize(static_cast<std::size_t>(output.stride) * size.height);
    const auto color = color_bgra | 0xff000000u;
    for (std::size_t offset = 0; offset < output.bgra.size(); offset += 4) {
        output.bgra[offset] = static_cast<std::uint8_t>(color);
        output.bgra[offset + 1] = static_cast<std::uint8_t>(color >> 8);
        output.bgra[offset + 2] = static_cast<std::uint8_t>(color >> 16);
        output.bgra[offset + 3] = 0xff;
    }
    return output;
}

Frame aspect_fit_bgra(const Frame& source, const Size output_size) {
    auto output = make_placeholder(output_size, 0xff000000u, source.presentation_time_100ns);
    if (!source.valid()) return output;
    output.received_at = source.received_at;
    output.sequence = source.sequence;
    output.freshness = source.freshness;

    const double scale = std::min(static_cast<double>(output_size.width) / source.size.width,
                                  static_cast<double>(output_size.height) / source.size.height);
    const auto fitted_width = std::max(1u, static_cast<std::uint32_t>(std::floor(source.size.width * scale)));
    const auto fitted_height = std::max(1u, static_cast<std::uint32_t>(std::floor(source.size.height * scale)));
    const auto left = (output_size.width - fitted_width) / 2u;
    const auto top = (output_size.height - fitted_height) / 2u;
    for (std::uint32_t y = 0; y < fitted_height; ++y) {
        const auto source_y = std::min(source.size.height - 1u,
            static_cast<std::uint32_t>((static_cast<std::uint64_t>(y) * source.size.height) / fitted_height));
        auto* destination = output.bgra.data() + static_cast<std::size_t>(top + y) * output.stride + left * 4u;
        const auto* input = source.bgra.data() + static_cast<std::size_t>(source_y) * source.stride;
        for (std::uint32_t x = 0; x < fitted_width; ++x) {
            const auto source_x = std::min(source.size.width - 1u,
                static_cast<std::uint32_t>((static_cast<std::uint64_t>(x) * source.size.width) / fitted_width));
            std::copy_n(input + source_x * 4u, 4, destination + x * 4u);
        }
    }
    return output;
}

Frame blend_bgra(const Frame& camera, const Frame& screen, const double screen_mix,
                 const std::uint32_t placeholder_color_bgra, const Size output_size,
                 const std::int64_t timestamp_100ns) {
    auto fitted_camera = camera.valid() ? aspect_fit_bgra(camera, output_size) : make_placeholder(output_size, placeholder_color_bgra);
    auto fitted_screen = screen.valid() ? aspect_fit_bgra(screen, output_size) : fitted_camera;
    const auto mix = std::clamp(screen_mix, 0.0, 1.0);
    Frame output = fitted_camera;
    output.presentation_time_100ns = timestamp_100ns;
    output.received_at = Clock::now();
    output.freshness = FrameFreshness::live;
    for (std::size_t index = 0; index < output.bgra.size(); ++index) {
        output.bgra[index] = static_cast<std::uint8_t>(std::lround(
            fitted_camera.bgra[index] * (1.0 - mix) + fitted_screen.bgra[index] * mix));
    }
    return output;
}

} // namespace asc
