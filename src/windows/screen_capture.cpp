#include "screen_capture.hpp"

#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <algorithm>

namespace asc::win {
namespace {
winrt::Windows::Graphics::DirectX::Direct3D11::IDirect3DDevice make_winrt_device(ID3D11Device* device) {
    ComPtr<IDXGIDevice> dxgi;
    check_hresult(device->QueryInterface(IID_PPV_ARGS(&dxgi)), "Query IDXGIDevice");
    winrt::Windows::Foundation::IInspectable inspectable{nullptr};
    check_hresult(CreateDirect3D11DeviceFromDXGIDevice(dxgi.Get(), reinterpret_cast<::IInspectable**>(winrt::put_abi(inspectable))),
                  "CreateDirect3D11DeviceFromDXGIDevice");
    return inspectable.as<winrt::Windows::Graphics::DirectX::Direct3D11::IDirect3DDevice>();
}

winrt::Windows::Graphics::Capture::GraphicsCaptureItem item_for_monitor(const HMONITOR monitor) {
    auto factory = winrt::get_activation_factory<winrt::Windows::Graphics::Capture::GraphicsCaptureItem, IGraphicsCaptureItemInterop>();
    winrt::Windows::Graphics::Capture::GraphicsCaptureItem item{nullptr};
    check_hresult(factory->CreateForMonitor(monitor, winrt::guid_of<winrt::Windows::Graphics::Capture::IGraphicsCaptureItem>(),
                                           winrt::put_abi(item)), "Create GraphicsCaptureItem for monitor");
    return item;
}
}

ScreenCapture::ScreenCapture(D3DDevice& d3d) : d3d_(d3d) {}
ScreenCapture::~ScreenCapture() { stop(); }

void ScreenCapture::start(const HMONITOR monitor, const bool include_cursor) {
    stop();
    if (!winrt::Windows::Graphics::Capture::GraphicsCaptureSession::IsSupported())
        throw std::runtime_error("Windows Graphics Capture is not supported on this system");
    try {
        item_ = item_for_monitor(monitor);
        const auto size = item_.Size();
        if (size.Width <= 0 || size.Height <= 0) throw std::runtime_error("capture monitor has invalid dimensions");
        pool_ = winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool::CreateFreeThreaded(
            make_winrt_device(d3d_.device()), winrt::Windows::Graphics::DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized, 3, size);
        session_ = pool_.CreateCaptureSession(item_);
        session_.IsCursorCaptureEnabled(include_cursor);
        session_.IsBorderRequired(false);
        const auto generation = ++generation_;
        running_.store(true, std::memory_order_release);
        frame_token_ = pool_.FrameArrived([this, generation](const auto& sender, const auto& args) {
            on_frame(sender, args, generation);
        });
        session_.StartCapture();
    } catch (...) {
        stop();
        throw;
    }
}

void ScreenCapture::stop() noexcept {
    running_.store(false, std::memory_order_release);
    ++generation_;
    try { if (pool_) pool_.FrameArrived(frame_token_); } catch (...) {}
    try { if (session_) session_.Close(); } catch (...) {}
    try { if (pool_) pool_.Close(); } catch (...) {}
    {
        std::scoped_lock callback_lock(callback_mutex_);
        session_ = nullptr;
        pool_ = nullptr;
        item_ = nullptr;
        frame_token_ = {};
        { std::scoped_lock lock(mutex_); latest_ = {}; capture_texture_.Reset(); }
        { std::scoped_lock comparison_lock(comparison_mutex_);
          comparison_enumerator_.Reset(); comparison_processor_.Reset(); comparison_output_.Reset();
          comparison_output_view_.Reset(); comparison_staging_.Reset(); comparison_source_size_ = {}; comparison_target_size_ = {}; }
    }
}

void ScreenCapture::on_frame(const winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool const& sender,
                             const winrt::Windows::Foundation::IInspectable const&, const std::uint64_t generation) {
    std::scoped_lock callback_lock(callback_mutex_);
    if (!running_.load(std::memory_order_acquire) || generation != generation_.load(std::memory_order_acquire)) return;
    try {
        const auto frame = sender.TryGetNextFrame();
        if (!frame) return;
        const auto content_size = frame.ContentSize();
        auto access = frame.Surface().as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
        std::scoped_lock lock(mutex_);
        ComPtr<ID3D11Texture2D> source_texture;
        check_hresult(access->GetInterface(IID_PPV_ARGS(&source_texture)), "Get capture texture");
        D3D11_TEXTURE2D_DESC source_desc{};
        source_texture->GetDesc(&source_desc);
        bool recreate = !capture_texture_;
        if (capture_texture_) {
            D3D11_TEXTURE2D_DESC current_desc{}; capture_texture_->GetDesc(&current_desc);
            recreate = current_desc.Width != source_desc.Width || current_desc.Height != source_desc.Height || current_desc.Format != source_desc.Format;
        }
        if (recreate) {
            source_desc.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;
            source_desc.MiscFlags = 0;
            source_desc.Usage = D3D11_USAGE_DEFAULT;
            source_desc.CPUAccessFlags = 0;
            check_hresult(d3d_.device()->CreateTexture2D(&source_desc, nullptr, &capture_texture_), "Create owned capture texture");
        }
        d3d_.context()->CopyResource(capture_texture_.Get(), source_texture.Get());
        VideoFrame received{capture_texture_, {static_cast<std::uint32_t>(content_size.Width), static_cast<std::uint32_t>(content_size.Height)},
                            source_desc.Format, std::chrono::steady_clock::now(), 0};
        latest_ = std::move(received);
    } catch (...) {
        std::scoped_lock lock(mutex_);
        latest_ = {};
    }
}

VideoFrame ScreenCapture::latest_frame() const { std::scoped_lock lock(mutex_); return latest_; }

std::optional<GrayImage> ScreenCapture::comparison_frame(const Size target) {
    std::scoped_lock comparison_lock(comparison_mutex_);
    const auto frame = latest_frame();
    if (!frame.valid()) return std::nullopt;
    D3D11_TEXTURE2D_DESC source_desc{};
    frame.texture->GetDesc(&source_desc);
    if (!comparison_processor_ || comparison_source_size_ != frame.size || comparison_target_size_ != target) {
        ComPtr<ID3D11VideoDevice> video_device;
        if (FAILED(d3d_.device()->QueryInterface(IID_PPV_ARGS(&video_device)))) return std::nullopt;
        D3D11_VIDEO_PROCESSOR_CONTENT_DESC content{};
        content.InputFrameFormat = D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE;
        content.InputFrameRate = {30, 1}; content.OutputFrameRate = {30, 1};
        content.InputWidth = source_desc.Width; content.InputHeight = source_desc.Height;
        content.OutputWidth = target.width; content.OutputHeight = target.height;
        content.Usage = D3D11_VIDEO_USAGE_PLAYBACK_NORMAL;
        if (FAILED(video_device->CreateVideoProcessorEnumerator(&content, &comparison_enumerator_)) ||
            FAILED(video_device->CreateVideoProcessor(comparison_enumerator_.Get(), 0, &comparison_processor_))) return std::nullopt;
        D3D11_TEXTURE2D_DESC output_desc{};
        output_desc.Width = target.width; output_desc.Height = target.height; output_desc.MipLevels = output_desc.ArraySize = 1;
        output_desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM; output_desc.SampleDesc.Count = 1;
        output_desc.Usage = D3D11_USAGE_DEFAULT; output_desc.BindFlags = D3D11_BIND_RENDER_TARGET;
        if (FAILED(d3d_.device()->CreateTexture2D(&output_desc, nullptr, &comparison_output_))) return std::nullopt;
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC output_view{};
        output_view.ViewDimension = D3D11_VPOV_DIMENSION_TEXTURE2D;
        if (FAILED(video_device->CreateVideoProcessorOutputView(comparison_output_.Get(), comparison_enumerator_.Get(),
                                                               &output_view, &comparison_output_view_))) return std::nullopt;
        output_desc.BindFlags = 0; output_desc.Usage = D3D11_USAGE_STAGING; output_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        if (FAILED(d3d_.device()->CreateTexture2D(&output_desc, nullptr, &comparison_staging_))) return std::nullopt;
        comparison_source_size_ = frame.size; comparison_target_size_ = target;
    }
    ComPtr<ID3D11VideoDevice> video_device;
    ComPtr<ID3D11VideoContext> video_context;
    if (FAILED(d3d_.device()->QueryInterface(IID_PPV_ARGS(&video_device))) ||
        FAILED(d3d_.context()->QueryInterface(IID_PPV_ARGS(&video_context)))) return std::nullopt;
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC input_desc{};
    input_desc.ViewDimension = D3D11_VPIV_DIMENSION_TEXTURE2D;
    ComPtr<ID3D11VideoProcessorInputView> input_view;
    if (FAILED(video_device->CreateVideoProcessorInputView(frame.texture.Get(), comparison_enumerator_.Get(), &input_desc, &input_view))) return std::nullopt;
    const RECT source_rect{0, 0, static_cast<LONG>(frame.size.width), static_cast<LONG>(frame.size.height)};
    const RECT destination_rect{0, 0, static_cast<LONG>(target.width), static_cast<LONG>(target.height)};
    video_context->VideoProcessorSetStreamFrameFormat(comparison_processor_.Get(), 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
    video_context->VideoProcessorSetStreamSourceRect(comparison_processor_.Get(), 0, TRUE, &source_rect);
    video_context->VideoProcessorSetStreamDestRect(comparison_processor_.Get(), 0, TRUE, &destination_rect);
    D3D11_VIDEO_PROCESSOR_STREAM stream{}; stream.Enable = TRUE; stream.pInputSurface = input_view.Get();
    if (FAILED(video_context->VideoProcessorBlt(comparison_processor_.Get(), comparison_output_view_.Get(), 0, 1, &stream))) return std::nullopt;
    d3d_.context()->CopyResource(comparison_staging_.Get(), comparison_output_.Get());
    D3D11_MAPPED_SUBRESOURCE mapped{};
    if (FAILED(d3d_.context()->Map(comparison_staging_.Get(), 0, D3D11_MAP_READ, 0, &mapped))) return std::nullopt;
    try {
        const auto bytes = std::span{static_cast<const std::uint8_t*>(mapped.pData), static_cast<std::size_t>(mapped.RowPitch) * target.height};
        auto gray = bgra_to_gray(bytes, target, mapped.RowPitch);
        d3d_.context()->Unmap(comparison_staging_.Get(), 0);
        return gray;
    } catch (...) {
        d3d_.context()->Unmap(comparison_staging_.Get(), 0);
        return std::nullopt;
    }
}

} // namespace asc::win
