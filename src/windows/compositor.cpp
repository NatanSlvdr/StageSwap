#include "compositor.hpp"

namespace asc::win {

VideoFrame Compositor::compose(const VideoFrame& camera, const VideoFrame& screen,
                               const double screen_mix, const std::int64_t timestamp_100ns) const {
    return blend_bgra(camera, screen, screen_mix, placeholder_color_, pipeline_size, timestamp_100ns);
}

} // namespace asc::win
