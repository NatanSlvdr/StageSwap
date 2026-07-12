#include "compositor.hpp"

#include <d3dcompiler.h>
#include <algorithm>
#include <array>
#include <cstring>
#include <stdexcept>

namespace asc::win {
namespace {
constexpr char shader_source[] = R"(
cbuffer BlendConstants : register(b0) {
    float4 cameraRect; float4 cameraUv;
    float4 previousScreenRect; float4 previousScreenUv;
    float4 screenRect; float4 screenUv;
    float screenSwitchMix; float screenMix; float2 padding;
};
Texture2D cameraTexture : register(t0);
Texture2D previousScreenTexture : register(t1);
Texture2D screenTexture : register(t2);
SamplerState linearSampler : register(s0);
struct VertexOut { float4 position : SV_POSITION; float2 uv : TEXCOORD0; };
VertexOut vertexMain(uint id : SV_VertexID) {
    VertexOut output;
    float2 positions[3] = { float2(-1,-1), float2(-1,3), float2(3,-1) };
    float2 uvs[3] = { float2(0,1), float2(0,-1), float2(2,1) };
    output.position = float4(positions[id], 0, 1); output.uv = uvs[id]; return output;
}
float4 scaledSample(Texture2D textureValue, float2 outputUv, float4 rect, float4 uvTransform) {
    if (outputUv.x < rect.x || outputUv.y < rect.y || outputUv.x > rect.x + rect.z || outputUv.y > rect.y + rect.w)
        return float4(0.035, 0.035, 0.04, 1);
    float2 local = (outputUv - rect.xy) / rect.zw;
    float2 sourceUv = uvTransform.xy + local * uvTransform.zw;
    return textureValue.Sample(linearSampler, sourceUv);
}
float4 pixelMain(VertexOut input) : SV_TARGET {
    float4 camera = scaledSample(cameraTexture, input.uv, cameraRect, cameraUv);
    float4 previousScreen = scaledSample(previousScreenTexture, input.uv, previousScreenRect, previousScreenUv);
    float4 currentScreen = scaledSample(screenTexture, input.uv, screenRect, screenUv);
    float4 screen = lerp(previousScreen, currentScreen, saturate(screenSwitchMix));
    return lerp(camera, screen, saturate(screenMix));
})";

ComPtr<ID3DBlob> compile(const char* entry, const char* target) {
    ComPtr<ID3DBlob> bytecode;
    ComPtr<ID3DBlob> errors;
    const auto result = D3DCompile(shader_source, sizeof(shader_source), "asc-compositor", nullptr, nullptr, entry, target,
                                  D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &bytecode, &errors);
    if (FAILED(result)) throw HResultError(result, errors ? static_cast<const char*>(errors->GetBufferPointer()) : "D3DCompile");
    return bytecode;
}
}

Compositor::Compositor(D3DDevice& d3d, const Size output_size, const std::uint32_t placeholder_color)
    : d3d_(d3d), output_size_(output_size), placeholder_color_(placeholder_color | 0xff000000u) {
    if (output_size_.width == 0 || output_size_.height == 0) throw std::invalid_argument("compositor output size must be non-zero");
    create_resources();
}

void Compositor::create_resources() {
    const auto vs = compile("vertexMain", "vs_5_0");
    const auto ps = compile("pixelMain", "ps_5_0");
    check_hresult(d3d_.device()->CreateVertexShader(vs->GetBufferPointer(), vs->GetBufferSize(), nullptr, &vertex_shader_), "Create compositor vertex shader");
    check_hresult(d3d_.device()->CreatePixelShader(ps->GetBufferPointer(), ps->GetBufferSize(), nullptr, &pixel_shader_), "Create compositor pixel shader");
    D3D11_SAMPLER_DESC sampler{};
    sampler.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
    sampler.AddressU = sampler.AddressV = sampler.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
    sampler.MaxLOD = D3D11_FLOAT32_MAX;
    check_hresult(d3d_.device()->CreateSamplerState(&sampler, &sampler_), "Create compositor sampler");
    D3D11_BUFFER_DESC constant_desc{sizeof(ShaderConstants), D3D11_USAGE_DYNAMIC, D3D11_BIND_CONSTANT_BUFFER, D3D11_CPU_ACCESS_WRITE, 0, 0};
    check_hresult(d3d_.device()->CreateBuffer(&constant_desc, nullptr, &constants_), "Create compositor constants");
    D3D11_TEXTURE2D_DESC output_desc{};
    output_desc.Width = output_size_.width; output_desc.Height = output_size_.height;
    output_desc.MipLevels = output_desc.ArraySize = 1; output_desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    output_desc.SampleDesc.Count = 1; output_desc.Usage = D3D11_USAGE_DEFAULT;
    output_desc.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
    check_hresult(d3d_.device()->CreateTexture2D(&output_desc, nullptr, &output_), "Create compositor output");
    check_hresult(d3d_.device()->CreateRenderTargetView(output_.Get(), nullptr, &output_view_), "Create compositor output view");
    const std::vector<std::uint32_t> placeholder_pixels(static_cast<std::size_t>(output_size_.width) * output_size_.height, placeholder_color_);
    D3D11_SUBRESOURCE_DATA placeholder_data{placeholder_pixels.data(), output_size_.width * 4, 0};
    output_desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
    check_hresult(d3d_.device()->CreateTexture2D(&output_desc, &placeholder_data, &placeholder_), "Create safe placeholder");
}

std::pair<std::array<float, 4>, std::array<float, 4>> Compositor::transform(const Size source, const Size output, const ScalingMode mode) {
    std::array<float, 4> rect{0, 0, 1, 1};
    std::array<float, 4> uv{0, 0, 1, 1};
    if (source.width == 0 || source.height == 0 || mode == ScalingMode::stretch) return {rect, uv};
    const float source_aspect = static_cast<float>(source.width) / source.height;
    const float output_aspect = static_cast<float>(output.width) / output.height;
    if (mode == ScalingMode::fit) {
        if (source_aspect > output_aspect) { rect[3] = output_aspect / source_aspect; rect[1] = (1.0f - rect[3]) * 0.5f; }
        else { rect[2] = source_aspect / output_aspect; rect[0] = (1.0f - rect[2]) * 0.5f; }
    } else {
        if (source_aspect > output_aspect) { uv[2] = output_aspect / source_aspect; uv[0] = (1.0f - uv[2]) * 0.5f; }
        else { uv[3] = source_aspect / output_aspect; uv[1] = (1.0f - uv[3]) * 0.5f; }
    }
    return {rect, uv};
}

Compositor::SourceView Compositor::sampled(const VideoFrame& frame, const std::size_t cache_index) {
    auto& cached = source_cache_[cache_index];
    ComPtr<ID3D11Texture2D> source = frame.valid() ? frame.texture : placeholder_;
    if (cached.source.Get() == source.Get() && cached.sampled.view) {
        if (cached.copied) d3d_.context()->CopyResource(cached.sampled.texture.Get(), source.Get());
        return cached.sampled;
    }
    cached = {};
    cached.source = source;
    cached.sampled.texture = source;
    if (FAILED(d3d_.device()->CreateShaderResourceView(source.Get(), nullptr, &cached.sampled.view))) {
        D3D11_TEXTURE2D_DESC desc{};
        source->GetDesc(&desc);
        if (desc.ArraySize != 1 || desc.MipLevels != 1 || desc.SampleDesc.Count != 1)
            throw std::runtime_error("compositor source is not a supported 2D texture");
        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE; desc.MiscFlags = 0; desc.Usage = D3D11_USAGE_DEFAULT; desc.CPUAccessFlags = 0;
        cached.sampled.texture.Reset();
        check_hresult(d3d_.device()->CreateTexture2D(&desc, nullptr, &cached.sampled.texture), "Create sampleable frame copy");
        d3d_.context()->CopyResource(cached.sampled.texture.Get(), source.Get());
        check_hresult(d3d_.device()->CreateShaderResourceView(cached.sampled.texture.Get(), nullptr, &cached.sampled.view), "Create frame shader view");
        cached.copied = true;
    }
    return cached.sampled;
}

VideoFrame Compositor::compose(const VideoFrame& camera, const VideoFrame& screen, const VideoFrame& previous_screen,
                               const double screen_switch_mix, const double screen_mix,
                               const ScalingMode camera_scaling, const ScalingMode screen_scaling, const std::int64_t timestamp_100ns) {
    const auto camera_view = sampled(camera, 0);
    const auto previous_screen_view = sampled(previous_screen.valid() ? previous_screen : screen, 1);
    const auto screen_view = sampled(screen, 2);
    const auto [camera_rect, camera_uv] = transform(camera.valid() ? camera.size : output_size_, output_size_, camera_scaling);
    const auto [previous_screen_rect, previous_screen_uv] = transform(previous_screen.valid() ? previous_screen.size :
                                                                      screen.valid() ? screen.size : output_size_, output_size_, screen_scaling);
    const auto [screen_rect, screen_uv] = transform(screen.valid() ? screen.size : output_size_, output_size_, screen_scaling);
    ShaderConstants values{};
    std::memcpy(values.camera_rect, camera_rect.data(), sizeof(values.camera_rect));
    std::memcpy(values.camera_uv, camera_uv.data(), sizeof(values.camera_uv));
    std::memcpy(values.previous_screen_rect, previous_screen_rect.data(), sizeof(values.previous_screen_rect));
    std::memcpy(values.previous_screen_uv, previous_screen_uv.data(), sizeof(values.previous_screen_uv));
    std::memcpy(values.screen_rect, screen_rect.data(), sizeof(values.screen_rect));
    std::memcpy(values.screen_uv, screen_uv.data(), sizeof(values.screen_uv));
    values.screen_switch_mix = static_cast<float>(std::clamp(screen_switch_mix, 0.0, 1.0));
    values.screen_mix = static_cast<float>(std::clamp(screen_mix, 0.0, 1.0));
    D3D11_MAPPED_SUBRESOURCE mapped{};
    check_hresult(d3d_.context()->Map(constants_.Get(), 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped), "Map compositor constants");
    std::memcpy(mapped.pData, &values, sizeof(values));
    d3d_.context()->Unmap(constants_.Get(), 0);
    const D3D11_VIEWPORT viewport{0, 0, static_cast<float>(output_size_.width), static_cast<float>(output_size_.height), 0, 1};
    ID3D11ShaderResourceView* views[]{camera_view.view.Get(), previous_screen_view.view.Get(), screen_view.view.Get()};
    ID3D11SamplerState* samplers[]{sampler_.Get()};
    ID3D11Buffer* constants[]{constants_.Get()};
    d3d_.context()->OMSetRenderTargets(1, output_view_.GetAddressOf(), nullptr);
    d3d_.context()->RSSetViewports(1, &viewport);
    d3d_.context()->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    d3d_.context()->VSSetShader(vertex_shader_.Get(), nullptr, 0);
    d3d_.context()->PSSetShader(pixel_shader_.Get(), nullptr, 0);
    d3d_.context()->PSSetShaderResources(0, 3, views);
    d3d_.context()->PSSetSamplers(0, 1, samplers);
    d3d_.context()->PSSetConstantBuffers(0, 1, constants);
    d3d_.context()->Draw(3, 0);
    ID3D11ShaderResourceView* null_views[]{nullptr, nullptr, nullptr};
    d3d_.context()->PSSetShaderResources(0, 3, null_views);
    return {output_, output_size_, DXGI_FORMAT_B8G8R8A8_UNORM, std::chrono::steady_clock::now(), timestamp_100ns};
}

void Compositor::reset() {
    source_cache_ = {};
    vertex_shader_.Reset(); pixel_shader_.Reset(); sampler_.Reset(); constants_.Reset(); output_.Reset(); output_view_.Reset(); placeholder_.Reset();
    create_resources();
}
void Compositor::reconfigure(const Size output_size) {
    if (output_size.width == 0 || output_size.height == 0) throw std::invalid_argument("compositor output size must be non-zero");
    output_size_ = output_size;
    reset();
}

} // namespace asc::win
