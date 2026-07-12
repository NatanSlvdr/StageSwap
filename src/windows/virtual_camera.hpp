#pragma once

#include "common.hpp"
#include "media_source/ids.hpp"

#include <mfvirtualcamera.h>
#include <string>
#include <cstdint>
#include <atomic>
#include <mutex>

namespace asc::win {

class VirtualCamera {
public:
    VirtualCamera() = default;
    ~VirtualCamera();
    void start(const std::wstring& pipe_name, std::uint32_t placeholder_color);
    void stop() noexcept;
    void restart();
    static void remove_registration();
    [[nodiscard]] bool running() const noexcept { return running_.load(); }
    [[nodiscard]] const std::wstring& symbolic_link() const noexcept { return symbolic_link_; }

private:
    void start_unlocked(const std::wstring& pipe_name, std::uint32_t placeholder_color);
    void stop_unlocked() noexcept;
    ComPtr<IMFVirtualCamera> camera_;
    ComPtr<IMFAsyncCallback> callback_;
    std::wstring symbolic_link_;
    std::wstring pipe_name_;
    std::uint32_t placeholder_color_{0xff171719u};
    std::atomic<bool> running_{false};
    std::atomic<std::uint64_t> event_generation_{0};
    std::mutex mutex_;
};

} // namespace asc::win
