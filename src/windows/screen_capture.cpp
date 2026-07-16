#include "screen_capture.hpp"

#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <cstring>

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
    item_ = item_for_monitor(monitor);
    const auto size = item_.Size();
    if (size.Width <= 0 || size.Height <= 0) throw std::runtime_error("capture monitor has invalid dimensions");
    pool_ = winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool::CreateFreeThreaded(
        make_winrt_device(d3d_.device()), winrt::Windows::Graphics::DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized, 3, size);
    session_ = pool_.CreateCaptureSession(item_);
    session_.IsCursorCaptureEnabled(include_cursor);
    session_.IsBorderRequired(false);
    const auto generation = ++generation_;
    running_ = true;
    frame_token_ = pool_.FrameArrived([this, generation](const auto& sender, const auto& args) { on_frame(sender, args, generation); });
    session_.StartCapture();
}

void ScreenCapture::stop() noexcept {
    running_ = false;
    ++generation_;
    try { if (pool_) pool_.FrameArrived(frame_token_); } catch (...) {}
    try { if (session_) session_.Close(); } catch (...) {}
    try { if (pool_) pool_.Close(); } catch (...) {}
    std::scoped_lock callback_lock(callback_mutex_);
    session_ = nullptr; pool_ = nullptr; item_ = nullptr; frame_token_ = {};
    std::scoped_lock lock(mutex_);
    latest_ = {};
    readback_texture_.Reset();
}

void ScreenCapture::on_frame(winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool sender,
                             winrt::Windows::Foundation::IInspectable, const std::uint64_t generation) {
    std::scoped_lock callback_lock(callback_mutex_);
    if (!running_ || generation != generation_) return;
    try {
        const auto captured = sender.TryGetNextFrame();
        if (!captured) return;
        auto access = captured.Surface().as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
        ComPtr<ID3D11Texture2D> source;
        check_hresult(access->GetInterface(IID_PPV_ARGS(&source)), "Get capture texture");
        D3D11_TEXTURE2D_DESC description{};
        source->GetDesc(&description);
        bool recreate = !readback_texture_;
        if (readback_texture_) {
            D3D11_TEXTURE2D_DESC current{}; readback_texture_->GetDesc(&current);
            recreate = current.Width != description.Width || current.Height != description.Height || current.Format != description.Format;
        }
        if (recreate) {
            description.BindFlags = 0; description.MiscFlags = 0;
            description.Usage = D3D11_USAGE_STAGING; description.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            check_hresult(d3d_.device()->CreateTexture2D(&description, nullptr, &readback_texture_), "Create screen readback texture");
        }
        d3d_.context()->CopyResource(readback_texture_.Get(), source.Get());
        D3D11_MAPPED_SUBRESOURCE mapped{};
        check_hresult(d3d_.context()->Map(readback_texture_.Get(), 0, D3D11_MAP_READ, 0, &mapped), "Map captured screen texture");
        const auto content = captured.ContentSize();
        VideoFrame frame;
        frame.size = {static_cast<std::uint32_t>(content.Width), static_cast<std::uint32_t>(content.Height)};
        frame.stride = frame.size.width * 4;
        frame.received_at = Clock::now();
        frame.sequence = ++sequence_;
        frame.bgra.resize(static_cast<std::size_t>(frame.stride) * frame.size.height);
        for (std::uint32_t y = 0; y < frame.size.height; ++y)
            std::memcpy(frame.bgra.data() + static_cast<std::size_t>(y) * frame.stride,
                        static_cast<const std::uint8_t*>(mapped.pData) + static_cast<std::size_t>(y) * mapped.RowPitch,
                        frame.stride);
        d3d_.context()->Unmap(readback_texture_.Get(), 0);
        std::scoped_lock lock(mutex_);
        latest_ = std::move(frame);
    } catch (...) {
        std::scoped_lock lock(mutex_);
        latest_ = {};
    }
}

VideoFrame ScreenCapture::latest_frame() const { std::scoped_lock lock(mutex_); return latest_; }
std::optional<GrayImage> ScreenCapture::comparison_frame(const Size target) {
    const auto frame = latest_frame();
    if (!frame.valid()) return std::nullopt;
    return resize_bilinear(bgra_to_gray(frame.bgra, frame.size, frame.stride), target);
}

} // namespace asc::win
