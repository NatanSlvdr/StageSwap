#include "media_stream.hpp"

#include "asc/core/pixel_conversion.hpp"

#include <mfapi.h>
#include <mferror.h>
#include <propvarutil.h>
#include <algorithm>
#include <cstring>
#include <cstdint>
#include <new>
#include <vector>
#include <utility>
#include <ks.h>
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
        if (FAILED(hr = type->SetUINT32(MF_MT_FIXED_SIZE_SAMPLES, TRUE))) return hr;
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

HRESULT MediaStream::start(IMFMediaType* media_type) {
    if (!media_type) return E_INVALIDARG;
    std::scoped_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    ComPtr<IMFMediaTypeHandler> handler;
    auto hr = descriptor_->GetMediaTypeHandler(&handler);
    if (FAILED(hr)) return hr;
    if (FAILED(hr = handler->IsMediaTypeSupported(media_type, nullptr))) return MF_E_INVALIDMEDIATYPE;
    UINT32 numerator = 30, denominator = 1;
    if (FAILED(hr = MFGetAttributeRatio(media_type, MF_MT_FRAME_RATE, &numerator, &denominator)) ||
        numerator == 0 || denominator == 0) return MF_E_INVALIDMEDIATYPE;
    frame_duration_ = 10'000'000LL * denominator / numerator;
    if (frame_duration_ <= 0 || !QueryPerformanceFrequency(&qpc_frequency_) || !QueryPerformanceCounter(&next_qpc_))
        return E_UNEXPECTED;
    current_type_ = media_type;
    const auto qpc_product = qpc_frequency_.QuadPart * frame_duration_;
    qpc_step_ = qpc_product / 10'000'000LL;
    qpc_remainder_step_ = qpc_product % 10'000'000LL;
    qpc_remainder_ = 0;
    next_time_ = MFGetSystemTime();
    state_ = MF_STREAM_STATE_RUNNING;
    PROPVARIANT start{}; InitPropVariantFromInt64(next_time_, &start);
    hr = events_->QueueEventParamVar(MEStreamStarted, GUID_NULL, S_OK, &start);
    PropVariantClear(&start);
    if (FAILED(hr)) state_ = MF_STREAM_STATE_STOPPED;
    state_changed_.notify_all();
    return hr;
}

HRESULT MediaStream::stop(const bool send_event) {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    state_ = MF_STREAM_STATE_STOPPED;
    state_changed_.notify_all();
    return send_event ? events_->QueueEventParamVar(MEStreamStopped, GUID_NULL, S_OK, nullptr) : S_OK;
}

HRESULT MediaStream::shutdown() {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return S_OK;
    shutdown_ = true; state_ = MF_STREAM_STATE_STOPPED;
    state_changed_.notify_all();
    if (events_) events_->Shutdown();
    events_.Reset(); descriptor_.Reset(); attributes_.Reset(); current_type_.Reset(); parent_.Reset();
    return S_OK;
}

HRESULT MediaStream::BeginGetEvent(IMFAsyncCallback* c, IUnknown* s) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->BeginGetEvent(c, s) : MF_E_SHUTDOWN;
}
HRESULT MediaStream::EndGetEvent(IMFAsyncResult* r, IMFMediaEvent** e) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->EndGetEvent(r, e) : MF_E_SHUTDOWN;
}
HRESULT MediaStream::GetEvent(const DWORD f, IMFMediaEvent** e) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->GetEvent(f, e) : MF_E_SHUTDOWN;
}
HRESULT MediaStream::QueueEvent(const MediaEventType t, REFGUID g, const HRESULT h, const PROPVARIANT* v) {
    ComPtr<IMFMediaEventQueue> queue;
    { std::scoped_lock lock(mutex_); queue = events_; }
    return queue ? queue->QueueEventParamVar(t, g, h, v) : MF_E_SHUTDOWN;
}
HRESULT MediaStream::GetMediaSource(IMFMediaSource** source) {
    if (!source) return E_POINTER;
    std::scoped_lock lock(mutex_);
    return parent_ ? parent_.CopyTo(source) : MF_E_SHUTDOWN;
}
HRESULT MediaStream::GetStreamDescriptor(IMFStreamDescriptor** descriptor) {
    if (!descriptor) return E_POINTER;
    std::scoped_lock lock(mutex_);
    return descriptor_ ? descriptor_.CopyTo(descriptor) : MF_E_SHUTDOWN;
}
HRESULT MediaStream::attributes(IMFAttributes** output) const {
    if (!output) return E_POINTER;
    std::scoped_lock lock(mutex_);
    return attributes_ ? attributes_.CopyTo(output) : MF_E_SHUTDOWN;
}

void MediaStream::prepare_scaling_map(const Size input_size, const Size output_size) {
    if (input_size.width == 0 || input_size.height == 0 || input_size == output_size) return;
    if (scaling_input_size_ == input_size && scaling_output_size_ == output_size &&
        x_samples_.size() == output_size.width && y_samples_.size() == output_size.height) return;
    x_samples_.resize(output_size.width);
    y_samples_.resize(output_size.height);
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
    build_axis(input_size.width, output_size.width, x_samples_);
    build_axis(input_size.height, output_size.height, y_samples_);
    scaling_input_size_ = input_size;
    scaling_output_size_ = output_size;
}

void MediaStream::scale_bgra(const CpuSharedFrame& input, std::uint8_t* output, const Size output_size, const std::uint32_t output_stride) {
    if (input.bgra.empty()) {
        for (std::uint32_t y = 0; y < output_size.height; ++y) {
            auto* row = reinterpret_cast<std::uint32_t*>(output + static_cast<std::size_t>(y) * output_stride);
            std::fill(row, row + output_size.width, placeholder_color_);
        }
        return;
    }
    if (input.size == output_size) {
        const auto row_bytes = static_cast<std::size_t>(output_size.width) * 4;
        if (input.stride == output_stride && input.stride == row_bytes) {
            std::memcpy(output, input.bgra.data(), row_bytes * output_size.height);
        } else {
            for (std::uint32_t y = 0; y < output_size.height; ++y)
                std::memcpy(output + static_cast<std::size_t>(y) * output_stride,
                            input.bgra.data() + static_cast<std::size_t>(y) * input.stride, row_bytes);
        }
        return;
    }
    for (std::uint32_t y = 0; y < output_size.height; ++y) {
        for (std::uint32_t x = 0; x < output_size.width; ++x) {
            const auto& xs = x_samples_[x]; const auto& ys = y_samples_[y];
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
    if (!nv12 && subtype != MFVideoFormat_RGB32) return MF_E_INVALIDMEDIATYPE;
    if (!((width == 1920 && height == 1080) || (width == 1280 && height == 720)) ||
        (nv12 && ((width & 1) != 0 || (height & 1) != 0))) return MF_E_INVALIDMEDIATYPE;
    if (!reader_.read_latest(shared_frame_cache_)) shared_frame_cache_ = {};
    prepare_scaling_map(shared_frame_cache_.size, {width, height});
    const bool direct_nv12 = nv12 && !shared_frame_cache_.bgra.empty() && shared_frame_cache_.size == Size{width, height};
    if (nv12 && !direct_nv12) scaled_bgra_.resize(static_cast<std::size_t>(width) * height * 4);
    ComPtr<IMFMediaBuffer> buffer;
    hr = MFCreate2DMediaBuffer(width, height, subtype.Data1, FALSE, &buffer); if (FAILED(hr)) return hr;
    ComPtr<IMF2DBuffer2> buffer_2d;
    if (FAILED(hr = buffer.As(&buffer_2d))) return hr;
    BYTE* destination = nullptr;
    BYTE* buffer_start = nullptr;
    DWORD buffer_length = 0;
    LONG pitch = 0;
    hr = buffer_2d->Lock2DSize(MF2DBuffer_LockFlags_Write, &destination, &pitch, &buffer_start, &buffer_length); if (FAILED(hr)) return hr;
    const auto minimum_stride = static_cast<std::uint64_t>(width) * (nv12 ? 1 : 4);
    if (pitch <= 0 || static_cast<std::uint64_t>(pitch) < minimum_stride) {
        buffer_2d->Unlock2D();
        return MF_E_BUFFERTOOSMALL;
    }
    const auto start_address = reinterpret_cast<std::uintptr_t>(buffer_start);
    const auto destination_address = reinterpret_cast<std::uintptr_t>(destination);
    const auto last_row = nv12 ? static_cast<std::uint64_t>(height + height / 2 - 1) : height - 1;
    const auto required_end = destination_address + last_row * static_cast<std::uint64_t>(pitch) + minimum_stride;
    if (destination_address < start_address || required_end < destination_address ||
        required_end - start_address > buffer_length) {
        buffer_2d->Unlock2D();
        return MF_E_BUFFERTOOSMALL;
    }
    if (nv12) {
        const auto destination_offset = static_cast<std::size_t>(destination_address - start_address);
        const std::span<std::uint8_t> nv12_bytes(destination, static_cast<std::size_t>(buffer_length) - destination_offset);
        bool converted = false;
        if (direct_nv12) {
            converted = asc::bgra_to_nv12(shared_frame_cache_.bgra, {width, height}, shared_frame_cache_.stride,
                                          nv12_bytes, static_cast<std::size_t>(pitch));
        } else {
            scale_bgra(shared_frame_cache_, scaled_bgra_.data(), {width, height}, width * 4);
            converted = asc::bgra_to_nv12(scaled_bgra_, {width, height}, static_cast<std::size_t>(width) * 4,
                                          nv12_bytes, static_cast<std::size_t>(pitch));
        }
        if (!converted) {
            buffer_2d->Unlock2D();
            return MF_E_BUFFERTOOSMALL;
        }
    } else scale_bgra(shared_frame_cache_, destination, {width, height}, static_cast<std::uint32_t>(pitch));
    if (FAILED(hr = buffer_2d->Unlock2D())) return hr;
    const auto valid_length = static_cast<DWORD>(required_end - start_address);
    if (FAILED(hr = buffer->SetCurrentLength(valid_length))) return hr;
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
    std::unique_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    if (state_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALIDREQUEST;
    if (qpc_frequency_.QuadPart <= 0) return MF_E_INVALIDREQUEST;
    LARGE_INTEGER now{};
    if (!QueryPerformanceCounter(&now)) return E_UNEXPECTED;
    if (qpc_step_ > 0 && now.QuadPart > next_qpc_.QuadPart + qpc_step_) {
        const auto skipped = (now.QuadPart - next_qpc_.QuadPart) / qpc_step_;
        next_qpc_.QuadPart += qpc_step_ * skipped;
        qpc_remainder_ += qpc_remainder_step_ * skipped;
        next_qpc_.QuadPart += qpc_remainder_ / 10'000'000LL;
        qpc_remainder_ %= 10'000'000LL;
        next_time_ += frame_duration_ * skipped;
    }
    if (now.QuadPart < next_qpc_.QuadPart) {
        const auto wait_ticks = next_qpc_.QuadPart - now.QuadPart;
        const auto wait = std::chrono::duration<double>(static_cast<double>(wait_ticks) / qpc_frequency_.QuadPart);
        state_changed_.wait_for(lock, wait, [&] { return shutdown_ || state_ != MF_STREAM_STATE_RUNNING; });
        if (shutdown_) return MF_E_SHUTDOWN;
        if (state_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALIDREQUEST;
    }
    ComPtr<IMFSample> sample;
    HRESULT hr = S_OK;
    try { hr = make_sample(token, &sample); }
    catch (const std::bad_alloc&) { hr = E_OUTOFMEMORY; }
    catch (...) { hr = E_UNEXPECTED; }
    if (FAILED(hr)) { events_->QueueEventParamVar(MEError, GUID_NULL, hr, nullptr); return hr; }
    next_qpc_.QuadPart += qpc_step_;
    qpc_remainder_ += qpc_remainder_step_;
    if (qpc_remainder_ >= 10'000'000LL) { next_qpc_.QuadPart += qpc_remainder_ / 10'000'000LL; qpc_remainder_ %= 10'000'000LL; }
    return events_->QueueEventParamUnk(MEMediaSample, GUID_NULL, S_OK, sample.Get());
}
HRESULT MediaStream::SetStreamState(const MF_STREAM_STATE state) {
    std::scoped_lock lock(mutex_);
    if (shutdown_) return MF_E_SHUTDOWN;
    if (state != MF_STREAM_STATE_STOPPED && state != MF_STREAM_STATE_RUNNING && state != MF_STREAM_STATE_PAUSED) return MF_E_INVALID_STATE_TRANSITION;
    if (state == MF_STREAM_STATE_PAUSED && state_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALID_STATE_TRANSITION;
    state_ = state;
    state_changed_.notify_all();
    return S_OK;
}
HRESULT MediaStream::GetStreamState(MF_STREAM_STATE* state) { if (!state) return E_POINTER; std::scoped_lock lock(mutex_); if (shutdown_) return MF_E_SHUTDOWN; *state = state_; return S_OK; }

} // namespace asc::win::source
