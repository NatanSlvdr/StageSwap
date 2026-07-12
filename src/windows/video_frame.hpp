#pragma once

#include "common.hpp"
#include "asc/core/types.hpp"

#include <d3d11.h>
#include <chrono>

namespace asc::win {

struct VideoFrame {
    ComPtr<ID3D11Texture2D> texture;
    asc::Size size;
    DXGI_FORMAT format{DXGI_FORMAT_UNKNOWN};
    std::chrono::steady_clock::time_point received_at{};
    std::int64_t presentation_time_100ns{0};
    [[nodiscard]] bool valid() const noexcept { return texture && size.width > 0 && size.height > 0; }
};

} // namespace asc::win

