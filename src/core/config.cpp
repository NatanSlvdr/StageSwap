#include "asc/core/config.hpp"

#include <charconv>
#include <cmath>
#include <fstream>
#include <sstream>
#include <stdexcept>
#ifdef _WIN32
#include <windows.h>
#endif

namespace asc {
namespace {

std::string escape_json(const std::string_view input) {
    std::string output;
    for (const char value : input) {
        switch (value) {
        case '\\': output += "\\\\"; break;
        case '"': output += "\\\""; break;
        case '\n': output += "\\n"; break;
        case '\r': output += "\\r"; break;
        case '\t': output += "\\t"; break;
        default: output += value; break;
        }
    }
    return output;
}

std::optional<std::size_t> value_start(const std::string_view json, const std::string_view key) {
    const auto token = '"' + std::string(key) + '"';
    const auto field = json.find(token);
    if (field == std::string_view::npos || json.find(token, field + token.size()) != std::string_view::npos) return std::nullopt;
    const auto colon = json.find(':', field + token.size());
    if (colon == std::string_view::npos) return std::nullopt;
    const auto value = json.find_first_not_of(" \t\r\n", colon + 1);
    return value == std::string_view::npos ? std::nullopt : std::optional<std::size_t>{value};
}

std::optional<std::string> string_value(const std::string_view json, const std::string_view key) {
    const auto start = value_start(json, key);
    if (!start || json[*start] != '"') return std::nullopt;
    std::string output;
    bool escaped = false;
    for (std::size_t index = *start + 1; index < json.size(); ++index) {
        const auto value = json[index];
        if (escaped) {
            if (value == 'n') output += '\n';
            else if (value == 'r') output += '\r';
            else if (value == 't') output += '\t';
            else if (value == '\\' || value == '"') output += value;
            else return std::nullopt;
            escaped = false;
        } else if (value == '\\') escaped = true;
        else if (value == '"') return output;
        else output += value;
    }
    return std::nullopt;
}

template <typename Value>
std::optional<Value> number_value(const std::string_view json, const std::string_view key) {
    const auto start = value_start(json, key);
    if (!start) return std::nullopt;
    auto end = *start;
    while (end < json.size() && (json[end] == '-' || json[end] == '+' || json[end] == '.' ||
           (json[end] >= '0' && json[end] <= '9') || json[end] == 'e' || json[end] == 'E')) ++end;
    if constexpr (std::is_integral_v<Value>) {
        Value output{};
        const auto result = std::from_chars(json.data() + *start, json.data() + end, output);
        return result.ec == std::errc{} && result.ptr == json.data() + end ? std::optional<Value>{output} : std::nullopt;
    } else {
        try { return static_cast<Value>(std::stod(std::string(json.substr(*start, end - *start)))); }
        catch (...) { return std::nullopt; }
    }
}

std::optional<bool> bool_value(const std::string_view json, const std::string_view key) {
    const auto start = value_start(json, key);
    if (!start) return std::nullopt;
    if (json.substr(*start, 4) == "true") return true;
    if (json.substr(*start, 5) == "false") return false;
    return std::nullopt;
}

bool has_field(const std::string_view json, const std::string_view key) {
    return json.find('"' + std::string(key) + '"') != std::string_view::npos;
}

bool looks_like_object(const std::string_view json) {
    const auto first = json.find_first_not_of(" \t\r\n");
    const auto last = json.find_last_not_of(" \t\r\n");
    return first != std::string_view::npos && json[first] == '{' && json[last] == '}';
}

std::string read_file(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    if (!stream) throw std::runtime_error("cannot open " + path.string());
    return {std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
}

const char* mode_name(const OutputMode mode) {
    switch (mode) {
    case OutputMode::automatic: return "automatic";
    case OutputMode::force_camera: return "force_camera";
    case OutputMode::force_screen: return "force_screen";
    }
    return "automatic";
}

} // namespace

ConfigStore::ConfigStore(std::filesystem::path directory) : directory_(std::move(directory)) {}
std::filesystem::path ConfigStore::config_path() const { return directory_ / "config.json"; }
std::filesystem::path ConfigStore::reference_path() const { return directory_ / "reference.png"; }
std::filesystem::path ConfigStore::comparison_path() const { return directory_ / "reference.gray"; }

std::string ConfigStore::serialize(const AppConfig& config) {
    std::ostringstream output;
    output << "{\n"
           << "  \"schema_version\": 2,\n"
           << "  \"selected_video_device_id\": \"" << escape_json(config.selected_video_device_id) << "\",\n"
           << "  \"reference_image_path\": \"" << escape_json(config.reference_image_path) << "\",\n"
           << "  \"similarity_threshold\": " << config.similarity_threshold << ",\n"
           << "  \"cursor_visible\": " << (config.cursor_visible ? "true" : "false") << ",\n"
           << "  \"start_with_windows\": " << (config.start_with_windows ? "true" : "false") << ",\n"
           << "  \"start_minimized\": " << (config.start_minimized ? "true" : "false") << ",\n"
           << "  \"start_automatically\": " << (config.start_automatically ? "true" : "false") << ",\n"
           << "  \"close_to_tray\": " << (config.close_to_tray ? "true" : "false") << ",\n"
           << "  \"show_notifications\": " << (config.show_notifications ? "true" : "false") << ",\n"
           << "  \"interface_language\": \"" << escape_json(config.interface_language) << "\",\n"
           << "  \"confirm_exit\": " << (config.confirm_exit ? "true" : "false") << ",\n"
           << "  \"output_mode\": \"" << mode_name(config.output_mode) << "\",\n"
           << "  \"placeholder_color_bgra\": " << (config.placeholder_color_bgra | 0xff000000u) << "\n"
           << "}\n";
    return output.str();
}

std::optional<AppConfig> ConfigStore::parse(const std::string_view json, std::string& error) {
    if (!looks_like_object(json) || !has_field(json, "schema_version")) {
        error = "configuration is not a JSON object with schema_version";
        return std::nullopt;
    }
    const auto schema = number_value<std::uint32_t>(json, "schema_version");
    if (!schema || (*schema != 1 && *schema != 2)) {
        error = "unsupported schema_version";
        return std::nullopt;
    }
    AppConfig config;
    if (has_field(json, "selected_video_device_id")) {
        const auto value = string_value(json, "selected_video_device_id");
        if (!value) { error = "invalid selected_video_device_id"; return std::nullopt; }
        config.selected_video_device_id = *value;
    }
    if (has_field(json, "reference_image_path")) {
        const auto value = string_value(json, "reference_image_path");
        if (!value) { error = "invalid reference_image_path"; return std::nullopt; }
        config.reference_image_path = *value;
    }
    const auto threshold_key = *schema == 1 ? "detection_threshold" : "similarity_threshold";
    if (has_field(json, threshold_key)) {
        const auto value = number_value<double>(json, threshold_key);
        if (!value || !std::isfinite(*value) || *value < 0.0 || *value > 1.0) {
            error = "invalid similarity threshold";
            return std::nullopt;
        }
        config.similarity_threshold = *value;
    }
    const auto read_bool = [&](const std::string_view key, bool& destination) {
        if (!has_field(json, key)) return true;
        const auto value = bool_value(json, key);
        if (!value) { error = "invalid boolean field: " + std::string(key); return false; }
        destination = *value;
        return true;
    };
    if (!read_bool("cursor_visible", config.cursor_visible) || !read_bool("start_with_windows", config.start_with_windows) ||
        !read_bool("start_minimized", config.start_minimized) || !read_bool("start_automatically", config.start_automatically) ||
        !read_bool("close_to_tray", config.close_to_tray) || !read_bool("show_notifications", config.show_notifications) ||
        !read_bool("confirm_exit", config.confirm_exit)) return std::nullopt;
    if (has_field(json, "interface_language")) {
        const auto value = string_value(json, "interface_language");
        if (!value) { error = "invalid interface_language"; return std::nullopt; }
        config.interface_language = *value;
    }
    if (has_field(json, "output_mode")) {
        const auto value = string_value(json, "output_mode");
        if (!value) { error = "invalid output_mode"; return std::nullopt; }
        if (*value == "automatic") config.output_mode = OutputMode::automatic;
        else if (*value == "force_camera") config.output_mode = OutputMode::force_camera;
        else if (*value == "force_screen") config.output_mode = OutputMode::force_screen;
        else { error = "invalid output_mode"; return std::nullopt; }
    }
    if (has_field(json, "placeholder_color_bgra")) {
        const auto value = number_value<std::uint32_t>(json, "placeholder_color_bgra");
        if (!value) { error = "invalid placeholder_color_bgra"; return std::nullopt; }
        config.placeholder_color_bgra = *value | 0xff000000u;
    }
    return config;
}

ConfigLoadResult ConfigStore::load() const {
    ConfigLoadResult result;
    if (!std::filesystem::exists(config_path())) return result;
    std::string error;
    try {
        if (auto parsed = parse(read_file(config_path()), error)) { result.config = std::move(*parsed); return result; }
    } catch (const std::exception& exception) { error = exception.what(); }
    result.warnings.push_back("Primary configuration invalid: " + error);
    const auto backup = directory_ / "config.backup.json";
    try {
        std::filesystem::copy_file(config_path(), directory_ / "config.invalid.json", std::filesystem::copy_options::overwrite_existing);
        if (std::filesystem::exists(backup)) {
            std::string backup_error;
            if (auto parsed = parse(read_file(backup), backup_error)) {
                result.config = std::move(*parsed);
                result.used_backup = true;
                result.warnings.push_back("Loaded last valid configuration backup");
            }
        }
    } catch (const std::exception& exception) { result.warnings.push_back(std::string("Could not preserve configuration: ") + exception.what()); }
    return result;
}

void ConfigStore::save(const AppConfig& config) const {
    std::filesystem::create_directories(directory_);
    const auto temporary = directory_ / "config.tmp.json";
    {
        std::ofstream stream(temporary, std::ios::binary | std::ios::trunc);
        if (!stream) throw std::runtime_error("cannot create temporary configuration");
        stream << serialize(config);
        if (!stream) throw std::runtime_error("cannot flush temporary configuration");
    }
    if (std::filesystem::exists(config_path())) {
        try { std::filesystem::copy_file(config_path(), directory_ / "config.backup.json", std::filesystem::copy_options::overwrite_existing); }
        catch (...) {}
    }
#ifdef _WIN32
    if (std::filesystem::exists(config_path())) {
        if (!ReplaceFileW(config_path().c_str(), temporary.c_str(), nullptr, REPLACEFILE_WRITE_THROUGH, nullptr, nullptr))
            throw std::runtime_error("cannot atomically replace configuration");
    } else if (!MoveFileExW(temporary.c_str(), config_path().c_str(), MOVEFILE_WRITE_THROUGH)) {
        throw std::runtime_error("cannot atomically install configuration");
    }
#else
    std::filesystem::rename(temporary, config_path());
#endif
}

} // namespace asc
