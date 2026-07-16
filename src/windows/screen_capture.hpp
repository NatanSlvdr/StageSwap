#pragma once

#include "d3d_device.hpp"
#include "video_frame.hpp"
#include "asc/core/image.hpp"

#include <atomic>
#include <mutex>
#include <optional>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>

namespace asc::win {

class ScreenCapture {
public:
    explicit ScreenCapture(D3DDevice& d3d);
    ~ScreenCapture();
    ScreenCapture(const ScreenCapture&) = delete;
    ScreenCapture& operator=(const ScreenCapture&) = delete;
    void start(HMONITOR monitor, bool include_cursor);
    void stop() noexcept;
    [[nodiscard]] VideoFrame latest_frame() const;
    [[nodiscard]] std::optional<GrayImage> comparison_frame(Size target = {160, 90});
private:
    void on_frame(winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool sender,
                  winrt::Windows::Foundation::IInspectable, std::uint64_t generation);
    D3DDevice& d3d_;
    std::atomic<bool> running_{false};
    std::atomic<std::uint64_t> generation_{0};
    std::mutex callback_mutex_;
    mutable std::mutex mutex_;
    VideoFrame latest_;
    ComPtr<ID3D11Texture2D> readback_texture_;
    std::uint64_t sequence_{0};
    winrt::Windows::Graphics::Capture::GraphicsCaptureItem item_{nullptr};
    winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool pool_{nullptr};
    winrt::Windows::Graphics::Capture::GraphicsCaptureSession session_{nullptr};
    winrt::event_token frame_token_{};
};

} // namespace asc::win
