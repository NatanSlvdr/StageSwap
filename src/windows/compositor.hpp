#pragma once

#include "video_frame.hpp"

namespace asc::win {

class Compositor {
public:
    explicit Compositor(std::uint32_t placeholder_color) : placeholder_color_(placeholder_color | 0xff000000u) {}
    [[nodiscard]] VideoFrame compose(const VideoFrame& camera, const VideoFrame& screen,
                                     double screen_mix, std::int64_t timestamp_100ns) const;
    void set_placeholder_color(std::uint32_t color) { placeholder_color_ = color | 0xff000000u; }
private:
    std::uint32_t placeholder_color_;
};

} // namespace asc::win
