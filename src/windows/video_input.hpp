#pragma once

#include "d3d_device.hpp"
#include "video_frame.hpp"

#include <mfreadwrite.h>
#include <atomic>
#include <condition_variable>
#include <mutex>
#include <string>

namespace asc::win {

class VideoInputCallback;

class VideoInput final : public IMFSourceReaderCallback {
public:
    explicit VideoInput(D3DDevice& d3d);
    ~VideoInput();
    void start(const std::string& symbolic_link, Size preferred_size, std::uint32_t preferred_fps);
    void stop() noexcept;
    void restart();
    [[nodiscard]] VideoFrame latest_frame() const;
    [[nodiscard]] HRESULT last_error() const noexcept { return last_error_.load(); }

    STDMETHODIMP QueryInterface(REFIID riid, void** object) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;
    STDMETHODIMP OnReadSample(HRESULT status, DWORD stream_index, DWORD stream_flags, LONGLONG timestamp, IMFSample* sample) override;
    STDMETHODIMP OnFlush(DWORD stream_index) override;
    STDMETHODIMP OnEvent(DWORD stream_index, IMFMediaEvent* event) override;

private:
    void request_next();
    void refresh_output_format(IMFSourceReader* reader) noexcept;
    D3DDevice& d3d_;
    std::atomic<ULONG> references_{1};
    std::atomic<bool> running_{false};
    std::atomic<HRESULT> last_error_{S_OK};
    // Serializes resource-using callbacks with stop(), which prevents a callback
    // from touching textures while the application rebuilds the D3D device.
    mutable std::mutex callback_mutex_;
    std::mutex flush_mutex_;
    std::condition_variable flush_condition_;
    bool flush_completed_{false};
    mutable std::mutex mutex_;
    ComPtr<IMFSourceReader> reader_;
    ComPtr<IMFSourceReaderCallback> callback_;
    VideoInputCallback* callback_impl_{nullptr};
    VideoFrame latest_;
    ComPtr<ID3D11Texture2D> upload_texture_;
    std::string symbolic_link_;
    Size preferred_size_{};
    std::uint32_t preferred_fps_{30};
    Size active_size_{};
    LONG active_stride_{0};
};

} // namespace asc::win
