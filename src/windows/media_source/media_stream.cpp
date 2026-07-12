#include "media_stream.hpp"

#include <mfapi.h>
#include <mferror.h>
#include <algorithm>
#include <cstring>
#include <vector>
#include <utility>
#include <ksmedia.h>

namespace asc::win::source {

HRESULT MediaStream::initialize(IMFMediaSource* parent, const DWORD id, const std::wstring& pipe_name, const std::uint32_t placeholder_color) {
    parent_ = parent; id_ = id;
    placeholder_color_ = placeholder_color | 0xff000000u;
    reader_.configure(pipe_name);
    auto hr = MFCreateEventQueue(&events_); if (FAILED(hr)) return hr;
    std::vector<ComPtr<IMFMediaType>> types;
    for (const auto& [size, subtype] : std::vector<std::pair<Size, GUID>>{
             {{1920, 1080}, MFVideoFormat_NV12}, {{1920, 1080}, MFVideoFormat_RGB32},
             {{1280, 720}, MFVideoFormat_NV12}, {{1280, 720}, MFVideoFormat_RGB32}}) {
        ComPtr<IMFMediaType> type;
        hr = MFCreateMediaType(&type); if (FAILED(hr)) return hr;
        if (FAILED(hr = type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video))) return hr;
        if (FAILED(hr = type->SetGUID(MF_MT_SUBTYPE, subtype))) return hr;
        if (FAILED(hr = MFSetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, size.width, size.height))) return hr;
        if (FAILED(hr = MFSetAttributeRatio(type.Get(), MF_MT_FRAME_RATE, 30, 1))) return hr;
        if (FAILED(hr = MFSetAttributeRatio(type.Get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1))) return hr;
        if (FAILED(hr = type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive))) return hr;
        if (FAILED(hr = type->SetUINT32(MF_MT_ALL_SAMPLES_INDEPENDENT, TRUE))) return hr;
        const bool nv12 = subtype == MFVideoFormat_NV12;
        if (FAILED(hr = type->SetUINT32(MF_MT_DEFAULT_STRIDE, nv12 ? size.width : size.width * 4))) return hr;
        if (FAILED(hr = type->SetUINT32(MF_MT_SAMPLE_SIZE, nv12 ? size.width * size.height * 3 / 2 : size.width * size.height * 4))) return hr;
        if (FAILED(hr = type->SetUINT32(MF_MT_AVG_BITRATE, (nv12 ? size.width * size.height * 12 : size.width * size.height * 32) * 30))) return hr;
        types.push_back(std::move(type));
    }
    IMFMediaType* raw[]{types[0].Get(), types[1].Get(), types[2].Get(), types[3].Get()};
    hr = MFCreateStreamDescriptor(id, ARRAYSIZE(raw), raw, &descriptor_); if (FAILED(hr)) return hr;
    if (FAILED(hr = MFCreateAttributes(&attributes_, 4))) return hr;
    for (IMFAttributes* store : {static_cast<IMFAttributes*>(attributes_.Get()), static_cast<IMFAttributes*>(descriptor_.Get())}) {
        if (FAILED(hr = store->SetGUID(MF_DEVICESTREAM_STREAM_CATEGORY, PINNAME_VIDEO_CAPTURE))) return hr;
        if (FAILED(hr = store->SetUINT32(MF_DEVICESTREAM_STREAM_ID, id_))) return hr;
        if (FAILED(hr = store->SetUINT32(MF_DEVICESTREAM_FRAMESERVER_SHARED, 1))) return hr;
        if (FAILED(hr = store->SetUINT32(MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes::MFFrameSourceTypes_Color))) return hr;
    }
    ComPtr<IMFMediaTypeHandler> handler;
    if (FAILED(hr = descriptor_->GetMediaTypeHandler(&handler))) return hr;
    if (FAILED(hr = handler->SetCurrentMediaType(raw[0]))) return hr;
    current_type_ = raw[0];
    return S_OK;
}

void MediaStream::bgra_to_nv12(const std::uint8_t* bgra, const Size size, std::uint8_t* nv12) {
    auto clamp_byte = [](const int value) { return static_cast<std::uint8_t>(std::clamp(value, 0, 255)); };
    auto* y_plane = nv12;
    auto* uv_plane = nv12 + static_cast<std::size_t>(size.width) * size.height;
    for (std::uint32_t y = 0; y < size.height; ++y) {
        for (std::uint32_t x = 0; x < size.width; ++x) {
            const auto* pixel = bgra + (static_cast<std::size_t>(y) * size.width + x) * 4;
            const int b = pixel[0], g = pixel[1], r = pixel[2];
            y_plane[static_cast<std::size_t>(y) * size.width + x] = clamp_byte(16 + ((47 * r + 157 * g + 16 * b + 128) >> 8));
        }
    }
    for (std::uint32_t y = 0; y < size.height; y += 2) {
        for (std::uint32_t x = 0; x < size.width; x += 2) {
            int sum_u = 0, sum_v = 0;
            for (std::uint32_t dy = 0; dy < 2; ++dy) for (std::uint32_t dx = 0; dx < 2; ++dx) {
                const auto* pixel = bgra + (static_cast<std::size_t>(y + dy) * size.width + x + dx) * 4;
                const int b = pixel[0], g = pixel[1], r = pixel[2];
                sum_u += 128 + ((-26 * r - 87 * g + 112 * b + 128) >> 8);
                sum_v += 128 + ((112 * r - 102 * g - 10 * b + 128) >> 8);
            }
            const auto offset = static_cast<std::size_t>(y / 2) * size.width + x;
            uv_plane[offset] = clamp_byte(sum_u / 4);
            uv_plane[offset + 1] = clamp_byte(sum_v / 4);
        }
    }
}

HRESULT MediaStream::start(IMFMediaType* media_type) {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    current_type_ = media_type;
    UINT32 numerator = 30, denominator = 1;
    MFGetAttributeRatio(media_type, MF_MT_FRAME_RATE, &numerator, &denominator);
    frame_duration_ = numerator ? (10'000'000LL * denominator / numerator) : 333333;
    next_time_ = MFGetSystemTime();
    state_ = MF_STREAM_STATE_RUNNING;
    PROPVARIANT start{}; InitPropVariantFromInt64(next_time_, &start);
    const auto hr = events_->QueueEventParamVar(MEStreamStarted, GUID_NULL, S_OK, &start);
    PropVariantClear(&start);
    return hr;
}

HRESULT MediaStream::stop(const bool send_event) {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    state_ = MF_STREAM_STATE_STOPPED;
    return send_event ? events_->QueueEventParamVar(MEStreamStopped, GUID_NULL, S_OK, nullptr) : S_OK;
}

HRESULT MediaStream::shutdown() {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return S_OK;
    shutdown_ = true; state_ = MF_STREAM_STATE_STOPPED;
    if (events_) events_->Shutdown();
    events_.Reset(); descriptor_.Reset(); attributes_.Reset(); current_type_.Reset(); parent_.Reset();
    return S_OK;
}

HRESULT MediaStream::BeginGetEvent(IMFAsyncCallback* c, IUnknown* s) { return events_ ? events_->BeginGetEvent(c, s) : MF_E_SHUTDOWN; }
HRESULT MediaStream::EndGetEvent(IMFAsyncResult* r, IMFMediaEvent** e) { return events_ ? events_->EndGetEvent(r, e) : MF_E_SHUTDOWN; }
HRESULT MediaStream::GetEvent(const DWORD f, IMFMediaEvent** e) { auto q = events_; return q ? q->GetEvent(f, e) : MF_E_SHUTDOWN; }
HRESULT MediaStream::QueueEvent(const MediaEventType t, REFGUID g, const HRESULT h, const PROPVARIANT* v) { return events_ ? events_->QueueEventParamVar(t, g, h, v) : MF_E_SHUTDOWN; }
HRESULT MediaStream::GetMediaSource(IMFMediaSource** source) { if (!source) return E_POINTER; return parent_ ? parent_.CopyTo(source) : MF_E_SHUTDOWN; }
HRESULT MediaStream::GetStreamDescriptor(IMFStreamDescriptor** descriptor) { if (!descriptor) return E_POINTER; return descriptor_ ? descriptor_.CopyTo(descriptor) : MF_E_SHUTDOWN; }
HRESULT MediaStream::attributes(IMFAttributes** output) const { if (!output) return E_POINTER; return attributes_ ? attributes_.CopyTo(output) : MF_E_SHUTDOWN; }

void MediaStream::scale_bgra(const CpuSharedFrame& input, std::uint8_t* output, const Size output_size, const std::uint32_t output_stride) {
    if (input.bgra.empty()) {
        for (std::uint32_t y = 0; y < output_size.height; ++y) {
            auto* row = reinterpret_cast<std::uint32_t*>(output + static_cast<std::size_t>(y) * output_stride);
            std::fill(row, row + output_size.width, placeholder_color_);
        }
        return;
    }
    struct AxisSample { std::uint32_t first; std::uint32_t second; std::uint32_t weight; };
    std::vector<AxisSample> x_samples(output_size.width), y_samples(output_size.height);
    const auto build_axis = [](const std::uint32_t input_count, const std::uint32_t output_count, std::vector<AxisSample>& samples) {
        for (std::uint32_t value = 0; value < output_count; ++value) {
            const std::uint64_t fixed = ((static_cast<std::uint64_t>(value) * 2 + 1) * input_count * 32768 / output_count);
            const std::int64_t centered = static_cast<std::int64_t>(fixed) - 32768;
            const std::int64_t raw_first = centered < 0 ? -1 : centered / 65536;
            if (raw_first < 0) samples[value] = {0, 0, 0};
            else if (raw_first >= static_cast<std::int64_t>(input_count - 1)) samples[value] = {input_count - 1, input_count - 1, 0};
            else samples[value] = {static_cast<std::uint32_t>(raw_first), static_cast<std::uint32_t>(raw_first + 1),
                                   static_cast<std::uint32_t>(centered - (raw_first << 16))};
        }
    };
    build_axis(input.size.width, output_size.width, x_samples);
    build_axis(input.size.height, output_size.height, y_samples);
    for (std::uint32_t y = 0; y < output_size.height; ++y) {
        for (std::uint32_t x = 0; x < output_size.width; ++x) {
            const auto& xs = x_samples[x]; const auto& ys = y_samples[y];
            const auto* top_left = input.bgra.data() + static_cast<std::size_t>(ys.first) * input.stride + xs.first * 4;
            const auto* top_right = input.bgra.data() + static_cast<std::size_t>(ys.first) * input.stride + xs.second * 4;
            const auto* bottom_left = input.bgra.data() + static_cast<std::size_t>(ys.second) * input.stride + xs.first * 4;
            const auto* bottom_right = input.bgra.data() + static_cast<std::size_t>(ys.second) * input.stride + xs.second * 4;
            auto* destination = output + static_cast<std::size_t>(y) * output_stride + x * 4;
            for (int channel = 0; channel < 4; ++channel) {
                const std::uint64_t top = top_left[channel] * (65536u - xs.weight) + top_right[channel] * xs.weight;
                const std::uint64_t bottom = bottom_left[channel] * (65536u - xs.weight) + bottom_right[channel] * xs.weight;
                destination[channel] = static_cast<std::uint8_t>(((top * (65536u - ys.weight) + bottom * ys.weight) + (1ull << 31)) >> 32);
            }
        }
    }
}

HRESULT MediaStream::make_sample(IUnknown* token, IMFSample** output) {
    if (!output) return E_POINTER;
    *output = nullptr;
    UINT32 width = 0, height = 0;
    auto hr = MFGetAttributeSize(current_type_.Get(), MF_MT_FRAME_SIZE, &width, &height); if (FAILED(hr)) return hr;
    GUID subtype{};
    if (FAILED(hr = current_type_->GetGUID(MF_MT_SUBTYPE, &subtype))) return hr;
    const bool nv12 = subtype == MFVideoFormat_NV12;
    const DWORD bytes = nv12 ? width * height * 3 / 2 : width * height * 4;
    ComPtr<IMFMediaBuffer> buffer;
    hr = MFCreateMemoryBuffer(bytes, &buffer); if (FAILED(hr)) return hr;
    BYTE* destination = nullptr;
    hr = buffer->Lock(&destination, nullptr, nullptr); if (FAILED(hr)) return hr;
    if (!reader_.read_latest(shared_frame_cache_)) shared_frame_cache_ = {};
    if (nv12) {
        scaled_bgra_.resize(static_cast<std::size_t>(width) * height * 4);
        scale_bgra(shared_frame_cache_, scaled_bgra_.data(), {width, height}, width * 4);
        bgra_to_nv12(scaled_bgra_.data(), {width, height}, destination);
    } else scale_bgra(shared_frame_cache_, destination, {width, height}, width * 4);
    buffer->Unlock();
    if (FAILED(hr = buffer->SetCurrentLength(bytes))) return hr;
    ComPtr<IMFSample> sample;
    if (FAILED(hr = MFCreateSample(&sample))) return hr;
    if (FAILED(hr = sample->AddBuffer(buffer.Get()))) return hr;
    if (FAILED(hr = sample->SetSampleTime(next_time_))) return hr;
    if (FAILED(hr = sample->SetSampleDuration(frame_duration_))) return hr;
    if (token && FAILED(hr = sample->SetUnknown(MFSampleExtension_Token, token))) return hr;
    next_time_ += frame_duration_;
    return sample.CopyTo(output);
}

HRESULT MediaStream::RequestSample(IUnknown* token) {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    if (state_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALIDREQUEST;
    ComPtr<IMFSample> sample;
    auto hr = make_sample(token, &sample);
    if (FAILED(hr)) { events_->QueueEventParamVar(MEError, GUID_NULL, hr, nullptr); return hr; }
    return events_->QueueEventParamUnk(MEMediaSample, GUID_NULL, S_OK, sample.Get());
}
HRESULT MediaStream::SetStreamState(const MF_STREAM_STATE state) {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    if (state != MF_STREAM_STATE_STOPPED && state != MF_STREAM_STATE_RUNNING && state != MF_STREAM_STATE_PAUSED) return MF_E_INVALID_STATE_TRANSITION;
    if (state == MF_STREAM_STATE_PAUSED && state_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALID_STATE_TRANSITION;
    state_ = state;
    return S_OK;
}
HRESULT MediaStream::GetStreamState(MF_STREAM_STATE* state) { if (!state) return E_POINTER; std::scoped_lock lock(mutex_); if (shutdown_) return MF_E_SHUTDOWN; *state = state_; return S_OK; }

} // namespace asc::win::source
