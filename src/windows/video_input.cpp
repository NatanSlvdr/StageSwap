#include "video_input.hpp"

#include <mfapi.h>
#include <mferror.h>
#include <new>

namespace asc::win {
namespace { constexpr DWORD video_stream = static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM); }

class VideoInputCallback final : public IMFSourceReaderCallback {
public:
    explicit VideoInputCallback(VideoInput& owner) : owner_(&owner) {}
    void detach() { std::scoped_lock lock(mutex_); owner_ = nullptr; }
    STDMETHODIMP QueryInterface(REFIID id, void** object) override {
        if (!object) return E_POINTER;
        if (id == IID_IUnknown || id == __uuidof(IMFSourceReaderCallback)) { *object = this; AddRef(); return S_OK; }
        *object = nullptr; return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return ++references_; }
    STDMETHODIMP_(ULONG) Release() override { const auto left = --references_; if (!left) delete this; return left; }
    STDMETHODIMP OnReadSample(HRESULT status, DWORD stream, DWORD flags, LONGLONG time, IMFSample* sample) override {
        std::scoped_lock lock(mutex_); return owner_ ? owner_->OnReadSample(status, stream, flags, time, sample) : S_OK;
    }
    STDMETHODIMP OnFlush(DWORD stream) override { std::scoped_lock lock(mutex_); return owner_ ? owner_->OnFlush(stream) : S_OK; }
    STDMETHODIMP OnEvent(DWORD stream, IMFMediaEvent* event) override { std::scoped_lock lock(mutex_); return owner_ ? owner_->OnEvent(stream, event) : S_OK; }
private:
    std::atomic<ULONG> references_{1};
    std::mutex mutex_;
    VideoInput* owner_;
};

VideoInput::~VideoInput() { stop(); }

void VideoInput::start(const std::string& symbolic_link) {
    if (symbolic_link.empty()) throw std::invalid_argument("video source identifier is empty");
    stop();
    symbolic_link_ = symbolic_link;
    ComPtr<IMFAttributes> source_attributes;
    check_hresult(MFCreateAttributes(&source_attributes, 2), "Create video source attributes");
    check_hresult(source_attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID), "Set video source type");
    const auto link = wide(symbolic_link);
    check_hresult(source_attributes->SetString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, link.c_str()), "Set video source identifier");
    ComPtr<IMFMediaSource> source;
    check_hresult(MFCreateDeviceSource(source_attributes.Get(), &source), "Open video source");

    auto* callback_impl = new (std::nothrow) VideoInputCallback(*this);
    if (!callback_impl) throw std::bad_alloc{};
    ComPtr<IMFSourceReaderCallback> callback;
    callback.Attach(callback_impl);
    ComPtr<IMFAttributes> reader_attributes;
    check_hresult(MFCreateAttributes(&reader_attributes, 2), "Create video reader attributes");
    check_hresult(reader_attributes->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, callback.Get()), "Set video callback");
    check_hresult(reader_attributes->SetUINT32(MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, TRUE), "Enable Media Foundation video processing");
    ComPtr<IMFSourceReader> reader;
    check_hresult(MFCreateSourceReaderFromMediaSource(source.Get(), reader_attributes.Get(), &reader), "Create video reader");

    ComPtr<IMFMediaType> output;
    check_hresult(MFCreateMediaType(&output), "Create fixed video format");
    check_hresult(output->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "Set video major type");
    check_hresult(output->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_RGB32), "Request RGB32 webcam output");
    check_hresult(MFSetAttributeSize(output.Get(), MF_MT_FRAME_SIZE, pipeline_size.width, pipeline_size.height), "Request 720p webcam output");
    check_hresult(MFSetAttributeRatio(output.Get(), MF_MT_FRAME_RATE, pipeline_fps, 1), "Request 30 fps webcam output");
    check_hresult(output->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive), "Request progressive webcam output");
    check_hresult(output->SetUINT32(MF_MT_DEFAULT_STRIDE, pipeline_size.width * 4u), "Request top-down RGB32 webcam stride");
    check_hresult(reader->SetCurrentMediaType(video_stream, nullptr, output.Get()), "Negotiate fixed 720p30 RGB32 webcam output");
    {
        std::scoped_lock lock(mutex_);
        reader_ = reader; callback_ = callback; callback_impl_ = callback_impl;
    }
    running_ = true;
    last_error_ = S_OK;
    request_next();
}

void VideoInput::stop() noexcept {
    running_ = false;
    { std::scoped_lock callback_lock(callback_mutex_); }
    ComPtr<IMFSourceReader> reader;
    VideoInputCallback* callback_impl = nullptr;
    { std::scoped_lock lock(mutex_); reader = reader_; callback_impl = callback_impl_; }
    if (reader) {
        { std::scoped_lock lock(flush_mutex_); flush_completed_ = false; }
        if (SUCCEEDED(reader->Flush(video_stream))) {
            std::unique_lock lock(flush_mutex_);
            flush_condition_.wait_for(lock, std::chrono::seconds{2}, [this] { return flush_completed_; });
        }
    }
    if (callback_impl) callback_impl->detach();
    std::scoped_lock callback_lock(callback_mutex_);
    std::scoped_lock lock(mutex_);
    reader_.Reset(); callback_.Reset(); callback_impl_ = nullptr; latest_ = {};
}

void VideoInput::restart() { start(symbolic_link_); }
VideoFrame VideoInput::latest_frame() const { std::scoped_lock lock(mutex_); return latest_; }
void VideoInput::request_next() {
    ComPtr<IMFSourceReader> reader;
    { std::scoped_lock lock(mutex_); reader = reader_; }
    if (running_ && reader) {
        const auto result = reader->ReadSample(video_stream, 0, nullptr, nullptr, nullptr, nullptr);
        if (FAILED(result)) last_error_ = result;
    }
}

HRESULT VideoInput::OnReadSample(const HRESULT status, DWORD, const DWORD flags, const LONGLONG timestamp, IMFSample* sample) {
    std::scoped_lock callback_lock(callback_mutex_);
    if (!running_) return S_OK;
    if (FAILED(status) || (flags & MF_SOURCE_READERF_ERROR)) last_error_ = FAILED(status) ? status : E_FAIL;
    else if (sample) {
        ComPtr<IMFMediaBuffer> buffer;
        if (SUCCEEDED(sample->ConvertToContiguousBuffer(&buffer))) {
            BYTE* bytes = nullptr; DWORD length = 0;
            if (SUCCEEDED(buffer->Lock(&bytes, nullptr, &length))) {
                const auto stride = pipeline_size.width * 4u;
                const auto required = static_cast<std::size_t>(stride) * pipeline_size.height;
                if (length >= required) {
                    VideoFrame frame;
                    frame.size = pipeline_size; frame.stride = stride; frame.received_at = Clock::now();
                    frame.presentation_time_100ns = timestamp; frame.sequence = ++sequence_;
                    frame.bgra.assign(bytes, bytes + required);
                    std::scoped_lock lock(mutex_);
                    latest_ = std::move(frame);
                    last_error_ = S_OK;
                }
                buffer->Unlock();
            }
        }
    }
    if (running_) request_next();
    return S_OK;
}

HRESULT VideoInput::OnFlush(DWORD) { { std::scoped_lock lock(flush_mutex_); flush_completed_ = true; } flush_condition_.notify_all(); return S_OK; }
HRESULT VideoInput::OnEvent(DWORD, IMFMediaEvent* event) { HRESULT status = S_OK; if (event && SUCCEEDED(event->GetStatus(&status)) && FAILED(status)) last_error_ = status; return S_OK; }
HRESULT VideoInput::QueryInterface(REFIID id, void** object) { if (!object) return E_POINTER; if (id == IID_IUnknown || id == __uuidof(IMFSourceReaderCallback)) { *object = this; AddRef(); return S_OK; } *object = nullptr; return E_NOINTERFACE; }
ULONG VideoInput::AddRef() { return ++references_; }
ULONG VideoInput::Release() { return --references_; }

} // namespace asc::win
