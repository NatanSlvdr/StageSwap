#pragma once

#include "common.hpp"
#include "shared_frame.hpp"

#include <mfidl.h>
#include <wrl/implements.h>
#include <condition_variable>
#include <mutex>
#include <vector>

namespace asc::win::source {

class MediaStream final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>, IMFMediaStream2> {
public:
    HRESULT initialize(IMFMediaSource* parent, DWORD id, const std::wstring& pipe_name, std::uint32_t placeholder_color);
    HRESULT start(IMFMediaType* media_type);
    HRESULT stop(bool send_event);
    HRESULT shutdown();
    [[nodiscard]] DWORD id() const noexcept { return id_; }
    HRESULT attributes(IMFAttributes** output) const;

    STDMETHODIMP BeginGetEvent(IMFAsyncCallback* callback, IUnknown* state) override;
    STDMETHODIMP EndGetEvent(IMFAsyncResult* result, IMFMediaEvent** event) override;
    STDMETHODIMP GetEvent(DWORD flags, IMFMediaEvent** event) override;
    STDMETHODIMP QueueEvent(MediaEventType type, REFGUID extended, HRESULT status, const PROPVARIANT* value) override;
    STDMETHODIMP GetMediaSource(IMFMediaSource** source) override;
    STDMETHODIMP GetStreamDescriptor(IMFStreamDescriptor** descriptor) override;
    STDMETHODIMP RequestSample(IUnknown* token) override;
    STDMETHODIMP SetStreamState(MF_STREAM_STATE state) override;
    STDMETHODIMP GetStreamState(MF_STREAM_STATE* state) override;

private:
    struct AxisSample { std::uint32_t first; std::uint32_t second; std::uint32_t weight; };
    [[nodiscard]] HRESULT make_sample(IUnknown* token, IMFSample** sample);
    void prepare_scaling_map(Size input_size, Size output_size);
    void scale_bgra(const CpuSharedFrame& input, std::uint8_t* output, Size output_size, std::uint32_t output_stride);
    mutable std::mutex mutex_;
    std::condition_variable state_changed_;
    ComPtr<IMFMediaSource> parent_;
    ComPtr<IMFMediaEventQueue> events_;
    ComPtr<IMFStreamDescriptor> descriptor_;
    ComPtr<IMFAttributes> attributes_;
    ComPtr<IMFMediaType> current_type_;
    SharedFrameReader reader_;
    CpuSharedFrame shared_frame_cache_;
    std::vector<std::uint8_t> scaled_bgra_;
    std::vector<AxisSample> x_samples_;
    std::vector<AxisSample> y_samples_;
    Size scaling_input_size_{};
    Size scaling_output_size_{};
    DWORD id_{0};
    MF_STREAM_STATE state_{MF_STREAM_STATE_STOPPED};
    bool shutdown_{false};
    LONGLONG next_time_{0};
    LONGLONG frame_duration_{333333};
    LARGE_INTEGER qpc_frequency_{};
    LARGE_INTEGER next_qpc_{};
    LONGLONG qpc_step_{0};
    LONGLONG qpc_remainder_step_{0};
    LONGLONG qpc_remainder_{0};
    std::uint32_t placeholder_color_{0xff171719u};
};

} // namespace asc::win::source
