#include "video_input.hpp"

#include <mfapi.h>
#include <mferror.h>

namespace asc::win {

VideoInput::VideoInput(D3DDevice& d3d) : d3d_(d3d) {}
VideoInput::~VideoInput() { stop(); }

void VideoInput::start(const std::string& symbolic_link, const Size preferred_size, const std::uint32_t preferred_fps) {
    stop();
    symbolic_link_ = symbolic_link;
    preferred_size_ = preferred_size;
    preferred_fps_ = preferred_fps;
    ComPtr<IMFAttributes> source_attributes;
    check_hresult(MFCreateAttributes(&source_attributes, 2), "MFCreateAttributes(video source)");
    check_hresult(source_attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID), "Set video source type");
    const auto link = wide(symbolic_link);
    check_hresult(source_attributes->SetString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, link.c_str()), "Set video symbolic link");
    ComPtr<IMFMediaSource> source;
    check_hresult(MFCreateDeviceSource(source_attributes.Get(), &source), "MFCreateDeviceSource");

    ComPtr<IMFAttributes> reader_attributes;
    check_hresult(MFCreateAttributes(&reader_attributes, 4), "MFCreateAttributes(source reader)");
    check_hresult(reader_attributes->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, this), "Set source reader callback");
    check_hresult(reader_attributes->SetUnknown(MF_SOURCE_READER_D3D_MANAGER, d3d_.mf_manager()), "Set source reader D3D manager");
    check_hresult(reader_attributes->SetUINT32(MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, TRUE), "Enable source reader video processing");
    check_hresult(MFCreateSourceReaderFromMediaSource(source.Get(), reader_attributes.Get(), &reader_), "MFCreateSourceReaderFromMediaSource");

    ComPtr<IMFMediaType> output_type;
    check_hresult(MFCreateMediaType(&output_type), "MFCreateMediaType(video output)");
    check_hresult(output_type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "Set video major type");
    check_hresult(output_type->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_ARGB32), "Set ARGB32 subtype");
    check_hresult(MFSetAttributeSize(output_type.Get(), MF_MT_FRAME_SIZE, preferred_size.width, preferred_size.height), "Set video frame size");
    check_hresult(MFSetAttributeRatio(output_type.Get(), MF_MT_FRAME_RATE, preferred_fps, 1), "Set video frame rate");
    check_hresult(reader_->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, nullptr, output_type.Get()), "Set source reader output type");
    running_ = true;
    last_error_ = S_OK;
    request_next();
}

void VideoInput::stop() noexcept {
    running_ = false;
    std::scoped_lock lock(mutex_);
    if (reader_) reader_->Flush(MF_SOURCE_READER_FIRST_VIDEO_STREAM);
    reader_.Reset();
    latest_ = {};
    upload_texture_.Reset();
}

void VideoInput::restart() { start(symbolic_link_, preferred_size_, preferred_fps_); }
VideoFrame VideoInput::latest_frame() const { std::scoped_lock lock(mutex_); return latest_; }
void VideoInput::request_next() {
    ComPtr<IMFSourceReader> reader;
    { std::scoped_lock lock(mutex_); reader = reader_; }
    if (running_ && reader) {
        const auto result = reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0, nullptr, nullptr, nullptr, nullptr);
        if (FAILED(result)) last_error_ = result;
    }
}

HRESULT VideoInput::OnReadSample(const HRESULT status, DWORD, const DWORD flags, const LONGLONG timestamp, IMFSample* sample) {
    if (FAILED(status)) last_error_ = status;
    else if ((flags & MF_SOURCE_READERF_ERROR) != 0) last_error_ = E_FAIL;
    else if (sample) {
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
                        d3d_.device()->CreateTexture2D(&desc, nullptr, &upload_texture_);
                    }
                    if (upload_texture_) {
                        d3d_.context()->CopyResource(upload_texture_.Get(), source_texture.Get());
                        latest_ = {upload_texture_, {desc.Width, desc.Height}, desc.Format, std::chrono::steady_clock::now(), timestamp};
                        gpu_frame_copied = true;
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
                    D3D11_TEXTURE2D_DESC desc{};
                    desc.Width = preferred_size_.width;
                    desc.Height = preferred_size_.height;
                    desc.MipLevels = 1;
                    desc.ArraySize = 1;
                    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
                    desc.SampleDesc.Count = 1;
                    desc.Usage = D3D11_USAGE_DEFAULT;
                    desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
                    if (length >= preferred_size_.width * preferred_size_.height * 4) {
                        std::scoped_lock lock(mutex_);
                        bool recreate = !upload_texture_;
                        if (upload_texture_) { D3D11_TEXTURE2D_DESC current{}; upload_texture_->GetDesc(&current); recreate = current.Width != desc.Width || current.Height != desc.Height || current.Format != desc.Format; }
                        if (recreate) { upload_texture_.Reset(); d3d_.device()->CreateTexture2D(&desc, nullptr, &upload_texture_); }
                        if (upload_texture_) {
                            d3d_.context()->UpdateSubresource(upload_texture_.Get(), 0, nullptr, bytes, preferred_size_.width * 4, 0);
                            latest_ = {upload_texture_, preferred_size_, desc.Format, std::chrono::steady_clock::now(), timestamp};
                        }
                    }
                    buffer->Unlock();
                }
            }
        }
    }
    if (running_) request_next();
    return S_OK;
}

HRESULT VideoInput::OnFlush(DWORD) { return S_OK; }
HRESULT VideoInput::OnEvent(DWORD, IMFMediaEvent*) { return S_OK; }
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
