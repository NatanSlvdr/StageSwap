#include "video_input.hpp"

#include "asc/core/video_format.hpp"

#include <mfapi.h>
#include <mferror.h>

#include <array>
#include <chrono>
#include <limits>
#include <new>
#include <vector>

namespace asc::win {

class VideoInputCallback final : public IMFSourceReaderCallback {
public:
    explicit VideoInputCallback(VideoInput& owner) noexcept : owner_(&owner) {}

    void detach() noexcept {
        std::scoped_lock lock(mutex_);
        owner_ = nullptr;
    }

    STDMETHODIMP QueryInterface(const REFIID riid, void** object) override {
        if (!object) return E_POINTER;
        if (riid == IID_IUnknown || riid == __uuidof(IMFSourceReaderCallback)) {
            *object = static_cast<IMFSourceReaderCallback*>(this);
            AddRef();
            return S_OK;
        }
        *object = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return ++references_; }
    STDMETHODIMP_(ULONG) Release() override {
        const auto remaining = --references_;
        if (remaining == 0) delete this;
        return remaining;
    }
    STDMETHODIMP OnReadSample(const HRESULT status, const DWORD stream_index, const DWORD stream_flags,
                              const LONGLONG timestamp, IMFSample* sample) override {
        std::scoped_lock lock(mutex_);
        return owner_ ? owner_->OnReadSample(status, stream_index, stream_flags, timestamp, sample) : S_OK;
    }
    STDMETHODIMP OnFlush(const DWORD stream_index) override {
        std::scoped_lock lock(mutex_);
        return owner_ ? owner_->OnFlush(stream_index) : S_OK;
    }
    STDMETHODIMP OnEvent(const DWORD stream_index, IMFMediaEvent* event) override {
        std::scoped_lock lock(mutex_);
        return owner_ ? owner_->OnEvent(stream_index, event) : S_OK;
    }

private:
    std::atomic<ULONG> references_{1};
    std::mutex mutex_;
    VideoInput* owner_;
};

VideoInput::VideoInput(D3DDevice& d3d) : d3d_(d3d) {}
VideoInput::~VideoInput() { stop(); }

namespace {

std::uint32_t subtype_rank(const GUID& subtype) noexcept {
    if (subtype == MFVideoFormat_NV12) return 0;
    if (subtype == MFVideoFormat_YUY2) return 1;
    if (subtype == MFVideoFormat_ARGB32 || subtype == MFVideoFormat_RGB32) return 2;
    if (subtype == MFVideoFormat_MJPG) return 3;
    return 10;
}

ComPtr<IMFMediaType> converted_type(const asc::CaptureFormatCandidate& format, const GUID& subtype) {
    ComPtr<IMFMediaType> type;
    check_hresult(MFCreateMediaType(&type), "MFCreateMediaType(video output)");
    check_hresult(type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "Set video major type");
    check_hresult(type->SetGUID(MF_MT_SUBTYPE, subtype), "Set converted video subtype");
    check_hresult(MFSetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, format.size.width, format.size.height), "Set converted video frame size");
    check_hresult(MFSetAttributeRatio(type.Get(), MF_MT_FRAME_RATE, format.frame_rate_numerator,
                                      format.frame_rate_denominator), "Set converted video frame rate");
    check_hresult(type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive), "Set progressive video output");
    const auto stride = static_cast<std::uint64_t>(format.size.width) * 4;
    if (stride <= std::numeric_limits<UINT32>::max())
        check_hresult(type->SetUINT32(MF_MT_DEFAULT_STRIDE, static_cast<UINT32>(stride)), "Set converted video stride");
    return type;
}

} // namespace

void VideoInput::start(const std::string& symbolic_link, const Size preferred_size, const std::uint32_t preferred_fps) {
    if (symbolic_link.empty()) throw std::invalid_argument("video source identifier is empty");
    if (preferred_size.width == 0 || preferred_size.height == 0 || preferred_fps == 0)
        throw std::invalid_argument("preferred video format is invalid");
    stop();
    {
        std::scoped_lock lock(mutex_);
        // Keep the requested device even when it is currently disconnected so
        // restart() can recover the same physical selection later.
        symbolic_link_ = symbolic_link;
        preferred_size_ = preferred_size;
        preferred_fps_ = preferred_fps;
    }
    ComPtr<IMFAttributes> source_attributes;
    check_hresult(MFCreateAttributes(&source_attributes, 2), "MFCreateAttributes(video source)");
    check_hresult(source_attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID), "Set video source type");
    const auto link = wide(symbolic_link);
    check_hresult(source_attributes->SetString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, link.c_str()), "Set video symbolic link");
    ComPtr<IMFMediaSource> source;
    check_hresult(MFCreateDeviceSource(source_attributes.Get(), &source), "MFCreateDeviceSource");

    ComPtr<IMFAttributes> reader_attributes;
    check_hresult(MFCreateAttributes(&reader_attributes, 4), "MFCreateAttributes(source reader)");
    auto* callback_impl = new (std::nothrow) VideoInputCallback(*this);
    if (!callback_impl) throw std::bad_alloc{};
    ComPtr<IMFSourceReaderCallback> callback;
    callback.Attach(callback_impl);
    check_hresult(reader_attributes->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, callback.Get()), "Set source reader callback");
    check_hresult(reader_attributes->SetUnknown(MF_SOURCE_READER_D3D_MANAGER, d3d_.mf_manager()), "Set source reader D3D manager");
    // Basic video processing is mutually exclusive with a D3D manager. The
    // advanced processor supports color conversion while retaining GPU buffers.
    check_hresult(reader_attributes->SetUINT32(MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, TRUE),
                  "Enable advanced source reader video processing");
    ComPtr<IMFSourceReader> reader;
    check_hresult(MFCreateSourceReaderFromMediaSource(source.Get(), reader_attributes.Get(), &reader), "MFCreateSourceReaderFromMediaSource");

    std::vector<CaptureFormatCandidate> native_formats;
    for (DWORD native_index = 0;; ++native_index) {
        ComPtr<IMFMediaType> native_type;
        const auto result = reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, native_index, &native_type);
        if (result == MF_E_NO_MORE_TYPES) break;
        if (FAILED(result)) {
            last_error_ = result;
            break;
        }
        GUID major{};
        GUID subtype{};
        UINT32 width = 0, height = 0, numerator = 0, denominator = 0;
        if (FAILED(native_type->GetGUID(MF_MT_MAJOR_TYPE, &major)) || major != MFMediaType_Video ||
            FAILED(native_type->GetGUID(MF_MT_SUBTYPE, &subtype)) ||
            FAILED(MFGetAttributeSize(native_type.Get(), MF_MT_FRAME_SIZE, &width, &height))) continue;
        if (width > D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION || height > D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION) continue;
        if (FAILED(MFGetAttributeRatio(native_type.Get(), MF_MT_FRAME_RATE, &numerator, &denominator)) &&
            FAILED(MFGetAttributeRatio(native_type.Get(), MF_MT_FRAME_RATE_RANGE_MAX, &numerator, &denominator))) continue;
        native_formats.push_back({{width, height}, numerator, denominator, subtype_rank(subtype), native_index});
    }
    const auto ranked = rank_capture_formats(native_formats, preferred_size, preferred_fps);
    if (ranked.empty()) throw std::runtime_error("selected video source exposes no usable native formats");

    HRESULT negotiation_result = MF_E_INVALIDMEDIATYPE;
    for (const auto& candidate : ranked) {
        for (const auto& converted_subtype : std::array{MFVideoFormat_ARGB32, MFVideoFormat_RGB32}) {
            const auto output_type = converted_type(candidate, converted_subtype);
            negotiation_result = reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, nullptr, output_type.Get());
            if (SUCCEEDED(negotiation_result)) break;
        }
        if (SUCCEEDED(negotiation_result)) break;
    }
    check_hresult(negotiation_result, "Negotiate converted video format");
    {
        std::scoped_lock lock(mutex_);
        reader_ = reader;
        callback_ = callback;
        callback_impl_ = callback_impl;
        refresh_output_format(reader_.Get());
    }
    last_error_ = S_OK;
    running_.store(true, std::memory_order_release);
    request_next();
}

void VideoInput::stop() noexcept {
    running_.store(false, std::memory_order_release);
    // Wait for a callback that already observed running=true to finish before
    // capture resources can be released or the shared D3D device can be reset.
    { std::scoped_lock callback_lock(callback_mutex_); }
    ComPtr<IMFSourceReader> reader;
    ComPtr<IMFSourceReaderCallback> callback;
    VideoInputCallback* callback_impl = nullptr;
    {
        std::scoped_lock lock(mutex_);
        reader = reader_;
        callback = callback_;
        callback_impl = callback_impl_;
    }
    if (reader) {
        {
            std::scoped_lock flush_lock(flush_mutex_);
            flush_completed_ = false;
        }
        if (SUCCEEDED(reader->Flush(MF_SOURCE_READER_FIRST_VIDEO_STREAM))) {
            std::unique_lock flush_lock(flush_mutex_);
            flush_condition_.wait_for(flush_lock, std::chrono::seconds{2}, [this] { return flush_completed_; });
        }
    }
    // A per-reader callback proxy makes a bounded flush safe: even if a broken
    // driver delivers a late callback, it no longer has access to this object.
    if (callback_impl) callback_impl->detach();
    std::scoped_lock callback_lock(callback_mutex_);
    std::scoped_lock lock(mutex_);
    reader_.Reset();
    callback_.Reset();
    callback_impl_ = nullptr;
    latest_ = {};
    upload_texture_.Reset();
    active_size_ = {};
    active_stride_ = 0;
}

void VideoInput::restart() {
    std::string symbolic_link;
    Size preferred_size;
    std::uint32_t preferred_fps = 0;
    {
        std::scoped_lock lock(mutex_);
        symbolic_link = symbolic_link_;
        preferred_size = preferred_size_;
        preferred_fps = preferred_fps_;
    }
    start(symbolic_link, preferred_size, preferred_fps);
}
VideoFrame VideoInput::latest_frame() const { std::scoped_lock lock(mutex_); return latest_; }
void VideoInput::request_next() {
    ComPtr<IMFSourceReader> reader;
    { std::scoped_lock lock(mutex_); reader = reader_; }
    if (running_ && reader) {
        const auto result = reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0, nullptr, nullptr, nullptr, nullptr);
        if (FAILED(result)) last_error_ = result;
    }
}

void VideoInput::refresh_output_format(IMFSourceReader* const reader) noexcept {
    if (!reader) return;
    ComPtr<IMFMediaType> type;
    if (FAILED(reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, &type))) return;
    UINT32 width = 0, height = 0;
    if (SUCCEEDED(MFGetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, &width, &height)) && width != 0 && height != 0)
        active_size_ = {width, height};
    UINT32 raw_stride = 0;
    if (SUCCEEDED(type->GetUINT32(MF_MT_DEFAULT_STRIDE, &raw_stride))) active_stride_ = static_cast<LONG>(raw_stride);
    else if (active_size_.width <= static_cast<std::uint32_t>(std::numeric_limits<LONG>::max() / 4))
        active_stride_ = static_cast<LONG>(active_size_.width * 4);
}

HRESULT VideoInput::OnReadSample(const HRESULT status, DWORD, const DWORD flags, const LONGLONG timestamp, IMFSample* sample) {
    std::scoped_lock callback_lock(callback_mutex_);
    if (!running_.load(std::memory_order_acquire)) return S_OK;
    if (FAILED(status)) last_error_ = status;
    else if ((flags & MF_SOURCE_READERF_ERROR) != 0) last_error_ = E_FAIL;
    else {
        if ((flags & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED)) != 0) {
            std::scoped_lock lock(mutex_);
            refresh_output_format(reader_.Get());
        }
    }
    if (SUCCEEDED(status) && (flags & MF_SOURCE_READERF_ERROR) == 0 && sample) {
        bool frame_copied = false;
        bool gpu_frame_copied = false;
        ComPtr<IMFMediaBuffer> first_buffer;
        ComPtr<IMFDXGIBuffer> dxgi_buffer;
        if (SUCCEEDED(sample->GetBufferByIndex(0, &first_buffer)) && SUCCEEDED(first_buffer.As(&dxgi_buffer))) {
            ComPtr<ID3D11Texture2D> source_texture;
            if (SUCCEEDED(dxgi_buffer->GetResource(IID_PPV_ARGS(&source_texture)))) {
                D3D11_TEXTURE2D_DESC desc{}; source_texture->GetDesc(&desc);
                if (desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM || desc.Format == DXGI_FORMAT_R8G8B8A8_UNORM) {
                    std::scoped_lock lock(mutex_);
                    bool recreate = !upload_texture_;
                    if (upload_texture_) { D3D11_TEXTURE2D_DESC current{}; upload_texture_->GetDesc(&current); recreate = current.Width != desc.Width || current.Height != desc.Height || current.Format != desc.Format; }
                    if (recreate) {
                        upload_texture_.Reset(); desc.BindFlags = D3D11_BIND_SHADER_RESOURCE; desc.MiscFlags = 0;
                        desc.Usage = D3D11_USAGE_DEFAULT; desc.CPUAccessFlags = 0;
                        const auto create_result = d3d_.device()->CreateTexture2D(&desc, nullptr, &upload_texture_);
                        if (FAILED(create_result)) last_error_ = create_result;
                    }
                    if (upload_texture_) {
                        d3d_.context()->CopyResource(upload_texture_.Get(), source_texture.Get());
                        latest_ = {upload_texture_, {desc.Width, desc.Height}, desc.Format, std::chrono::steady_clock::now(), timestamp};
                        gpu_frame_copied = true;
                        frame_copied = true;
                    }
                }
            }
        }
        if (!gpu_frame_copied) {
            ComPtr<IMFMediaBuffer> buffer;
            if (SUCCEEDED(sample->ConvertToContiguousBuffer(&buffer))) {
                BYTE* bytes = nullptr;
                DWORD length = 0;
                if (SUCCEEDED(buffer->Lock(&bytes, nullptr, &length))) {
                    Size frame_size;
                    LONG frame_stride = 0;
                    {
                        std::scoped_lock lock(mutex_);
                        frame_size = active_size_;
                        frame_stride = active_stride_;
                    }
                    D3D11_TEXTURE2D_DESC desc{};
                    desc.Width = frame_size.width;
                    desc.Height = frame_size.height;
                    desc.MipLevels = 1;
                    desc.ArraySize = 1;
                    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
                    desc.SampleDesc.Count = 1;
                    desc.Usage = D3D11_USAGE_DEFAULT;
                    desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
                    const auto row_bytes = static_cast<std::uint64_t>(frame_size.width) * 4;
                    const auto stride = frame_stride > 0 ? static_cast<std::uint64_t>(frame_stride) : row_bytes;
                    const bool valid_dimensions = frame_size.width != 0 && frame_size.height != 0 &&
                        frame_size.width <= D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION &&
                        frame_size.height <= D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION;
                    const auto required = valid_dimensions ? stride * (frame_size.height - 1) + row_bytes : 0;
                    if (valid_dimensions && row_bytes <= std::numeric_limits<UINT>::max() &&
                        stride <= std::numeric_limits<UINT>::max() && required <= length) {
                        std::scoped_lock lock(mutex_);
                        bool recreate = !upload_texture_;
                        if (upload_texture_) { D3D11_TEXTURE2D_DESC current{}; upload_texture_->GetDesc(&current); recreate = current.Width != desc.Width || current.Height != desc.Height || current.Format != desc.Format; }
                        if (recreate) {
                            upload_texture_.Reset();
                            const auto create_result = d3d_.device()->CreateTexture2D(&desc, nullptr, &upload_texture_);
                            if (FAILED(create_result)) last_error_ = create_result;
                        }
                        if (upload_texture_) {
                            d3d_.context()->UpdateSubresource(upload_texture_.Get(), 0, nullptr, bytes, static_cast<UINT>(stride), 0);
                            latest_ = {upload_texture_, frame_size, desc.Format, std::chrono::steady_clock::now(), timestamp};
                            frame_copied = true;
                        }
                    }
                    buffer->Unlock();
                }
            }
        }
        if (frame_copied) last_error_ = S_OK;
    }
    if (running_.load(std::memory_order_acquire)) request_next();
    return S_OK;
}

HRESULT VideoInput::OnFlush(DWORD) {
    {
        std::scoped_lock flush_lock(flush_mutex_);
        flush_completed_ = true;
    }
    flush_condition_.notify_all();
    return S_OK;
}
HRESULT VideoInput::OnEvent(DWORD, IMFMediaEvent* event) {
    if (event) {
        HRESULT status = S_OK;
        if (SUCCEEDED(event->GetStatus(&status)) && FAILED(status)) last_error_ = status;
    }
    return S_OK;
}
HRESULT VideoInput::QueryInterface(const REFIID riid, void** object) {
    if (!object) return E_POINTER;
    if (riid == IID_IUnknown || riid == __uuidof(IMFSourceReaderCallback)) {
        *object = static_cast<IMFSourceReaderCallback*>(this); AddRef(); return S_OK;
    }
    *object = nullptr;
    return E_NOINTERFACE;
}
ULONG VideoInput::AddRef() { return ++references_; }
ULONG VideoInput::Release() {
    const auto remaining = --references_;
    // Lifetime is owned by the application; the callback's reader reference is released by stop().
    return remaining;
}

} // namespace asc::win
