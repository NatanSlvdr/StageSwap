#include "shared_frame.hpp"

#include <sddl.h>
#include <algorithm>
#include <cstring>

namespace asc::win {
namespace {
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
SECURITY_ATTRIBUTES pipe_security(PSECURITY_DESCRIPTOR& descriptor) {
    descriptor = nullptr;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(
        L"D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;LS)", SDDL_REVISION_1, &descriptor, nullptr);
    return {sizeof(SECURITY_ATTRIBUTES), descriptor, FALSE};
}
}

SharedFramePublisher::SharedFramePublisher(D3DDevice& d3d, const Size maximum_size, std::wstring pipe_name)
    : d3d_(d3d), slot_capacity_(static_cast<std::size_t>(maximum_size.width) * maximum_size.height * 4),
      pipe_name_(std::move(pipe_name)), server_thread_([this](const std::stop_token stop) { server_loop(stop); }) {}

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

void SharedFramePublisher::publish(const VideoFrame& frame) {
    if (!frame.valid()) return;
    const auto frame_bytes = static_cast<std::size_t>(frame.size.width) * frame.size.height * 4;
    if (frame_bytes > slot_capacity_) throw std::runtime_error("frame exceeds IPC capacity");
    if (!staging_ || staging_size_ != frame.size) {
        D3D11_TEXTURE2D_DESC desc{}; frame.texture->GetDesc(&desc);
        desc.BindFlags = 0; desc.MiscFlags = 0; desc.Usage = D3D11_USAGE_STAGING; desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        check_hresult(d3d_.device()->CreateTexture2D(&desc, nullptr, &staging_), "Create virtual camera readback texture");
        staging_size_ = frame.size;
    }
    d3d_.context()->CopyResource(staging_.Get(), frame.texture.Get());
    D3D11_MAPPED_SUBRESOURCE mapped{};
    check_hresult(d3d_.context()->Map(staging_.Get(), 0, D3D11_MAP_READ, 0, &mapped), "Map virtual camera frame");
    staging_pixels_.resize(frame_bytes);
    const auto stride = frame.size.width * 4;
    for (std::uint32_t y = 0; y < frame.size.height; ++y)
        std::memcpy(staging_pixels_.data() + static_cast<std::size_t>(y) * stride,
                    static_cast<const std::uint8_t*>(mapped.pData) + static_cast<std::size_t>(y) * mapped.RowPitch, stride);
    d3d_.context()->Unmap(staging_.Get(), 0);
    {
        std::scoped_lock lock(frame_mutex_);
        ++latest_packet_.sequence;
        latest_packet_.width = frame.size.width; latest_packet_.height = frame.size.height; latest_packet_.stride = stride;
        latest_packet_.timestamp_100ns = frame.presentation_time_100ns;
        latest_packet_.frame_bytes = static_cast<std::uint32_t>(frame_bytes);
        latest_pixels_.swap(staging_pixels_);
    }
    frame_ready_.notify_all();
}

void SharedFramePublisher::invalidate() {
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
    auto security = pipe_security(descriptor);
    while (!stop.stop_requested()) {
        const HANDLE pipe = CreateNamedPipeW(pipe_name_.c_str(), PIPE_ACCESS_OUT, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                                             1, static_cast<DWORD>(slot_capacity_ + sizeof(SharedFramePacket)), 64 * 1024, 0,
                                             descriptor ? &security : nullptr);
        if (pipe == INVALID_HANDLE_VALUE) break;
        const bool connected = ConnectNamedPipe(pipe, nullptr) || GetLastError() == ERROR_PIPE_CONNECTED;
        if (!connected) { CloseHandle(pipe); if (!stop.stop_requested()) Sleep(250); continue; }
        std::uint64_t sent_sequence = 0;
        std::vector<std::uint8_t> pixels;
        while (!stop.stop_requested()) {
            SharedFramePacket packet;
            {
                std::unique_lock lock(frame_mutex_);
                frame_ready_.wait(lock, stop, [&] { return latest_packet_.sequence != sent_sequence; });
                if (stop.stop_requested()) break;
                packet = latest_packet_; pixels = latest_pixels_;
            }
            if (!write_all(pipe, &packet, sizeof(packet)) || !write_all(pipe, pixels.data(), pixels.size())) break;
            sent_sequence = packet.sequence;
        }
        DisconnectNamedPipe(pipe); CloseHandle(pipe);
    }
    if (descriptor) LocalFree(descriptor);
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
    while (!stop.stop_requested()) {
        if (!WaitNamedPipeW(pipe_name_.c_str(), 500)) { if (!stop.stop_requested()) Sleep(250); continue; }
        const HANDLE pipe = CreateFileW(pipe_name_.c_str(), GENERIC_READ, 0, nullptr, OPEN_EXISTING, 0, nullptr);
        if (pipe == INVALID_HANDLE_VALUE) { Sleep(250); continue; }
        while (!stop.stop_requested()) {
            SharedFramePacket packet;
            if (!read_all(pipe, &packet, sizeof(packet))) break;
            if (packet.magic != shared_frame_magic || packet.version != 1) break;
            if (packet.frame_bytes == 0) {
                std::scoped_lock lock(mutex_); latest_ = {}; received_tick_ = 0; continue;
            }
            if (packet.width == 0 || packet.height == 0 ||
                packet.stride != packet.width * 4 || packet.frame_bytes != packet.stride * packet.height ||
                packet.frame_bytes > 1920u * 1080u * 4u) break;
            receive_buffer_.resize(packet.frame_bytes);
            if (!read_all(pipe, receive_buffer_.data(), receive_buffer_.size())) break;
            std::scoped_lock lock(mutex_);
            latest_.size = {packet.width, packet.height}; latest_.stride = packet.stride; latest_.timestamp_100ns = packet.timestamp_100ns;
            latest_.bgra.swap(receive_buffer_); received_tick_ = GetTickCount64();
        }
        CloseHandle(pipe);
    }
}

bool SharedFrameReader::read_latest(CpuSharedFrame& output) {
    std::scoped_lock lock(mutex_);
    const auto now = GetTickCount64();
    if (received_tick_ == 0 || now < received_tick_ || now - received_tick_ > 2000 || latest_.bgra.empty()) return false;
    output = latest_;
    return true;
}

} // namespace asc::win
