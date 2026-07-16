#pragma once

#include "video_frame.hpp"

#include <condition_variable>
#include <cstddef>
#include <cstdint>
#include <mutex>
#include <stop_token>
#include <string>
#include <thread>
#include <vector>

namespace asc::win {

inline constexpr std::uint32_t shared_frame_magic = 0x41534346; // ASCF
inline constexpr std::uint32_t shared_frame_version = 1;

#pragma pack(push, 1)
struct SharedFramePacket {
    std::uint32_t magic{shared_frame_magic};
    std::uint32_t version{shared_frame_version};
    std::uint64_t sequence{0};
    std::uint32_t width{0};
    std::uint32_t height{0};
    std::uint32_t stride{0};
    std::int64_t timestamp_100ns{0};
    std::uint32_t frame_bytes{0};
};
#pragma pack(pop)
static_assert(sizeof(SharedFramePacket) == 40);

struct CpuSharedFrame {
    std::uint64_t sequence{0};
    Size size;
    std::uint32_t stride{0};
    std::int64_t timestamp_100ns{0};
    std::vector<std::uint8_t> bgra;
};

class SharedFramePublisher {
public:
    SharedFramePublisher(Size maximum_size, std::wstring pipe_name);
    ~SharedFramePublisher();
    SharedFramePublisher(const SharedFramePublisher&) = delete;
    SharedFramePublisher& operator=(const SharedFramePublisher&) = delete;
    void publish(const VideoFrame& frame);
    void invalidate();
    [[nodiscard]] const std::wstring& pipe_name() const noexcept { return pipe_name_; }

private:
    void server_loop(std::stop_token stop);
    std::size_t slot_capacity_;
    std::wstring pipe_name_;
    std::mutex frame_mutex_;
    std::condition_variable_any frame_ready_;
    SharedFramePacket latest_packet_;
    std::vector<std::uint8_t> latest_pixels_;
    std::jthread server_thread_;
};

class SharedFrameReader {
public:
    SharedFrameReader() = default;
    ~SharedFrameReader();
    SharedFrameReader(const SharedFrameReader&) = delete;
    SharedFrameReader& operator=(const SharedFrameReader&) = delete;
    void configure(std::wstring pipe_name);
    [[nodiscard]] bool read_latest(CpuSharedFrame& output);

private:
    void reader_loop(std::stop_token stop);
    std::wstring pipe_name_;
    mutable std::mutex mutex_;
    CpuSharedFrame latest_;
    std::vector<std::uint8_t> receive_buffer_;
    std::uint64_t received_tick_{0};
    std::jthread reader_thread_;
};

} // namespace asc::win
