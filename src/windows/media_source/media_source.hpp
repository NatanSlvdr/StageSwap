#pragma once

#include "common.hpp"
#include "media_stream.hpp"

#include <mfidl.h>
#include <ks.h>
#include <wrl/implements.h>
#include <mutex>

namespace asc::win::source {

class MediaSource final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>, IMFMediaSourceEx, IMFGetService, IKsControl> {
public:
    HRESULT initialize(IMFAttributes* activation_attributes);

    STDMETHODIMP BeginGetEvent(IMFAsyncCallback* callback, IUnknown* state) override;
    STDMETHODIMP EndGetEvent(IMFAsyncResult* result, IMFMediaEvent** event) override;
    STDMETHODIMP GetEvent(DWORD flags, IMFMediaEvent** event) override;
    STDMETHODIMP QueueEvent(MediaEventType type, REFGUID extended, HRESULT status, const PROPVARIANT* value) override;
    STDMETHODIMP CreatePresentationDescriptor(IMFPresentationDescriptor** descriptor) override;
    STDMETHODIMP GetCharacteristics(DWORD* characteristics) override;
    STDMETHODIMP Pause() override;
    STDMETHODIMP Shutdown() override;
    STDMETHODIMP Start(IMFPresentationDescriptor* descriptor, const GUID* time_format, const PROPVARIANT* start) override;
    STDMETHODIMP Stop() override;
    STDMETHODIMP GetSourceAttributes(IMFAttributes** attributes) override;
    STDMETHODIMP GetStreamAttributes(DWORD stream_id, IMFAttributes** attributes) override;
    STDMETHODIMP SetD3DManager(IUnknown* manager) override;
    STDMETHODIMP GetService(REFGUID service, REFIID riid, void** object) override;
    STDMETHODIMP KsProperty(PKSPROPERTY property, ULONG property_length, void* data, ULONG data_length, ULONG* bytes_returned) override;
    STDMETHODIMP KsMethod(PKSMETHOD method, ULONG method_length, void* data, ULONG data_length, ULONG* bytes_returned) override;
    STDMETHODIMP KsEvent(PKSEVENT event, ULONG event_length, void* data, ULONG data_length, ULONG* bytes_returned) override;

private:
    enum class State { stopped, started, shutdown };
    std::mutex mutex_;
    State state_{State::stopped};
    bool stream_announced_{false};
    ComPtr<IMFMediaEventQueue> events_;
    ComPtr<IMFAttributes> attributes_;
    ComPtr<IMFPresentationDescriptor> descriptor_;
    ComPtr<MediaStream> stream_;
    ComPtr<IUnknown> d3d_manager_;
};

} // namespace asc::win::source
