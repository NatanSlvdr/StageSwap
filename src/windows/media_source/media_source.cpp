#include "media_source.hpp"
#include "ids.hpp"

#include <mfapi.h>
#include <mferror.h>
#include <propvarutil.h>
#include <ksmedia.h>

namespace asc::win::source {

HRESULT MediaSource::initialize(IMFAttributes* activation_attributes) {
    auto hr = MFCreateEventQueue(&events_); if (FAILED(hr)) return hr;
    if (FAILED(hr = MFCreateAttributes(&attributes_, 8))) return hr;
    if (activation_attributes && FAILED(hr = activation_attributes->CopyAllItems(attributes_.Get()))) return hr;
    if (FAILED(hr = attributes_->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID))) return hr;
    if (FAILED(hr = attributes_->SetString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, L"Automatic Screen Camera"))) return hr;
    ComPtr<IMFSensorProfileCollection> profiles;
    if (SUCCEEDED(MFCreateSensorProfileCollection(&profiles))) {
        ComPtr<IMFSensorProfile> legacy;
        if (SUCCEEDED(MFCreateSensorProfile(KSCAMERAPROFILE_Legacy, 0, nullptr, &legacy)) &&
            SUCCEEDED(legacy->AddProfileFilter(0, L"((RES==;FRT<=30,1;SUT==))")) &&
            SUCCEEDED(profiles->AddProfile(legacy.Get())))
            (void)attributes_->SetUnknown(MF_DEVICEMFT_SENSORPROFILE_COLLECTION, profiles.Get());
    }
    stream_ = Microsoft::WRL::Make<MediaStream>();
    if (!stream_) return E_OUTOFMEMORY;
    CoTaskMemString pipe_name;
    UINT32 pipe_length = 0;
    hr = attributes_->GetAllocatedString(ASC_FRAME_PIPE_NAME, &pipe_name.value, &pipe_length);
    if (FAILED(hr) && hr != MF_E_ATTRIBUTENOTFOUND) return hr;
    UINT32 placeholder_color = 0xff171719u;
    hr = attributes_->GetUINT32(ASC_PLACEHOLDER_COLOR, &placeholder_color);
    if (FAILED(hr) && hr != MF_E_ATTRIBUTENOTFOUND) return hr;
    if (FAILED(hr = stream_->initialize(this, 0, pipe_name.value ? pipe_name.value : L"", placeholder_color))) return hr;
    ComPtr<IMFStreamDescriptor> stream_descriptor;
    if (FAILED(hr = stream_->GetStreamDescriptor(&stream_descriptor))) return hr;
    IMFStreamDescriptor* raw[]{stream_descriptor.Get()};
    if (FAILED(hr = MFCreatePresentationDescriptor(1, raw, &descriptor_))) return hr;
    if (FAILED(hr = descriptor_->SelectStream(0))) return hr;
    state_ = State::stopped;
    return S_OK;
}

HRESULT MediaSource::BeginGetEvent(IMFAsyncCallback* c, IUnknown* s) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->BeginGetEvent(c, s) : MF_E_SHUTDOWN;
}
HRESULT MediaSource::EndGetEvent(IMFAsyncResult* r, IMFMediaEvent** e) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->EndGetEvent(r, e) : MF_E_SHUTDOWN;
}
HRESULT MediaSource::GetEvent(const DWORD f, IMFMediaEvent** e) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->GetEvent(f, e) : MF_E_SHUTDOWN;
}
HRESULT MediaSource::QueueEvent(const MediaEventType t, REFGUID g, const HRESULT h, const PROPVARIANT* v) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->QueueEventParamVar(t, g, h, v) : MF_E_SHUTDOWN;
}

HRESULT MediaSource::CreatePresentationDescriptor(IMFPresentationDescriptor** descriptor) {
    if (!descriptor) return E_POINTER;
    std::scoped_lock lock(mutex_);
    return descriptor_ ? descriptor_->Clone(descriptor) : MF_E_SHUTDOWN;
}
HRESULT MediaSource::GetCharacteristics(DWORD* characteristics) {
    if (!characteristics) return E_POINTER;
    std::scoped_lock lock(mutex_);
    if (state_ == State::shutdown) return MF_E_SHUTDOWN;
    *characteristics = MFMEDIASOURCE_IS_LIVE;
    return S_OK;
}
HRESULT MediaSource::Pause() {
    std::scoped_lock lock(mutex_);
    return state_ == State::shutdown ? MF_E_SHUTDOWN : MF_E_INVALID_STATE_TRANSITION;
}

HRESULT MediaSource::Start(IMFPresentationDescriptor* presentation, const GUID* time_format, const PROPVARIANT* start_position) {
    if (!presentation || !start_position) return E_INVALIDARG;
    if (time_format && *time_format != GUID_NULL) return MF_E_UNSUPPORTED_TIME_FORMAT;
    std::scoped_lock lock(mutex_);
    if (state_ == State::shutdown) return MF_E_SHUTDOWN;
    BOOL selected = FALSE;
    ComPtr<IMFStreamDescriptor> stream_descriptor;
    auto hr = presentation->GetStreamDescriptorByIndex(0, &selected, &stream_descriptor); if (FAILED(hr)) return hr;
    if (!selected) return MF_E_INVALIDREQUEST;
    ComPtr<IMFMediaTypeHandler> handler;
    ComPtr<IMFMediaType> type;
    if (FAILED(hr = stream_descriptor->GetMediaTypeHandler(&handler))) return hr;
    if (FAILED(hr = handler->GetCurrentMediaType(&type))) return hr;
    const auto event_type = stream_announced_ ? MEUpdatedStream : MENewStream;
    if (FAILED(hr = events_->QueueEventParamUnk(event_type, GUID_NULL, S_OK, stream_.Get()))) return hr;
    stream_announced_ = true;
    if (FAILED(hr = stream_->start(type.Get()))) return hr;
    PROPVARIANT start{};
    InitPropVariantFromInt64(MFGetSystemTime(), &start);
    hr = events_->QueueEventParamVar(MESourceStarted, GUID_NULL, S_OK, &start);
    PropVariantClear(&start);
    if (FAILED(hr)) {
        (void)stream_->stop(false);
        return hr;
    }
    state_ = State::started;
    return hr;
}

HRESULT MediaSource::Stop() {
    std::scoped_lock lock(mutex_);
    if (state_ == State::shutdown) return MF_E_SHUTDOWN;
    if (state_ != State::started) return MF_E_INVALID_STATE_TRANSITION;
    const auto hr = stream_->stop(true);
    if (FAILED(hr)) return hr;
    state_ = State::stopped;
    return events_->QueueEventParamVar(MESourceStopped, GUID_NULL, S_OK, nullptr);
}

HRESULT MediaSource::Shutdown() {
    std::scoped_lock lock(mutex_);
    if (state_ == State::shutdown) return S_OK;
    state_ = State::shutdown;
    if (stream_) stream_->shutdown();
    if (events_) events_->Shutdown();
    stream_.Reset(); descriptor_.Reset(); attributes_.Reset(); d3d_manager_.Reset(); events_.Reset();
    return S_OK;
}

HRESULT MediaSource::GetSourceAttributes(IMFAttributes** attributes) {
    if (!attributes) return E_POINTER;
    std::scoped_lock lock(mutex_);
    return attributes_ ? attributes_.CopyTo(attributes) : MF_E_SHUTDOWN;
}
HRESULT MediaSource::GetStreamAttributes(const DWORD stream_id, IMFAttributes** attributes) {
    if (!attributes) return E_POINTER;
    *attributes = nullptr;
    if (stream_id != 0) return MF_E_INVALIDSTREAMNUMBER;
    std::scoped_lock lock(mutex_);
    return stream_ ? stream_->attributes(attributes) : MF_E_SHUTDOWN;
}
HRESULT MediaSource::SetD3DManager(IUnknown* manager) {
    std::scoped_lock lock(mutex_);
    if (state_ == State::shutdown) return MF_E_SHUTDOWN;
    if (manager) {
        ComPtr<IMFDXGIDeviceManager> dxgi_manager;
        const auto hr = manager->QueryInterface(IID_PPV_ARGS(&dxgi_manager));
        if (FAILED(hr)) return E_NOINTERFACE;
    }
    d3d_manager_ = manager;
    return S_OK;
}
HRESULT MediaSource::GetService(REFGUID, REFIID, void** object) { if (!object) return E_POINTER; *object = nullptr; return MF_E_UNSUPPORTED_SERVICE; }
HRESULT MediaSource::KsProperty(PKSPROPERTY, ULONG, void*, ULONG, ULONG* bytes_returned) {
    if (bytes_returned) *bytes_returned = 0;
    return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}
HRESULT MediaSource::KsMethod(PKSMETHOD, ULONG, void*, ULONG, ULONG* bytes_returned) {
    if (bytes_returned) *bytes_returned = 0;
    return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}
HRESULT MediaSource::KsEvent(PKSEVENT, ULONG, void*, ULONG, ULONG* bytes_returned) {
    if (bytes_returned) *bytes_returned = 0;
    return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}

} // namespace asc::win::source
