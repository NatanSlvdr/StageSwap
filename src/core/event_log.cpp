#include "asc/core/event_log.hpp"

#include <fstream>
#include <iomanip>
#include <sstream>
#include <algorithm>
#include <stdexcept>

namespace asc {
namespace {
std::string escape(const std::string_view input) {
    std::string out;
    for (const char c : input) {
        if (c == '\\') out += "\\\\";
        else if (c == '"') out += "\\\"";
        else if (c == '\n') out += "\\n";
        else if (c == '\r') out += "\\r";
        else if (static_cast<unsigned char>(c) >= 0x20) out += c;
    }
    return out;
}
const char* name(const LogLevel level) {
    switch (level) { case LogLevel::trace: return "trace"; case LogLevel::debug: return "debug"; case LogLevel::info: return "info";
    case LogLevel::warning: return "warning"; case LogLevel::error: return "error"; }
    return "info";
}
std::tm local_time(const std::time_t value) {
    std::tm result{};
#ifdef _WIN32
    localtime_s(&result, &value);
#else
    localtime_r(&value, &result);
#endif
    return result;
}
}

EventLog::EventLog(std::filesystem::path directory, const std::uint32_t retention_days, const std::size_t recent_limit)
    : directory_(std::move(directory)), retention_days_(retention_days), recent_limit_(recent_limit) {
    std::filesystem::create_directories(directory_);
    rotate();
}
void EventLog::set_minimum_level(const LogLevel level) { std::scoped_lock lock(mutex_); minimum_level_ = level; }
void EventLog::set_retention_days(const std::uint32_t days) {
    std::scoped_lock lock(mutex_);
    retention_days_ = days;
    const auto cutoff = std::filesystem::file_time_type::clock::now() - std::chrono::hours{24 * retention_days_};
    std::error_code error;
    for (const auto& entry : std::filesystem::directory_iterator(directory_, error))
        if (entry.is_regular_file() && entry.path().extension() == ".jsonl" && entry.last_write_time() < cutoff)
            std::filesystem::remove(entry.path(), error);
}

std::filesystem::path EventLog::current_path() const {
    const auto now = std::chrono::system_clock::to_time_t(std::chrono::system_clock::now());
    const auto tm = local_time(now);
    std::ostringstream filename;
    filename << "automatic-screen-camera-" << std::put_time(&tm, "%Y-%m-%d") << ".jsonl";
    return directory_ / filename.str();
}

void EventLog::write(const LogLevel level, std::string component, std::string code, std::string message, std::string details_json) {
    std::scoped_lock lock(mutex_);
    if (level < minimum_level_) return;
    LogEvent event{std::chrono::system_clock::now(), level, std::move(component), std::move(code), std::move(message), std::move(details_json)};
    recent_.push_front(event);
    while (recent_.size() > recent_limit_) recent_.pop_back();
    const auto milliseconds = std::chrono::duration_cast<std::chrono::milliseconds>(event.timestamp.time_since_epoch()) % 1000;
    const auto time = std::chrono::system_clock::to_time_t(event.timestamp);
    const auto tm = local_time(time);
    std::ofstream stream(current_path(), std::ios::binary | std::ios::app);
    stream << "{\"timestamp\":\"" << std::put_time(&tm, "%Y-%m-%dT%H:%M:%S") << '.'
           << std::setfill('0') << std::setw(3) << milliseconds.count() << "\",\"level\":\"" << name(level)
           << "\",\"component\":\"" << escape(event.component) << "\",\"event_code\":\"" << escape(event.code)
           << "\",\"message\":\"" << escape(event.message) << "\",\"details\":" << event.details_json << "}\n";
}

std::vector<LogEvent> EventLog::recent() const {
    std::scoped_lock lock(mutex_);
    return {recent_.begin(), recent_.end()};
}

void EventLog::rotate() {
    const auto cutoff = std::filesystem::file_time_type::clock::now() - std::chrono::hours{24 * retention_days_};
    std::error_code error;
    for (const auto& entry : std::filesystem::directory_iterator(directory_, error)) {
        if (entry.is_regular_file() && entry.path().extension() == ".jsonl" && entry.last_write_time() < cutoff)
            std::filesystem::remove(entry.path(), error);
    }
}

void EventLog::clear() {
    std::scoped_lock lock(mutex_);
    std::error_code error;
    for (const auto& entry : std::filesystem::directory_iterator(directory_, error))
        if (entry.is_regular_file() && entry.path().extension() == ".jsonl") std::filesystem::remove(entry.path(), error);
    recent_.clear();
}

void EventLog::export_to(const std::filesystem::path& destination) const {
    std::scoped_lock lock(mutex_);
    std::ofstream output(destination, std::ios::binary | std::ios::trunc);
    if (!output) throw std::runtime_error("cannot create log export");
    std::error_code error;
    std::vector<std::filesystem::path> files;
    for (const auto& entry : std::filesystem::directory_iterator(directory_, error))
        if (entry.is_regular_file() && entry.path().extension() == ".jsonl" && entry.path() != destination) files.push_back(entry.path());
    std::sort(files.begin(), files.end());
    for (const auto& file : files) {
        std::ifstream input(file, std::ios::binary);
        output << input.rdbuf();
    }
    if (!output) throw std::runtime_error("cannot write log export");
}

} // namespace asc
