#include "shared_frame.hpp"
#include "asc/core/shared_frame_validation.hpp"

#include <sddl.h>
#include <algorithm>
#include <cstring>
#include <limits>
#include <stdexcept>

namespace asc::win {
namespace {
class ScopedHandle {
public:
    explicit ScopedHandle(const HANDLE value) noexcept : value_(value) {}
    ~ScopedHandle() { if (value_ && value_ != INVALID_HANDLE_VALUE) CloseHandle(value_); }
    ScopedHandle(const ScopedHandle&) = delete;
    ScopedHandle& operator=(const ScopedHandle&) = delete;
    [[nodiscard]] HANDLE get() const noexcept { return value_; }
private:
    HANDLE value_;
};
class ScopedLocalMemory {
public:
    explicit ScopedLocalMemory(HLOCAL value) noexcept : value_(value) {}
    ~ScopedLocalMemory() { if (value_) LocalFree(value_); }
    ScopedLocalMemory(const ScopedLocalMemory&) = delete;
    ScopedLocalMemory& operator=(const ScopedLocalMemory&) = delete;
private:
    HLOCAL value_;
};
bool write_all(const HANDLE pipe, const void* data, std::size_t bytes) {
    const auto* cursor = static_cast<const std::uint8_t*>(data);
    while (bytes > 0) {
        DWORD written = 0;
        const auto chunk = static_cast<DWORD>(std::min<std::size_t>(bytes, MAXDWORD));
        if (!WriteFile(pipe, cursor, chunk, &written, nullptr) || written == 0) return false;
        cursor += written; bytes -= written;
    }
    return true;
}
bool read_all(const HANDLE pipe, void* data, std::size_t bytes) {
    auto* cursor = static_cast<std::uint8_t*>(data);
    while (bytes > 0) {
        DWORD received = 0;
        const auto chunk = static_cast<DWORD>(std::min<std::size_t>(bytes, MAXDWORD));
        if (!ReadFile(pipe, cursor, chunk, &received, nullptr) || received == 0) return false;
        cursor += received; bytes -= received;
    }
    return true;
}
bool pipe_security(PSECURITY_DESCRIPTOR& descriptor, SECURITY_ATTRIBUTES& attributes) {
    descriptor = nullptr;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
            L"D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;LS)", SDDL_REVISION_1, &descriptor, nullptr)) return false;
    attributes = {static_cast<DWORD>(sizeof(SECURITY_ATTRIBUTES)), descriptor, FALSE};
    return true;
}
std::size_t checked_capacity(const Size size) {
    if (size.width == 0 || size.height == 0) throw std::invalid_argument("IPC frame capacity must be non-zero");
    const auto bytes = static_cast<std::uint64_t>(size.width) * size.height * 4;
    if (bytes > std::numeric_limits<std::uint32_t>::max() - sizeof(SharedFramePacket))
        throw std::invalid_argument("IPC frame capacity is too large");
    return static_cast<std::size_t>(bytes);
}
}

SharedFramePublisher::SharedFramePublisher(D3DDevice& d3d, const Size maximum_size, std::wstring pipe_name)
    : d3d_(d3d), slot_capacity_(checked_capacity(maximum_size)), pipe_name_(std::move(pipe_name)) {
    if (pipe_name_.empty()) throw std::invalid_argument("IPC pipe name must not be empty");
    latest_pixels_.reserve(slot_capacity_);
    staging_pixels_.reserve(slot_capacity_);
    server_thread_ = std::jthread([this](const std::stop_token stop) { server_loop(stop); });
}

SharedFramePublisher::~SharedFramePublisher() {
    server_thread_.request_stop();
    frame_ready_.notify_all();
    if (server_thread_.joinable()) {
        CancelSynchronousIo(server_thread_.native_handle());
        // Connecting once also releases a thread currently blocked in ConnectNamedPipe.
        const HANDLE wake = CreateFileW(pipe_name_.c_str(), GENERIC_READ, 0, nullptr, OPEN_EXISTING, 0, nullptr);
        if (wake != INVALID_HANDLE_VALUE) CloseHandle(wake);
        server_thread_.join();
    }
}

void SharedFramePublisher::prepare_readback(ReadbackSlot& slot, ID3D11Texture2D* source, const Size size) {
    if (slot.texture && slot.ready && slot.size == size) return;
    slot = {};
    D3D11_TEXTURE2D_DESC desc{};
    source->GetDesc(&desc);
    if (desc.Width != size.width || desc.Height != size.height || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM ||
        desc.ArraySize != 1 || desc.MipLevels != 1 || desc.SampleDesc.Count != 1)
        throw std::runtime_error("frame texture is not a supported BGRA surface");
    desc.BindFlags = 0;
    desc.MiscFlags = 0;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    check_hresult(d3d_.device()->CreateTexture2D(&desc, nullptr, &slot.texture), "Create virtual camera readback texture");
    D3D11_QUERY_DESC query_desc{D3D11_QUERY_EVENT, 0};
    check_hresult(d3d_.device()->CreateQuery(&query_desc, &slot.ready), "Create virtual camera readback query");
    slot.size = size;
}

bool SharedFramePublisher::collect_oldest_readback() {
    ReadbackSlot* oldest = nullptr;
    for (auto& slot : readbacks_) {
        if (slot.pending && (!oldest || slot.submission < oldest->submission)) oldest = &slot;
    }
    if (!oldest) return false;
    const auto ready = d3d_.context()->GetData(oldest->ready.Get(), nullptr, 0, 0);
    if (ready == S_FALSE) return false;
    check_hresult(ready, "Poll virtual camera readback");
    const auto frame_bytes = static_cast<std::size_t>(oldest->size.width) * oldest->size.height * 4;
    D3D11_MAPPED_SUBRESOURCE mapped{};
    const auto mapped_result = d3d_.context()->Map(oldest->texture.Get(), 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT, &mapped);
    if (mapped_result == DXGI_ERROR_WAS_STILL_DRAWING) return false;
    check_hresult(mapped_result, "Map virtual camera frame");
    staging_pixels_.resize(frame_bytes);
    const auto stride = oldest->size.width * 4;
    if (mapped.RowPitch < stride) {
        d3d_.context()->Unmap(oldest->texture.Get(), 0);
        throw std::runtime_error("virtual camera readback stride is too small");
    }
    for (std::uint32_t y = 0; y < oldest->size.height; ++y)
        std::memcpy(staging_pixels_.data() + static_cast<std::size_t>(y) * stride,
                    static_cast<const std::uint8_t*>(mapped.pData) + static_cast<std::size_t>(y) * mapped.RowPitch, stride);
    d3d_.context()->Unmap(oldest->texture.Get(), 0);
    {
        std::scoped_lock lock(frame_mutex_);
        ++latest_packet_.sequence;
        latest_packet_.width = oldest->size.width; latest_packet_.height = oldest->size.height; latest_packet_.stride = stride;
        latest_packet_.timestamp_100ns = oldest->timestamp_100ns;
        latest_packet_.frame_bytes = static_cast<std::uint32_t>(frame_bytes);
        latest_pixels_.swap(staging_pixels_);
    }
    frame_ready_.notify_all();
    oldest->pending = false;
    return true;
}

void SharedFramePublisher::publish(const VideoFrame& frame) {
    if (!frame.valid()) return;
    const auto frame_bytes = static_cast<std::uint64_t>(frame.size.width) * frame.size.height * 4;
    if (frame_bytes > slot_capacity_) throw std::runtime_error("frame exceeds IPC capacity");
    std::scoped_lock readback_lock(readback_mutex_);
    (void)collect_oldest_readback();
    ReadbackSlot* destination = nullptr;
    for (std::size_t offset = 0; offset < readbacks_.size(); ++offset) {
        auto& candidate = readbacks_[(next_readback_ + offset) % readbacks_.size()];
        if (!candidate.pending) { destination = &candidate; next_readback_ = (next_readback_ + offset + 1) % readbacks_.size(); break; }
    }
    // If the GPU is more than three frames behind, retain the last good frame instead of blocking this thread.
    if (!destination) return;
    prepare_readback(*destination, frame.texture.Get(), frame.size);
    d3d_.context()->CopyResource(destination->texture.Get(), frame.texture.Get());
    d3d_.context()->End(destination->ready.Get());
    destination->timestamp_100ns = frame.presentation_time_100ns;
    destination->submission = next_submission_++;
    destination->pending = true;
}

void SharedFramePublisher::reset_device() {
    std::scoped_lock lock(readback_mutex_);
    readbacks_ = {};
    next_submission_ = 1;
    next_readback_ = 0;
}

void SharedFramePublisher::invalidate() {
    std::scoped_lock readback_lock(readback_mutex_);
    for (auto& slot : readbacks_) slot.pending = false;
    next_submission_ = 1;
    {
        std::scoped_lock lock(frame_mutex_);
        ++latest_packet_.sequence;
        latest_packet_.width = latest_packet_.height = latest_packet_.stride = latest_packet_.frame_bytes = 0;
        latest_pixels_.clear();
    }
    frame_ready_.notify_all();
}

void SharedFramePublisher::server_loop(const std::stop_token stop) {
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    SECURITY_ATTRIBUTES security{};
    if (!pipe_security(descriptor, security)) return;
    const ScopedLocalMemory descriptor_memory(descriptor);
    try {
        std::vector<std::uint8_t> pixels;
        pixels.reserve(slot_capacity_);
        while (!stop.stop_requested()) {
            const ScopedHandle pipe(CreateNamedPipeW(
                pipe_name_.c_str(), PIPE_ACCESS_OUT,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1, static_cast<DWORD>(slot_capacity_ + sizeof(SharedFramePacket)), 64 * 1024, 0, &security));
            if (pipe.get() == INVALID_HANDLE_VALUE) break;
            const bool connected = ConnectNamedPipe(pipe.get(), nullptr) || GetLastError() == ERROR_PIPE_CONNECTED;
            if (!connected) { if (!stop.stop_requested()) Sleep(250); continue; }
            std::uint64_t sent_sequence = 0;
            while (!stop.stop_requested()) {
                SharedFramePacket packet;
                {
                    std::unique_lock lock(frame_mutex_);
                    frame_ready_.wait(lock, stop, [&] { return latest_packet_.sequence != sent_sequence; });
                    if (stop.stop_requested()) break;
                    packet = latest_packet_; pixels = latest_pixels_;
                }
                if (!write_all(pipe.get(), &packet, sizeof(packet)) || !write_all(pipe.get(), pixels.data(), pixels.size())) break;
                sent_sequence = packet.sequence;
            }
            DisconnectNamedPipe(pipe.get());
        }
    } catch (...) {
        // A background IPC failure must degrade to the media source's placeholder, not terminate the host process.
    }
}

SharedFrameReader::~SharedFrameReader() {
    reader_thread_.request_stop();
    if (reader_thread_.joinable()) { CancelSynchronousIo(reader_thread_.native_handle()); reader_thread_.join(); }
}

void SharedFrameReader::configure(std::wstring pipe_name) {
    if (reader_thread_.joinable()) {
        reader_thread_.request_stop(); CancelSynchronousIo(reader_thread_.native_handle()); reader_thread_.join();
    }
    pipe_name_ = std::move(pipe_name);
    if (!pipe_name_.empty()) reader_thread_ = std::jthread([this](const std::stop_token stop) { reader_loop(stop); });
}

void SharedFrameReader::reader_loop(const std::stop_token stop) {
    constexpr std::uint64_t maximum_frame_bytes = 1920ull * 1080 * 4;
    const auto invalidate_latest = [this] {
        std::scoped_lock lock(mutex_);
        latest_ = {};
        received_tick_ = 0;
    };
    try {
        receive_buffer_.reserve(static_cast<std::size_t>(maximum_frame_bytes));
        while (!stop.stop_requested()) {
            if (!WaitNamedPipeW(pipe_name_.c_str(), 500)) { if (!stop.stop_requested()) Sleep(250); continue; }
            const ScopedHandle pipe(CreateFileW(pipe_name_.c_str(), GENERIC_READ, 0, nullptr, OPEN_EXISTING, 0, nullptr));
            if (pipe.get() == INVALID_HANDLE_VALUE) { Sleep(250); continue; }
            std::uint64_t received_sequence = 0;
            while (!stop.stop_requested()) {
                SharedFramePacket packet;
                if (!read_all(pipe.get(), &packet, sizeof(packet))) break;
                if (packet.magic != shared_frame_magic || packet.version != shared_frame_version ||
                    packet.sequence == 0 || packet.sequence <= received_sequence) {
                    invalidate_latest();
                    break;
                }
                received_sequence = packet.sequence;
                const auto metadata_status = validate_shared_frame_metadata(
                    packet.width, packet.height, packet.stride, packet.frame_bytes, maximum_frame_bytes);
                if (metadata_status == SharedFrameMetadataStatus::invalid) {
                    invalidate_latest();
                    break;
                }
                if (metadata_status == SharedFrameMetadataStatus::invalidation) {
                    invalidate_latest();
                    continue;
                }
                receive_buffer_.resize(packet.frame_bytes);
                if (!read_all(pipe.get(), receive_buffer_.data(), receive_buffer_.size())) break;
                std::scoped_lock lock(mutex_);
                latest_.sequence = packet.sequence; latest_.size = {packet.width, packet.height}; latest_.stride = packet.stride;
                latest_.timestamp_100ns = packet.timestamp_100ns;
                latest_.bgra.swap(receive_buffer_); received_tick_ = GetTickCount64();
            }
        }
    } catch (...) {
        invalidate_latest();
    }
}

bool SharedFrameReader::read_latest(CpuSharedFrame& output) {
    std::scoped_lock lock(mutex_);
    const auto now = GetTickCount64();
    if (received_tick_ == 0 || now < received_tick_ || now - received_tick_ > 2000 || latest_.bgra.empty()) return false;
    if (output.sequence != latest_.sequence) output = latest_;
    return true;
}

} // namespace asc::win
