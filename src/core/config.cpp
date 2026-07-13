#include "asc/core/config.hpp"

#include <algorithm>
#include <array>
#include <charconv>
#include <cmath>
#include <fstream>
#include <limits>
#include <sstream>
#include <stdexcept>
#ifdef _WIN32
#include <windows.h>
#endif

namespace asc {
namespace {

std::string escape_json(const std::string_view input) {
    std::string output;
    output.reserve(input.size() + 8);
    for (const char c : input) {
        switch (c) {
        case '\\': output += "\\\\"; break;
        case '"': output += "\\\""; break;
        case '\n': output += "\\n"; break;
        case '\r': output += "\\r"; break;
        case '\t': output += "\\t"; break;
        default: output += c; break;
        }
    }
    return output;
}

std::optional<std::size_t> value_start(std::string_view json, std::string_view key);

std::optional<std::string> string_value(const std::string_view json, const std::string_view key) {
    const auto start = value_start(json, key);
    if (!start || json[*start] != '"') return std::nullopt;
    std::string result;
    bool escaped = false;
    for (std::size_t i = *start + 1; i < json.size(); ++i) {
        const char c = json[i];
        if (escaped) {
            switch (c) {
            case 'n': result += '\n'; break;
            case 'r': result += '\r'; break;
            case 't': result += '\t'; break;
            case '\\': result += '\\'; break;
            case '"': result += '"'; break;
            default: return std::nullopt;
            }
            escaped = false;
        } else if (c == '\\') escaped = true;
        else if (c == '"') return result;
        else result += c;
    }
    return std::nullopt;
}

template <typename T>
std::optional<T> number_value(const std::string_view json, const std::string_view key) {
    const auto start = value_start(json, key);
    if (!start) return std::nullopt;
    auto end = *start;
    while (end < json.size() && (json[end] == '-' || json[end] == '+' || json[end] == '.' ||
           (json[end] >= '0' && json[end] <= '9') || json[end] == 'e' || json[end] == 'E')) ++end;
    T result{};
    if constexpr (std::is_integral_v<T>) {
        const auto parsed = std::from_chars(json.data() + *start, json.data() + end, result);
        if (parsed.ec != std::errc{} || parsed.ptr != json.data() + end) return std::nullopt;
    } else {
        try { result = static_cast<T>(std::stod(std::string(json.substr(*start, end - *start)))); }
        catch (...) { return std::nullopt; }
    }
    return result;
}

std::optional<bool> bool_value(const std::string_view json, const std::string_view key) {
    const auto start = value_start(json, key);
    if (!start) return std::nullopt;
    if (json.substr(*start, 4) == "true") return true;
    if (json.substr(*start, 5) == "false") return false;
    return std::nullopt;
}

void skip_json_whitespace(const std::string_view json, std::size_t& position) {
    while (position < json.size() && (json[position] == ' ' || json[position] == '\t' ||
                                      json[position] == '\r' || json[position] == '\n')) ++position;
}

bool consume_json_string(const std::string_view json, std::size_t& position) {
    if (position >= json.size() || json[position] != '"') return false;
    ++position;
    while (position < json.size()) {
        const auto value = static_cast<unsigned char>(json[position++]);
        if (value == '"') return true;
        if (value < 0x20) return false;
        if (value != '\\') continue;
        if (position >= json.size()) return false;
        const char escape = json[position++];
        if (escape == 'u') {
            for (int digit = 0; digit < 4; ++digit) {
                if (position >= json.size()) return false;
                const char hex = json[position++];
                if (!((hex >= '0' && hex <= '9') || (hex >= 'a' && hex <= 'f') || (hex >= 'A' && hex <= 'F'))) return false;
            }
        } else if (std::string_view{"\"\\/bfnrt"}.find(escape) == std::string_view::npos) return false;
    }
    return false;
}

bool consume_json_number(const std::string_view json, std::size_t& position) {
    if (position < json.size() && json[position] == '-') ++position;
    if (position >= json.size()) return false;
    if (json[position] == '0') ++position;
    else {
        if (json[position] < '1' || json[position] > '9') return false;
        while (position < json.size() && json[position] >= '0' && json[position] <= '9') ++position;
    }
    if (position < json.size() && json[position] == '.') {
        ++position;
        const auto fraction_start = position;
        while (position < json.size() && json[position] >= '0' && json[position] <= '9') ++position;
        if (position == fraction_start) return false;
    }
    if (position < json.size() && (json[position] == 'e' || json[position] == 'E')) {
        ++position;
        if (position < json.size() && (json[position] == '+' || json[position] == '-')) ++position;
        const auto exponent_start = position;
        while (position < json.size() && json[position] >= '0' && json[position] <= '9') ++position;
        if (position == exponent_start) return false;
    }
    return true;
}

bool consume_json_primitive(const std::string_view json, std::size_t& position) {
    if (position >= json.size()) return false;
    if (json[position] == '"') return consume_json_string(json, position);
    for (const auto literal : {std::string_view{"true"}, std::string_view{"false"}, std::string_view{"null"}}) {
        if (json.substr(position, literal.size()) == literal) {
            position += literal.size();
            return true;
        }
    }
    return consume_json_number(json, position);
}

bool valid_flat_json_object(const std::string_view json) {
    std::size_t position = 0;
    skip_json_whitespace(json, position);
    if (position >= json.size() || json[position++] != '{') return false;
    skip_json_whitespace(json, position);
    if (position < json.size() && json[position] == '}') {
        ++position;
        skip_json_whitespace(json, position);
        return position == json.size();
    }
    while (position < json.size()) {
        if (!consume_json_string(json, position)) return false;
        skip_json_whitespace(json, position);
        if (position >= json.size() || json[position++] != ':') return false;
        skip_json_whitespace(json, position);
        if (!consume_json_primitive(json, position)) return false;
        skip_json_whitespace(json, position);
        if (position >= json.size()) return false;
        if (json[position] == '}') {
            ++position;
            skip_json_whitespace(json, position);
            return position == json.size();
        }
        if (json[position++] != ',') return false;
        skip_json_whitespace(json, position);
    }
    return false;
}

std::optional<std::size_t> value_start(const std::string_view json, const std::string_view key) {
    std::size_t position = 0;
    skip_json_whitespace(json, position);
    if (position >= json.size() || json[position++] != '{') return std::nullopt;
    skip_json_whitespace(json, position);
    while (position < json.size() && json[position] != '}') {
        if (json[position] != '"') return std::nullopt;
        const auto key_start = position + 1;
        if (!consume_json_string(json, position)) return std::nullopt;
        const auto key_end = position - 1;
        skip_json_whitespace(json, position);
        if (position >= json.size() || json[position++] != ':') return std::nullopt;
        skip_json_whitespace(json, position);
        const auto candidate_value_start = position;
        if (json.substr(key_start, key_end - key_start) == key) return candidate_value_start;
        if (!consume_json_primitive(json, position)) return std::nullopt;
        skip_json_whitespace(json, position);
        if (position < json.size() && json[position] == ',') {
            ++position;
            skip_json_whitespace(json, position);
        } else break;
    }
    return std::nullopt;
}

std::size_t field_count(const std::string_view json, const std::string_view key) {
    std::size_t count = 0;
    std::size_t position = 0;
    skip_json_whitespace(json, position);
    if (position >= json.size() || json[position++] != '{') return 0;
    skip_json_whitespace(json, position);
    while (position < json.size() && json[position] != '}') {
        if (json[position] != '"') return count;
        const auto key_start = position + 1;
        if (!consume_json_string(json, position)) return count;
        const auto key_end = position - 1;
        if (json.substr(key_start, key_end - key_start) == key) ++count;
        skip_json_whitespace(json, position);
        if (position >= json.size() || json[position++] != ':') return count;
        skip_json_whitespace(json, position);
        if (!consume_json_primitive(json, position)) return count;
        skip_json_whitespace(json, position);
        if (position < json.size() && json[position] == ',') {
            ++position;
            skip_json_whitespace(json, position);
        } else break;
    }
    return count;
}

template <typename T, std::size_t N>
bool validate_number_fields(const std::string_view json, const std::array<std::string_view, N>& keys,
                            std::string& error) {
    for (const auto key : keys) {
        const auto count = field_count(json, key);
        if (count > 1) {
            error = "duplicate configuration field: " + std::string(key);
            return false;
        }
        if (count == 1 && !number_value<T>(json, key)) {
            error = "invalid numeric configuration field: " + std::string(key);
            return false;
        }
    }
    return true;
}

template <std::size_t N>
bool validate_uint32_fields(const std::string_view json, const std::array<std::string_view, N>& keys,
                            std::string& error) {
    for (const auto key : keys) {
        const auto count = field_count(json, key);
        if (count > 1) {
            error = "duplicate configuration field: " + std::string(key);
            return false;
        }
        if (count == 0) continue;
        const auto value = number_value<std::uint64_t>(json, key);
        if (!value || *value > std::numeric_limits<std::uint32_t>::max()) {
            error = "invalid unsigned configuration field: " + std::string(key);
            return false;
        }
    }
    return true;
}

template <std::size_t N>
bool validate_string_fields(const std::string_view json, const std::array<std::string_view, N>& keys,
                            std::string& error) {
    for (const auto key : keys) {
        const auto count = field_count(json, key);
        if (count > 1) {
            error = "duplicate configuration field: " + std::string(key);
            return false;
        }
        if (count == 1 && !string_value(json, key)) {
            error = "invalid string configuration field: " + std::string(key);
            return false;
        }
    }
    return true;
}

template <std::size_t N>
bool validate_bool_fields(const std::string_view json, const std::array<std::string_view, N>& keys,
                          std::string& error) {
    for (const auto key : keys) {
        const auto count = field_count(json, key);
        if (count > 1) {
            error = "duplicate configuration field: " + std::string(key);
            return false;
        }
        if (count == 1 && !bool_value(json, key)) {
            error = "invalid boolean configuration field: " + std::string(key);
            return false;
        }
    }
    return true;
}

std::string read_file(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    if (!stream) throw std::runtime_error("cannot open " + path.string());
    return {std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
}

const char* output_mode_name(const OutputMode mode) {
    switch (mode) { case OutputMode::automatic: return "automatic"; case OutputMode::force_camera: return "force_camera"; case OutputMode::force_screen: return "force_screen"; }
    return "automatic";
}

const char* missing_name(const MissingReferenceBehavior behavior) {
    switch (behavior) { case MissingReferenceBehavior::use_camera: return "use_camera"; case MissingReferenceBehavior::keep_current: return "keep_current";
    case MissingReferenceBehavior::use_last_screen: return "use_last_screen"; case MissingReferenceBehavior::use_placeholder: return "use_placeholder"; }
    return "use_camera";
}

const char* scaling_name(const ScalingMode mode) {
    switch (mode) { case ScalingMode::fit: return "fit"; case ScalingMode::fill: return "fill"; case ScalingMode::stretch: return "stretch"; }
    return "fit";
}

const char* log_level_name(const LogLevel level) {
    switch (level) { case LogLevel::trace: return "trace"; case LogLevel::debug: return "debug"; case LogLevel::info: return "info";
    case LogLevel::warning: return "warning"; case LogLevel::error: return "error"; }
    return "info";
}

} // namespace

ConfigStore::ConfigStore(std::filesystem::path directory) : directory_(std::move(directory)) {}
std::filesystem::path ConfigStore::config_path() const { return directory_ / "config.json"; }
std::filesystem::path ConfigStore::reference_path() const { return directory_ / "reference.png"; }
std::filesystem::path ConfigStore::comparison_path() const { return directory_ / "reference-thumbnail.gray"; }

std::string ConfigStore::serialize(const AppConfig& c) {
    std::ostringstream out;
    out << "{\n"
        << "  \"schema_version\": " << c.schema_version << ",\n"
        << "  \"selected_video_device_id\": \"" << escape_json(c.selected_video_device_id) << "\",\n"
        << "  \"preferred_input_width\": " << c.preferred_input_size.width << ",\n"
        << "  \"preferred_input_height\": " << c.preferred_input_size.height << ",\n"
        << "  \"preferred_input_fps\": " << c.preferred_input_fps << ",\n"
        << "  \"reference_image_path\": \"" << escape_json(c.reference_image_path) << "\",\n"
        << "  \"detection_threshold\": " << c.detector.threshold << ",\n"
        << "  \"detection_interval_ms\": " << c.detection_interval.count() << ",\n"
        << "  \"matches_required\": " << c.detector.matches_required << ",\n"
        << "  \"mismatches_required\": " << c.detector.mismatches_required << ",\n"
        << "  \"full_scan_interval_seconds\": " << c.full_scan_interval.count() << ",\n"
        << "  \"reassignment_confirmations\": " << c.monitor_tracker.confirmations_required << ",\n"
        << "  \"reassignment_margin\": " << c.monitor_tracker.reassignment_margin << ",\n"
        << "  \"output_width\": " << c.output_size.width << ",\n"
        << "  \"output_height\": " << c.output_size.height << ",\n"
        << "  \"output_fps\": " << c.output_fps << ",\n"
        << "  \"fade_duration_ms\": " << c.fade_duration.count() << ",\n"
        << "  \"cursor_visible\": " << (c.cursor_visible ? "true" : "false") << ",\n"
        << "  \"start_with_windows\": " << (c.start_with_windows ? "true" : "false") << ",\n"
        << "  \"start_minimized\": " << (c.start_minimized ? "true" : "false") << ",\n"
        << "  \"start_automatically\": " << (c.start_automatically ? "true" : "false") << ",\n"
        << "  \"close_to_tray\": " << (c.close_to_tray ? "true" : "false") << ",\n"
        << "  \"show_notifications\": " << (c.show_notifications ? "true" : "false") << ",\n"
        << "  \"interface_language\": \"" << escape_json(c.interface_language) << "\",\n"
        << "  \"confirm_exit\": " << (c.confirm_exit ? "true" : "false") << ",\n"
        << "  \"output_mode\": \"" << output_mode_name(c.output_mode) << "\",\n"
        << "  \"missing_reference_behavior\": \"" << missing_name(c.missing_behavior) << "\",\n"
        << "  \"camera_scaling\": \"" << scaling_name(c.camera_scaling) << "\",\n"
        << "  \"screen_scaling\": \"" << scaling_name(c.screen_scaling) << "\",\n"
        << "  \"placeholder_color_bgra\": " << c.placeholder_color_bgra << ",\n"
        << "  \"log_retention_days\": " << c.log_retention_days << ",\n"
        << "  \"diagnostic_logging\": " << (c.diagnostic_logging ? "true" : "false") << ",\n"
        << "  \"log_level\": \"" << log_level_name(c.log_level) << "\",\n"
        << "  \"video_auto_reconnect\": " << (c.video_auto_reconnect ? "true" : "false") << ",\n"
        << "  \"video_reconnect_interval_seconds\": " << c.video_reconnect_interval.count();
    if (c.last_tracked_monitor) {
        const auto& m = *c.last_tracked_monitor;
        out << ",\n  \"monitor_device_path\": \"" << escape_json(m.device_path) << "\""
            << ",\n  \"monitor_hardware_id\": \"" << escape_json(m.hardware_id) << "\""
            << ",\n  \"monitor_manufacturer\": \"" << escape_json(m.manufacturer) << "\""
            << ",\n  \"monitor_model\": \"" << escape_json(m.model) << "\""
            << ",\n  \"monitor_serial\": \"" << escape_json(m.serial) << "\""
            << ",\n  \"monitor_adapter_id\": \"" << escape_json(m.adapter_id) << "\""
            << ",\n  \"monitor_width\": " << m.resolution.width
            << ",\n  \"monitor_height\": " << m.resolution.height
            << ",\n  \"monitor_orientation\": " << m.orientation_degrees
            << ",\n  \"monitor_refresh_millihz\": " << m.refresh_rate_millihz
            << ",\n  \"monitor_x\": " << m.desktop_x
            << ",\n  \"monitor_y\": " << m.desktop_y;
    }
    out << "\n}\n";
    return out.str();
}

std::optional<AppConfig> ConfigStore::parse(const std::string_view json, std::string& error) {
    if (!valid_flat_json_object(json)) {
        error = "configuration is not a valid flat JSON object";
        return std::nullopt;
    }
    if (field_count(json, "schema_version") != 1) {
        error = "missing or duplicate schema_version";
        return std::nullopt;
    }
    AppConfig c;
    const auto schema = number_value<std::uint32_t>(json, "schema_version");
    if (!schema || *schema != 1) { error = "missing or unsupported schema_version"; return std::nullopt; }

    constexpr std::array string_fields{
        std::string_view{"selected_video_device_id"}, std::string_view{"reference_image_path"},
        std::string_view{"interface_language"}, std::string_view{"log_level"},
        std::string_view{"output_mode"}, std::string_view{"missing_reference_behavior"},
        std::string_view{"camera_scaling"}, std::string_view{"screen_scaling"},
        std::string_view{"monitor_device_path"}, std::string_view{"monitor_hardware_id"},
        std::string_view{"monitor_manufacturer"}, std::string_view{"monitor_model"},
        std::string_view{"monitor_serial"}, std::string_view{"monitor_adapter_id"}};
    constexpr std::array bool_fields{
        std::string_view{"cursor_visible"}, std::string_view{"start_with_windows"},
        std::string_view{"start_minimized"}, std::string_view{"start_automatically"},
        std::string_view{"close_to_tray"}, std::string_view{"show_notifications"},
        std::string_view{"confirm_exit"}, std::string_view{"diagnostic_logging"},
        std::string_view{"video_auto_reconnect"}};
    constexpr std::array uint32_fields{
        std::string_view{"preferred_input_width"}, std::string_view{"preferred_input_height"},
        std::string_view{"preferred_input_fps"}, std::string_view{"matches_required"},
        std::string_view{"mismatches_required"}, std::string_view{"reassignment_confirmations"},
        std::string_view{"output_width"}, std::string_view{"output_height"},
        std::string_view{"output_fps"}, std::string_view{"placeholder_color_bgra"},
        std::string_view{"log_retention_days"}, std::string_view{"monitor_width"},
        std::string_view{"monitor_height"}, std::string_view{"monitor_orientation"},
        std::string_view{"monitor_refresh_millihz"}};
    constexpr std::array int64_fields{
        std::string_view{"detection_interval_ms"}, std::string_view{"full_scan_interval_seconds"},
        std::string_view{"fade_duration_ms"}, std::string_view{"video_reconnect_interval_seconds"}};
    constexpr std::array int32_fields{std::string_view{"monitor_x"}, std::string_view{"monitor_y"}};
    constexpr std::array floating_fields{std::string_view{"detection_threshold"}, std::string_view{"reassignment_margin"}};
    if (!validate_string_fields(json, string_fields, error) ||
        !validate_bool_fields(json, bool_fields, error) ||
        !validate_uint32_fields(json, uint32_fields, error) ||
        !validate_number_fields<long long>(json, int64_fields, error) ||
        !validate_number_fields<std::int32_t>(json, int32_fields, error) ||
        !validate_number_fields<double>(json, floating_fields, error)) {
        return std::nullopt;
    }
    c.schema_version = *schema;
    if (const auto v = string_value(json, "selected_video_device_id")) c.selected_video_device_id = *v;
    if (const auto v = string_value(json, "reference_image_path")) c.reference_image_path = *v;
    if (const auto v = number_value<std::uint32_t>(json, "preferred_input_width")) c.preferred_input_size.width = *v;
    if (const auto v = number_value<std::uint32_t>(json, "preferred_input_height")) c.preferred_input_size.height = *v;
    if (const auto v = number_value<std::uint32_t>(json, "preferred_input_fps")) c.preferred_input_fps = *v;
    if (const auto v = number_value<double>(json, "detection_threshold")) c.detector.threshold = *v;
    if (const auto v = number_value<long long>(json, "detection_interval_ms")) c.detection_interval = std::chrono::milliseconds{*v};
    if (const auto v = number_value<std::uint32_t>(json, "matches_required")) c.detector.matches_required = *v;
    if (const auto v = number_value<std::uint32_t>(json, "mismatches_required")) c.detector.mismatches_required = *v;
    if (const auto v = number_value<long long>(json, "full_scan_interval_seconds")) c.full_scan_interval = std::chrono::seconds{*v};
    if (const auto v = number_value<std::uint32_t>(json, "reassignment_confirmations")) c.monitor_tracker.confirmations_required = *v;
    if (const auto v = number_value<double>(json, "reassignment_margin")) c.monitor_tracker.reassignment_margin = *v;
    c.monitor_tracker.match_threshold = c.detector.threshold;
    if (const auto v = number_value<std::uint32_t>(json, "output_width")) c.output_size.width = *v;
    if (const auto v = number_value<std::uint32_t>(json, "output_height")) c.output_size.height = *v;
    if (const auto v = number_value<std::uint32_t>(json, "output_fps")) c.output_fps = *v;
    if (const auto v = number_value<long long>(json, "fade_duration_ms")) c.fade_duration = std::chrono::milliseconds{*v};
    if (const auto v = bool_value(json, "cursor_visible")) c.cursor_visible = *v;
    if (const auto v = bool_value(json, "start_with_windows")) c.start_with_windows = *v;
    if (const auto v = bool_value(json, "start_minimized")) c.start_minimized = *v;
    if (const auto v = bool_value(json, "start_automatically")) c.start_automatically = *v;
    if (const auto v = bool_value(json, "close_to_tray")) c.close_to_tray = *v;
    if (const auto v = bool_value(json, "show_notifications")) c.show_notifications = *v;
    if (const auto v = string_value(json, "interface_language")) c.interface_language = *v;
    if (const auto v = bool_value(json, "confirm_exit")) c.confirm_exit = *v;
    if (const auto v = bool_value(json, "diagnostic_logging")) c.diagnostic_logging = *v;
    if (const auto v = string_value(json, "log_level")) {
        if (*v == "trace") c.log_level = LogLevel::trace;
        else if (*v == "debug") c.log_level = LogLevel::debug;
        else if (*v == "info") c.log_level = LogLevel::info;
        else if (*v == "warning") c.log_level = LogLevel::warning;
        else if (*v == "error") c.log_level = LogLevel::error;
        else { error = "invalid log_level"; return std::nullopt; }
    }
    if (const auto v = bool_value(json, "video_auto_reconnect")) c.video_auto_reconnect = *v;
    if (const auto v = number_value<long long>(json, "video_reconnect_interval_seconds")) c.video_reconnect_interval = std::chrono::seconds{*v};
    if (const auto v = number_value<std::uint32_t>(json, "log_retention_days")) c.log_retention_days = *v;
    if (const auto v = string_value(json, "output_mode")) {
        if (*v == "automatic") c.output_mode = OutputMode::automatic;
        else if (*v == "force_camera") c.output_mode = OutputMode::force_camera;
        else if (*v == "force_screen") c.output_mode = OutputMode::force_screen;
        else { error = "invalid output_mode"; return std::nullopt; }
    }
    if (const auto v = string_value(json, "missing_reference_behavior")) {
        if (*v == "use_camera") c.missing_behavior = MissingReferenceBehavior::use_camera;
        else if (*v == "keep_current") c.missing_behavior = MissingReferenceBehavior::keep_current;
        else if (*v == "use_last_screen") c.missing_behavior = MissingReferenceBehavior::use_last_screen;
        else if (*v == "use_placeholder") c.missing_behavior = MissingReferenceBehavior::use_placeholder;
        else { error = "invalid missing_reference_behavior"; return std::nullopt; }
    }
    const auto parse_scaling = [&error](const std::optional<std::string>& value, ScalingMode& output) {
        if (!value || *value == "fit") output = ScalingMode::fit;
        else if (*value == "fill") output = ScalingMode::fill;
        else if (*value == "stretch") output = ScalingMode::stretch;
        else { error = "invalid scaling mode"; return false; }
        return true;
    };
    if (!parse_scaling(string_value(json, "camera_scaling"), c.camera_scaling) ||
        !parse_scaling(string_value(json, "screen_scaling"), c.screen_scaling)) return std::nullopt;
    if (const auto v = number_value<std::uint32_t>(json, "placeholder_color_bgra")) c.placeholder_color_bgra = *v | 0xff000000u;

    const bool supported_input_size = (c.preferred_input_size == Size{1920, 1080}) || (c.preferred_input_size == Size{1280, 720});
    const bool supported_output_size = (c.output_size == Size{1920, 1080}) || (c.output_size == Size{1280, 720});
    if (!supported_input_size || c.preferred_input_fps != 30 || !supported_output_size || c.output_fps != 30 ||
        !std::isfinite(c.detector.threshold) || c.detector.threshold < 0.0 || c.detector.threshold > 1.0 ||
        c.detector.matches_required == 0 || c.detector.matches_required > 30 ||
        c.detector.mismatches_required == 0 || c.detector.mismatches_required > 30 ||
        c.detection_interval < std::chrono::milliseconds{100} ||
        c.detection_interval > std::chrono::milliseconds{1000} || c.fade_duration < std::chrono::milliseconds{0} ||
        c.fade_duration > std::chrono::milliseconds{2000} || c.log_retention_days == 0 || c.log_retention_days > 365 ||
        c.full_scan_interval < std::chrono::seconds{5} || c.full_scan_interval > std::chrono::seconds{3600} ||
        !std::isfinite(c.monitor_tracker.reassignment_margin) || c.monitor_tracker.reassignment_margin < 0.0 ||
        c.monitor_tracker.reassignment_margin > 1.0 || c.monitor_tracker.confirmations_required == 0 ||
        c.monitor_tracker.confirmations_required > 10 ||
        c.video_reconnect_interval < std::chrono::seconds{1} || c.video_reconnect_interval > std::chrono::seconds{60}) {
        error = "configuration contains out-of-range critical values";
        return std::nullopt;
    }
    if (const auto path = string_value(json, "monitor_device_path")) {
        MonitorIdentity m;
        m.device_path = *path;
        if (const auto v = string_value(json, "monitor_hardware_id")) m.hardware_id = *v;
        if (const auto v = string_value(json, "monitor_manufacturer")) m.manufacturer = *v;
        if (const auto v = string_value(json, "monitor_model")) m.model = *v;
        if (const auto v = string_value(json, "monitor_serial")) m.serial = *v;
        if (const auto v = string_value(json, "monitor_adapter_id")) m.adapter_id = *v;
        if (const auto v = number_value<std::uint32_t>(json, "monitor_width")) m.resolution.width = *v;
        if (const auto v = number_value<std::uint32_t>(json, "monitor_height")) m.resolution.height = *v;
        if (const auto v = number_value<std::uint32_t>(json, "monitor_orientation")) m.orientation_degrees = *v;
        if (const auto v = number_value<std::uint32_t>(json, "monitor_refresh_millihz")) m.refresh_rate_millihz = *v;
        if (const auto v = number_value<std::int32_t>(json, "monitor_x")) m.desktop_x = *v;
        if (const auto v = number_value<std::int32_t>(json, "monitor_y")) m.desktop_y = *v;
        c.last_tracked_monitor = std::move(m);
    }
    return c;
}

ConfigLoadResult ConfigStore::load() const {
    ConfigLoadResult result;
    if (!std::filesystem::exists(config_path())) return result;
    std::string error;
    try {
        if (auto parsed = parse(read_file(config_path()), error)) { result.config = std::move(*parsed); return result; }
    } catch (const std::exception& e) { error = e.what(); }
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
            } else result.warnings.push_back("Configuration backup invalid: " + backup_error);
        }
    } catch (const std::exception& e) { result.warnings.push_back(std::string("Could not preserve configuration: ") + e.what()); }
    return result;
}

void ConfigStore::save(const AppConfig& config) const {
    std::filesystem::create_directories(directory_);
    const auto temporary = directory_ / "config.tmp.json";
    {
        std::ofstream stream(temporary, std::ios::binary | std::ios::trunc);
        if (!stream) throw std::runtime_error("cannot create temporary configuration");
        stream << serialize(config);
        stream.flush();
        if (!stream) throw std::runtime_error("cannot flush temporary configuration");
    }
    std::string error;
    if (!parse(read_file(temporary), error)) {
        std::filesystem::remove(temporary);
        throw std::runtime_error("refusing to save invalid configuration: " + error);
    }
    if (std::filesystem::exists(config_path())) {
        std::string existing_error;
        try {
            if (parse(read_file(config_path()), existing_error))
                std::filesystem::copy_file(config_path(), directory_ / "config.backup.json", std::filesystem::copy_options::overwrite_existing);
        } catch (...) {}
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
