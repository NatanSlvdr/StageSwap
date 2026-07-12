#pragma once

#include "d3d_device.hpp"
#include "video_frame.hpp"
#include "asc/core/image.hpp"

#include <mutex>
#include <optional>
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
    void on_frame(winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool const& sender,
                  winrt::Windows::Foundation::IInspectable const&);
    D3DDevice& d3d_;
    mutable std::mutex mutex_;
    VideoFrame latest_;
    ComPtr<ID3D11Texture2D> capture_texture_;
    std::mutex comparison_mutex_;
    Size comparison_source_size_{};
    Size comparison_target_size_{};
    ComPtr<ID3D11VideoProcessorEnumerator> comparison_enumerator_;
    ComPtr<ID3D11VideoProcessor> comparison_processor_;
    ComPtr<ID3D11Texture2D> comparison_output_;
    ComPtr<ID3D11VideoProcessorOutputView> comparison_output_view_;
    ComPtr<ID3D11Texture2D> comparison_staging_;
    winrt::Windows::Graphics::Capture::GraphicsCaptureItem item_{nullptr};
    winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool pool_{nullptr};
    winrt::Windows::Graphics::Capture::GraphicsCaptureSession session_{nullptr};
    winrt::event_token frame_token_{};
};

} // namespace asc::win
