#pragma once

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <filesystem>
#include <mutex>
#include <string>
#include <vector>

namespace asc {

enum class LogLevel { trace, debug, info, warning, error };

struct LogEvent {
    std::chrono::system_clock::time_point timestamp;
    LogLevel level{LogLevel::info};
    std::string component;
    std::string code;
    std::string message;
    std::string details_json{"{}"};
};

class EventLog {
public:
    EventLog(std::filesystem::path directory, std::uint32_t retention_days = 14, std::size_t recent_limit = 20);
    void set_minimum_level(LogLevel level);
    void set_retention_days(std::uint32_t days);
    void write(LogLevel level, std::string component, std::string code, std::string message,
               std::string details_json = "{}");
    [[nodiscard]] std::vector<LogEvent> recent() const;
    void rotate();
    void clear();
    void export_to(const std::filesystem::path& destination) const;
    [[nodiscard]] const std::filesystem::path& directory() const noexcept { return directory_; }

private:
    [[nodiscard]] std::filesystem::path current_path() const;
    std::filesystem::path directory_;
    std::uint32_t retention_days_;
    std::size_t recent_limit_;
    LogLevel minimum_level_{LogLevel::info};
    mutable std::mutex mutex_;
    std::deque<LogEvent> recent_;
};

} // namespace asc
