#pragma once

#include "d3d_device.hpp"
#include "video_frame.hpp"
#include "asc/core/types.hpp"

#include <d3d11.h>
#include <array>
#include <utility>

namespace asc::win {

class Compositor {
public:
    Compositor(D3DDevice& d3d, Size output_size, std::uint32_t placeholder_color);
    [[nodiscard]] VideoFrame compose(const VideoFrame& camera, const VideoFrame& screen, const VideoFrame& previous_screen,
                                     double screen_switch_mix, double screen_mix,
                                     ScalingMode camera_scaling, ScalingMode screen_scaling, std::int64_t timestamp_100ns);
    void reset();
    void reconfigure(Size output_size);
    void set_placeholder_color(std::uint32_t color) { placeholder_color_ = color | 0xff000000u; reset(); }

private:
    struct SourceView { ComPtr<ID3D11Texture2D> texture; ComPtr<ID3D11ShaderResourceView> view; };
    struct alignas(16) ShaderConstants {
        float camera_rect[4];
        float camera_uv[4];
        float previous_screen_rect[4];
        float previous_screen_uv[4];
        float screen_rect[4];
        float screen_uv[4];
        float screen_switch_mix;
        float screen_mix;
        float padding[2];
    };
    void create_resources();
    [[nodiscard]] SourceView sampled(const VideoFrame& frame);
    [[nodiscard]] static std::pair<std::array<float, 4>, std::array<float, 4>> transform(Size source, Size output, ScalingMode mode);
    D3DDevice& d3d_;
    Size output_size_;
    std::uint32_t placeholder_color_;
    ComPtr<ID3D11VertexShader> vertex_shader_;
    ComPtr<ID3D11PixelShader> pixel_shader_;
    ComPtr<ID3D11SamplerState> sampler_;
    ComPtr<ID3D11Buffer> constants_;
    ComPtr<ID3D11Texture2D> output_;
    ComPtr<ID3D11RenderTargetView> output_view_;
    ComPtr<ID3D11Texture2D> placeholder_;
};

} // namespace asc::win
