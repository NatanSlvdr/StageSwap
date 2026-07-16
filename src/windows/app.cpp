#include "app.hpp"
#include "status_presentation.hpp"
#include "tray_window.hpp"

#include <mfapi.h>
#include <shlobj.h>
#include <sddl.h>
#include <algorithm>
#include <chrono>
#include <fstream>
#include <cstring>

using namespace std::chrono_literals;

namespace asc::win {
namespace {
bool same_monitor(const RuntimeMonitorDescriptor& a, const RuntimeMonitorDescriptor& b) {
    return !a.gdi_display_name.empty() && a.gdi_display_name == b.gdi_display_name;
}
std::string video_source_description(const std::string& name, const std::string& identifier) {
    if (identifier.empty()) return "no video input";
    if (name.empty()) return "unavailable saved source [" + identifier + ']';
    return name + " [" + identifier + ']';
}
const char* device_state_name(const DeviceState state) {
    switch (state) {
    case DeviceState::unavailable: return "unavailable";
    case DeviceState::initializing: return "initializing";
    case DeviceState::ready: return "ready";
    case DeviceState::recovering: return "recovering";
    case DeviceState::failed: return "failed";
    }
    return "unknown";
}
template <typename FrameProvider>
bool wait_for_valid_frame(FrameProvider&& provider, const std::chrono::milliseconds timeout = 1500ms) {
    const auto deadline = Clock::now() + timeout;
    while (Clock::now() < deadline) {
        if (provider().valid()) return true;
        std::this_thread::sleep_for(25ms);
    }
    return provider().valid();
}
}

std::filesystem::path App::local_data_directory() {
    PWSTR path = nullptr;
    check_hresult(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_CREATE, nullptr, &path), "Locate LocalAppData");
    const std::filesystem::path result = std::filesystem::path(path) / L"AutomaticScreenCamera";
    CoTaskMemFree(path);
    return result;
}

std::wstring App::frame_pipe_name() {
    HANDLE token = nullptr;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "Open process token");
    DWORD bytes = 0;
    GetTokenInformation(token, TokenUser, nullptr, 0, &bytes);
    std::vector<std::byte> storage(bytes);
    if (!GetTokenInformation(token, TokenUser, storage.data(), bytes, &bytes)) { const auto error = GetLastError(); CloseHandle(token); throw HResultError(HRESULT_FROM_WIN32(error), "Read user SID"); }
    CloseHandle(token);
    LPWSTR sid = nullptr;
    if (!ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(storage.data())->User.Sid, &sid)) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "Format user SID");
    const std::wstring result = std::wstring(L"\\\\.\\pipe\\AutomaticScreenCamera.FinalFrame.") + sid;
    LocalFree(sid);
    return result;
}

App::App(const HINSTANCE instance)
    : instance_(instance), data_directory_(local_data_directory()), config_store_(data_directory_),
      loaded_config_(config_store_.load()), config_(loaded_config_.config),
      log_(data_directory_ / L"logs", 14), d3d_(), reference_store_(),
      video_input_(), screen_capture_(d3d_), compositor_(config_.placeholder_color_bgra),
      publisher_(pipeline_size, frame_pipe_name()) {
    log_.write(LogLevel::info, "configuration", "CONFIGURATION_LOADED", loaded_config_.used_backup ? "Configuration backup loaded" : "Configuration loaded");
    for (const auto& warning : loaded_config_.warnings) {
        log_.write(LogLevel::warning, "configuration", "CONFIGURATION_WARNING", warning);
        if (!configuration_warning_.empty()) configuration_warning_ += "; ";
        configuration_warning_ += warning;
    }
    log_.set_minimum_level(LogLevel::info);
    log_.set_retention_days(14);
    wchar_t startup_command[MAX_PATH]{};
    DWORD startup_bytes = sizeof(startup_command);
    config_.start_with_windows = RegGetValueW(HKEY_CURRENT_USER, L"Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                                               L"AutomaticScreenCamera", RRF_RT_REG_SZ, nullptr, startup_command, &startup_bytes) == ERROR_SUCCESS;
    controller_ = std::make_unique<AppController>(config_, log_);
    const auto reference_path = config_store_.reference_path();
    if (std::filesystem::exists(reference_path)) {
        try {
            if (const auto saved_thumbnail = load_reference_thumbnail()) reference_thumbnail_ = *saved_thumbnail;
            else reference_thumbnail_ = reference_store_.load_thumbnail(reference_path);
            save_reference_thumbnail(reference_thumbnail_);
            has_reference_ = true;
        }
        catch (const std::exception& e) { report_error("reference", "REFERENCE_LOAD_FAILED", e); }
    }
}

App::~App() {
    exiting_ = true;
    automation_running_ = false;
    if (reference_worker_.joinable()) reference_worker_.request_stop();
    if (detector_thread_.joinable()) detector_thread_.request_stop();
    if (rescan_thread_.joinable()) rescan_thread_.request_stop();
    if (compositor_thread_.joinable()) compositor_thread_.request_stop();
    if (compositor_thread_.joinable()) compositor_thread_.join();
    publisher_.invalidate();
    if (reference_worker_.joinable()) reference_worker_.join();
    if (detector_thread_.joinable()) detector_thread_.join();
    if (rescan_thread_.joinable()) rescan_thread_.join();
    virtual_camera_.stop();
    screen_capture_.stop();
    video_input_.stop();
    save_config();
}

int App::run() {
    window_ = std::make_unique<TrayWindow>(instance_, *this);
    initialize_components();
    if (!config_.start_automatically) {
        controller_->stop();
        suspend_screen_capture();
    }
    compositor_thread_ = std::jthread([this](const std::stop_token stop) { compositor_loop(stop); });
    if (config_.start_automatically) start_automation();
    if (!config_.start_minimized) window_->show();
    return window_->message_loop();
}

void App::initialize_components() {
    refresh_selected_video_device_name();
    controller_->begin_start();
    bool video_ready = false, screen_ready = false, virtual_ready = false;
    try {
        if (!config_.selected_video_device_id.empty()) {
            video_input_.start(config_.selected_video_device_id);
            video_ready = wait_for_valid_frame([this] { return video_input_.latest_frame(); });
            log_.write(LogLevel::info, "video_input", "VIDEO_INPUT_INITIALIZED",
                       "Selected video input initialized: " + video_source_description(selected_video_device_name_, config_.selected_video_device_id));
        }
    } catch (const std::exception& e) { report_error("video_input", "VIDEO_INPUT_FAILED", e); }
    try {
        const auto monitors = enumerate_monitors();
        if (const auto selected = resolve_tracked_monitor(monitors)) {
            controller_->set_tracked_monitor(selected->descriptor);
            screen_capture_.start(selected->handle, config_.cursor_visible);
            screen_ready = wait_for_valid_frame([this] { return screen_capture_.latest_frame(); });
            log_.write(LogLevel::info, "screen_capture", "SCREEN_CAPTURE_INITIALIZED", "Tracked monitor capture initialized");
        }
    } catch (const std::exception& e) { report_error("screen_capture", "SCREEN_CAPTURE_FAILED", e); }
    try { virtual_camera_.start(publisher_.pipe_name(), config_.placeholder_color_bgra); virtual_ready = true; log_.write(LogLevel::info, "virtual_camera", "VIRTUAL_CAMERA_INITIALIZED", "Virtual camera registered and started"); }
    catch (const std::exception& e) { report_error("virtual_camera", "VIRTUAL_CAMERA_FAILED", e); }
    controller_->finish_start(video_ready, screen_ready, virtual_ready);
    rescan_requested_ = true;
}

std::optional<MonitorDevice> App::resolve_tracked_monitor(const std::vector<MonitorDevice>& monitors) const {
    const auto tracked = controller_->status().tracked_monitor;
    if (tracked) {
        const auto found = std::find_if(monitors.begin(), monitors.end(), [&](const auto& monitor) { return same_monitor(monitor.descriptor, *tracked); });
        if (found != monitors.end()) return *found;
        // A persisted physical display must never be replaced with an
        // arbitrary primary display merely because it is temporarily absent.
        // Reference rediscovery is the only automatic path allowed to move
        // tracking to another monitor.
        return std::nullopt;
    }
    const auto primary = std::find_if(monitors.begin(), monitors.end(), [](const auto& monitor) {
        MONITORINFO info{sizeof(info)}; return GetMonitorInfoW(monitor.handle, &info) && (info.dwFlags & MONITORINFOF_PRIMARY) != 0;
    });
    if (primary != monitors.end()) return *primary;
    if (!monitors.empty()) return monitors.front();
    return std::nullopt;
}

void App::start_automation() {
    if (automation_running_.exchange(true)) return;
    if (controller_->status().run_state == RunState::stopped) {
        if (!config_.selected_video_device_id.empty() && controller_->status().video_input != DeviceState::ready)
            restart_video_input();
        if (controller_->status().screen_capture != DeviceState::ready) restart_screen_capture();
        if (controller_->status().virtual_camera != DeviceState::ready) restart_virtual_camera();
        const auto previous = controller_->status();
        controller_->begin_start();
        controller_->finish_start(previous.video_input == DeviceState::ready, previous.screen_capture == DeviceState::ready,
                                  previous.virtual_camera == DeviceState::ready);
    }
    detector_thread_ = std::jthread([this](const std::stop_token stop) { detector_loop(stop); });
    rescan_thread_ = std::jthread([this](const std::stop_token stop) { rescan_loop(stop); });
    rescan_requested_ = true;
    log_.write(LogLevel::info, "detector", "DETECTION_STARTED", "Automatic detection started");
}

void App::stop_automation() {
    if (!automation_running_.exchange(false)) return;
    const auto stop_requested_at = Clock::now();
    controller_->stop(stop_requested_at);
    if (detector_thread_.joinable()) { detector_thread_.request_stop(); detector_thread_.join(); }
    if (rescan_thread_.joinable()) { rescan_thread_.request_stop(); rescan_thread_.join(); }
    const auto transition_deadline = stop_requested_at + 750ms;
    while (controller_->status().transition.active && Clock::now() < transition_deadline)
        std::this_thread::sleep_for(10ms);
    if (controller_->status().transition.active)
        log_.write(LogLevel::warning, "lifecycle", "STOP_SAFE_TRANSITION_TIMEOUT",
                   "Safe stopped output transition timed out; closing screen capture immediately");
    suspend_screen_capture();
    log_.write(LogLevel::info, "detector", "DETECTION_STOPPED", "Automatic detection stopped");
}

void App::suspend_screen_capture() {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    {
        std::scoped_lock lock(component_mutex_);
        screen_capture_.stop();
    }
    controller_->set_component_state(Source::screen, DeviceState::unavailable, Clock::now());
    log_.write(LogLevel::info, "screen_capture", "SCREEN_CAPTURE_SUSPENDED",
               "Tracked screen capture closed while automation is stopped");
}

void App::set_mode(const OutputMode mode) {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    controller_->set_mode(mode, Clock::now());
    save_config();
}

void App::compositor_loop(const std::stop_token stop) {
    constexpr auto frame_period = std::chrono::nanoseconds{1'000'000'000 / pipeline_fps};
    auto next = std::chrono::steady_clock::now();
    std::int64_t timestamp = 0;
    while (!stop.stop_requested()) {
        try {
            const auto now = Clock::now();
            const auto device_status = d3d_.device()->GetDeviceRemovedReason();
            if (FAILED(device_status) && !graphics_relaunch_required_.exchange(true)) {
                controller_->set_component_state(Source::screen, DeviceState::failed, now);
                log_.write(LogLevel::error, "graphics", "D3D_DEVICE_REMOVED",
                           "The graphics device was removed; relaunch Automatic Screen Camera to restore screen capture",
                           std::string("{\"hresult\":") + std::to_string(device_status) + "}");
            }
            controller_->tick(now);
            const auto state = controller_->status();
            VideoFrame camera, screen;
            { std::scoped_lock lock(component_mutex_); camera = video_input_.latest_frame(); screen = screen_capture_.latest_frame(); }
            if (!camera.fresh(now)) camera = {};
            if (!screen.fresh(now)) screen = {};
            VideoFrame frame;
            {
                std::scoped_lock compositor_lock(compositor_mutex_);
                frame = compositor_.compose(camera, screen, state.transition.screen_mix, timestamp);
                { std::scoped_lock lock(component_mutex_); final_output_frame_ = frame; }
                publisher_.publish(frame);
            }
            timestamp += 10'000'000 / pipeline_fps;
        } catch (const std::exception& e) { report_error("compositor", "COMPOSITOR_FRAME_FAILED", e); }
        next += frame_period;
        if (next + frame_period < std::chrono::steady_clock::now()) next = std::chrono::steady_clock::now();
        std::this_thread::sleep_until(next);
    }
}

void App::detector_loop(const std::stop_token stop) {
    while (!stop.stop_requested() && automation_running_) {
        try {
            if (!has_reference_) controller_->on_similarity(0, false, Clock::now());
            else {
                GrayImage reference;
                { std::scoped_lock reference_lock(reference_mutex_); reference = reference_thumbnail_; }
                std::optional<GrayImage> current;
                { std::scoped_lock lock(component_mutex_); current = screen_capture_.comparison_frame(reference.size); }
                const double similarity = current ? image_similarity(reference, *current) : 0;
                controller_->on_similarity(similarity, current.has_value(), Clock::now());
            }
        } catch (const std::exception& e) { report_error("detector", "DETECTION_CHECK_FAILED", e); controller_->on_similarity(0, false, Clock::now()); }
        std::this_thread::sleep_for(250ms);
    }
}

void App::rescan_loop(const std::stop_token stop) {
    winrt::init_apartment(winrt::apartment_type::multi_threaded);
    auto next = Clock::now();
    while (!stop.stop_requested() && automation_running_) {
        if (rescan_requested_.exchange(false) || Clock::now() >= next) {
            try { full_monitor_scan(); }
            catch (const std::exception& e) { report_error("monitors", "FULL_SCAN_FAILED", e); controller_->on_similarity(0, false, Clock::now()); }
            next = Clock::now() + 30s;
        }
        std::this_thread::sleep_for(100ms);
    }
}


void App::full_monitor_scan() {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    log_.write(LogLevel::info, "monitors", "FULL_SCAN_STARTED", "Full display scan started");
    const auto monitors = enumerate_monitors();
    std::vector<MonitorScore> scores;
    scores.reserve(monitors.size());
    GrayImage reference;
    if (has_reference_) { std::scoped_lock reference_lock(reference_mutex_); reference = reference_thumbnail_; }
    for (const auto& monitor : monitors) {
        MonitorScore score{monitor.descriptor, 0, false};
        if (has_reference_) {
            try {
                ScreenCapture capture(d3d_);
                capture.start(monitor.handle, false);
                for (int attempt = 0; attempt < 15 && !capture.latest_frame().valid(); ++attempt) std::this_thread::sleep_for(20ms);
                if (const auto thumbnail = capture.comparison_frame(reference.size)) {
                    score.similarity = image_similarity(reference, *thumbnail); score.capture_valid = true;
                }
            } catch (const std::exception& e) { report_error("monitors", "MONITOR_SCAN_CAPTURE_FAILED", e); }
        }
        scores.push_back(std::move(score));
    }
    const auto result = controller_->on_monitor_scan(scores, Clock::now());
    if (result.confirmation_pending) rescan_requested_ = true;
    if (automation_running_ && result.changed && result.tracked) {
        const auto found = std::find_if(monitors.begin(), monitors.end(), [&](const auto& monitor) { return same_monitor(monitor.descriptor, *result.tracked); });
        if (found != monitors.end()) {
            {
                std::scoped_lock lock(component_mutex_);
                screen_capture_.start(found->handle, config_.cursor_visible);
            }
            const bool new_screen_ready = wait_for_valid_frame([this] { return screen_capture_.latest_frame(); });
            controller_->set_component_state(Source::screen, new_screen_ready ? DeviceState::ready : DeviceState::recovering, Clock::now());
        }
    }
    std::string scan_message = "Full display scan completed; " + result.message;
    if (result.tracked && !result.tracked->label.empty()) scan_message += " (" + result.tracked->label + ")";
    log_.write(LogLevel::info, "monitors", "FULL_SCAN_COMPLETED", std::move(scan_message),
               std::string("{\"display_count\":") + std::to_string(monitors.size()) +
                   ",\"best_similarity\":" + std::to_string(result.best_similarity) + "}");
}

void App::set_current_screen_reference() {
    if (reference_worker_.joinable()) reference_worker_.request_stop();
    reference_worker_ = std::jthread([this](const std::stop_token stop) {
        winrt::init_apartment(winrt::apartment_type::multi_threaded);
        for (int i = 0; i < 30 && !stop.stop_requested(); ++i) std::this_thread::sleep_for(100ms);
        if (stop.stop_requested()) return;
        try {
            std::scoped_lock lifecycle_lock(lifecycle_mutex_);
            const auto monitors = enumerate_monitors();
            const auto selected = resolve_tracked_monitor(monitors);
            if (!selected) throw std::runtime_error("no monitor is available for the reference capture");
            ScreenCapture reference_capture(d3d_);
            ReferenceStore worker_reference_store;
            reference_capture.start(selected->handle, false);
            if (!wait_for_valid_frame([&] { return reference_capture.latest_frame(); })) throw std::runtime_error("reference monitor did not produce a capture frame");
            auto thumbnail = worker_reference_store.save_frame(reference_capture.latest_frame(), config_store_.reference_path());
            if (const auto live_thumbnail = reference_capture.comparison_frame({160, 90})) thumbnail = *live_thumbnail;
            const auto verification = reference_capture.comparison_frame(thumbnail.size);
            const double initial_score = verification ? image_similarity(thumbnail, *verification) : 0.0;
            { std::scoped_lock reference_lock(reference_mutex_); reference_thumbnail_ = thumbnail; save_reference_thumbnail(reference_thumbnail_); }
            has_reference_ = true;
            config_.reference_image_path = config_store_.reference_path().string();
            save_config();
            log_.write(LogLevel::info, "reference", "REFERENCE_CREATED", "Current screen saved as reference",
                       std::string("{\"similarity\":") + std::to_string(initial_score) + "}");
            rescan_requested_ = true;
        } catch (const std::exception& e) { report_error("reference", "REFERENCE_CREATE_FAILED", e); }
    });
}

void App::set_reference_monitor(const RuntimeMonitorDescriptor& monitor) {
    try {
        select_tracked_monitor(monitor);
        set_current_screen_reference();
    } catch (const std::exception& e) { report_error("reference", "REFERENCE_MONITOR_SELECTION_FAILED", e); }
}

void App::select_tracked_monitor(const RuntimeMonitorDescriptor& monitor) {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    const auto available = enumerate_monitors();
    const auto found = std::find_if(available.begin(), available.end(), [&](const auto& value) { return same_monitor(value.descriptor, monitor); });
    if (found == available.end()) throw std::runtime_error("selected monitor is no longer connected");
    controller_->set_tracked_monitor(found->descriptor);
    if (!automation_running_) return;
    {
        std::scoped_lock lock(component_mutex_);
        screen_capture_.start(found->handle, config_.cursor_visible);
    }
    const bool ready = wait_for_valid_frame([this] { return screen_capture_.latest_frame(); });
    controller_->set_component_state(Source::screen, ready ? DeviceState::ready : DeviceState::recovering, Clock::now());
}

void App::import_reference(const std::filesystem::path& source) {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    try {
        const auto thumbnail = reference_store_.import_image(source, config_store_.reference_path());
        { std::scoped_lock reference_lock(reference_mutex_); reference_thumbnail_ = thumbnail; save_reference_thumbnail(reference_thumbnail_); }
        has_reference_ = true; config_.reference_image_path = config_store_.reference_path().string(); save_config();
        log_.write(LogLevel::info, "reference", "REFERENCE_IMPORTED", "Reference image imported"); rescan_requested_ = true;
    } catch (const std::exception& e) { report_error("reference", "REFERENCE_IMPORT_FAILED", e); }
}

void App::request_rescan() {
    if (automation_running_) rescan_requested_ = true;
    else {
        rescan_requested_ = false;
        full_monitor_scan();
        if (rescan_requested_.exchange(false)) full_monitor_scan();
    }
}
void App::restart_video_input() {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    refresh_selected_video_device_name();
    controller_->set_component_state(Source::camera, DeviceState::recovering, Clock::now());
    log_.write(LogLevel::info, "recovery", "VIDEO_INPUT_RECOVERY_STARTED",
               "Restarting selected video input: " + video_source_description(selected_video_device_name_, config_.selected_video_device_id));
    if (config_.selected_video_device_id.empty()) {
        { std::scoped_lock lock(component_mutex_); video_input_.stop(); }
        controller_->set_component_state(Source::camera, DeviceState::unavailable, Clock::now());
        log_.write(LogLevel::info, "recovery", "VIDEO_INPUT_UNAVAILABLE", "No video input is selected");
        return;
    }
    try { { std::scoped_lock lock(component_mutex_); video_input_.restart(); }
          if (!wait_for_valid_frame([this] { return video_input_.latest_frame(); })) throw std::runtime_error("video input did not produce a valid frame");
          controller_->set_component_state(Source::camera, DeviceState::ready, Clock::now());
          log_.write(LogLevel::info, "recovery", "VIDEO_INPUT_RECOVERED",
                     "Video input restarted: " + video_source_description(selected_video_device_name_, config_.selected_video_device_id)); }
    catch (const std::exception& e) { controller_->set_component_state(Source::camera, DeviceState::failed, Clock::now()); report_error("recovery", "VIDEO_INPUT_RECOVERY_FAILED", e); }
}
void App::restart_screen_capture() {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    controller_->set_component_state(Source::screen, DeviceState::recovering, Clock::now());
    log_.write(LogLevel::info, "recovery", "SCREEN_CAPTURE_RECOVERY_STARTED", "Restarting tracked screen capture");
    try { const auto monitors = enumerate_monitors(); const auto selected = resolve_tracked_monitor(monitors); if (!selected) throw std::runtime_error("no monitor available");
          { std::scoped_lock lock(component_mutex_); screen_capture_.start(selected->handle, config_.cursor_visible); }
          if (!wait_for_valid_frame([this] { return screen_capture_.latest_frame(); })) throw std::runtime_error("screen capture did not produce a valid frame");
          controller_->set_component_state(Source::screen, DeviceState::ready, Clock::now()); log_.write(LogLevel::info, "recovery", "SCREEN_CAPTURE_RECOVERED", "Screen capture restarted"); }
    catch (const std::exception& e) { controller_->set_component_state(Source::screen, DeviceState::failed, Clock::now()); report_error("recovery", "SCREEN_CAPTURE_RECOVERY_FAILED", e); }
}
void App::restart_virtual_camera() {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    controller_->set_virtual_camera_state(DeviceState::recovering);
    log_.write(LogLevel::info, "recovery", "VIRTUAL_CAMERA_RECOVERY_STARTED", "Restarting virtual camera");
    try { virtual_camera_.restart(); controller_->set_virtual_camera_state(DeviceState::ready); log_.write(LogLevel::info, "recovery", "VIRTUAL_CAMERA_RECOVERED", "Virtual camera restarted"); }
    catch (const std::exception& e) { controller_->set_virtual_camera_state(DeviceState::failed); report_error("recovery", "VIRTUAL_CAMERA_RECOVERY_FAILED", e); }
}
void App::restart_all() {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    log_.write(LogLevel::info, "recovery", "RECOVERY_STARTED", "Restarting all video components");
    publisher_.invalidate();
    restart_video_input();
    restart_screen_capture();
    restart_virtual_camera();
    request_rescan();
    const auto state = controller_->status();
    const bool video_ready = config_.selected_video_device_id.empty() ? state.video_input == DeviceState::unavailable :
                                                                       state.video_input == DeviceState::ready;
    const bool recovered = video_ready && state.screen_capture == DeviceState::ready &&
                           state.virtual_camera == DeviceState::ready;
    const std::string details = std::string("{\"video_input\":\"") + device_state_name(state.video_input) +
        "\",\"screen_capture\":\"" + device_state_name(state.screen_capture) +
        "\",\"virtual_camera\":\"" + device_state_name(state.virtual_camera) + "\"}";
    log_.write(recovered ? LogLevel::info : LogLevel::error, "recovery",
               recovered ? "RECOVERY_SUCCEEDED" : "RECOVERY_FAILED",
               recovered ? "All video components recovered" : "One or more video components did not recover",
               details);
}
void App::export_logs(const std::filesystem::path& destination) const { log_.export_to(destination); }
void App::clear_logs() { log_.clear(); log_.write(LogLevel::info, "logging", "LOGS_CLEARED", "Diagnostic logs cleared by user"); }
std::optional<PreviewImage> App::preview(const PreviewKind kind, const Size target) {
    if (target.width == 0 || target.height == 0) return std::nullopt;
    if (kind == PreviewKind::reference) {
        std::scoped_lock lock(reference_mutex_);
        if (!reference_thumbnail_.valid()) return std::nullopt;
        const Size preview_size = fit_preview_size(reference_thumbnail_.size, target);
        const auto gray = reference_thumbnail_.size == preview_size
            ? reference_thumbnail_ : resize_bilinear(reference_thumbnail_, preview_size);
        PreviewImage result{preview_size, std::vector<std::uint8_t>(static_cast<std::size_t>(preview_size.width) * preview_size.height * 4)};
        for (std::size_t i = 0; i < gray.pixels.size(); ++i) {
            result.bgra[i * 4] = result.bgra[i * 4 + 1] = result.bgra[i * 4 + 2] = gray.pixels[i];
            result.bgra[i * 4 + 3] = 255;
        }
        return result;
    }
    std::scoped_lock lock(component_mutex_);
    VideoFrame frame;
    if (kind == PreviewKind::camera) frame = video_input_.latest_frame();
    else if (kind == PreviewKind::screen) frame = screen_capture_.latest_frame();
    else frame = final_output_frame_;
    if (!frame.valid()) return std::nullopt;
    const Size preview_size = fit_preview_size(frame.size, target);
    PreviewImage result{preview_size, std::vector<std::uint8_t>(static_cast<std::size_t>(preview_size.width) * preview_size.height * 4)};
    for (std::uint32_t y = 0; y < preview_size.height; ++y) {
        const auto source_y = std::min(frame.size.height - 1, static_cast<std::uint32_t>((static_cast<std::uint64_t>(y) * frame.size.height) / preview_size.height));
        for (std::uint32_t x = 0; x < preview_size.width; ++x) {
            const auto source_x = std::min(frame.size.width - 1, static_cast<std::uint32_t>((static_cast<std::uint64_t>(x) * frame.size.width) / preview_size.width));
            auto* destination = result.bgra.data() + (static_cast<std::size_t>(y) * preview_size.width + x) * 4;
            const auto* source = frame.bgra.data() + static_cast<std::size_t>(source_y) * frame.stride + source_x * 4;
            std::memcpy(destination, source, 4);
        }
    }
    return result;
}
void App::log_system_event(std::string code, std::string message) { log_.write(LogLevel::info, "lifecycle", std::move(code), std::move(message)); }

std::vector<VideoDevice> App::video_devices() const {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    try { return enumerate_video_devices(); }
    catch (...) { return {}; }
}

SelectedVideoSourceInfo App::selected_video_source() const {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    return {config_.selected_video_device_id,
            selected_video_device_name_for_id_ == config_.selected_video_device_id ? selected_video_device_name_ : std::string{}};
}

void App::refresh_selected_video_device_name() {
    if (config_.selected_video_device_id.empty()) {
        selected_video_device_name_.clear();
        selected_video_device_name_for_id_.clear();
        return;
    }
    if (selected_video_device_name_for_id_ != config_.selected_video_device_id) {
        selected_video_device_name_.clear();
        selected_video_device_name_for_id_ = config_.selected_video_device_id;
    }
    try {
        if (const auto name = find_video_device_name(config_.selected_video_device_id)) selected_video_device_name_ = *name;
    } catch (...) {
        // Keep the last known friendly name. Device initialization/recovery
        // reports the actionable error and retains the stable identifier.
    }
}
std::vector<MonitorDevice> App::monitors() const {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    try { return enumerate_monitors(); }
    catch (...) { return {}; }
}

void App::apply_settings(AppConfig updated) {
    const bool was_running = automation_running_;
    const auto prior_mode = controller_->status().mode;
    if (was_running) stop_automation();
    if (compositor_thread_.joinable()) { compositor_thread_.request_stop(); compositor_thread_.join(); }
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    const auto old = config_;
    updated.output_mode = prior_mode;
    config_ = std::move(updated);
    refresh_selected_video_device_name();
    configuration_warning_.clear();
    log_.set_minimum_level(LogLevel::info);
    log_.set_retention_days(14);
    controller_->reconfigure(config_, Clock::now());
    if (old.placeholder_color_bgra != config_.placeholder_color_bgra) {
        std::scoped_lock compositor_lock(compositor_mutex_);
        compositor_.set_placeholder_color(config_.placeholder_color_bgra);
    }
    if (old.placeholder_color_bgra != config_.placeholder_color_bgra) {
        try { virtual_camera_.start(publisher_.pipe_name(), config_.placeholder_color_bgra); controller_->set_virtual_camera_state(DeviceState::ready); }
        catch (const std::exception& e) { controller_->set_virtual_camera_state(DeviceState::failed); report_error("virtual_camera", "VIRTUAL_CAMERA_RECONFIGURE_FAILED", e); }
    }
    if (old.selected_video_device_id != config_.selected_video_device_id) {
        try {
            if (config_.selected_video_device_id.empty()) {
                { std::scoped_lock lock(component_mutex_); video_input_.stop(); }
                controller_->set_component_state(Source::camera, DeviceState::unavailable, Clock::now());
            } else {
                { std::scoped_lock lock(component_mutex_);
                  video_input_.start(config_.selected_video_device_id); }
                if (!wait_for_valid_frame([this] { return video_input_.latest_frame(); })) throw std::runtime_error("configured video input did not produce a valid frame");
                controller_->set_component_state(Source::camera, DeviceState::ready, Clock::now());
                log_.write(LogLevel::info, "video_input", "VIDEO_INPUT_INITIALIZED",
                           "Configured video input initialized: " +
                               video_source_description(selected_video_device_name_, config_.selected_video_device_id));
            }
        } catch (const std::exception& e) { controller_->set_component_state(Source::camera, DeviceState::failed, Clock::now()); report_error("video_input", "VIDEO_INPUT_RECONFIGURE_FAILED", e); }
    }
    if (old.cursor_visible != config_.cursor_visible) restart_screen_capture();
    wchar_t executable[MAX_PATH]{}; GetModuleFileNameW(nullptr, executable, ARRAYSIZE(executable));
    HKEY run_key = nullptr;
    if (RegCreateKeyExW(HKEY_CURRENT_USER, L"Software\\Microsoft\\Windows\\CurrentVersion\\Run", 0, nullptr, 0, KEY_SET_VALUE, nullptr, &run_key, nullptr) == ERROR_SUCCESS) {
        if (config_.start_with_windows) {
            const std::wstring command = std::wstring(L"\"") + executable + L"\"";
            RegSetValueExW(run_key, L"AutomaticScreenCamera", 0, REG_SZ, reinterpret_cast<const BYTE*>(command.c_str()), static_cast<DWORD>((command.size() + 1) * sizeof(wchar_t)));
        } else RegDeleteValueW(run_key, L"AutomaticScreenCamera");
        RegCloseKey(run_key);
    }
    if (!was_running) suspend_screen_capture();
    save_config();
    compositor_thread_ = std::jthread([this](const std::stop_token stop) { compositor_loop(stop); });
    if (was_running) start_automation();
}

void App::save_config() {
    std::scoped_lock lifecycle_lock(lifecycle_mutex_);
    try { if (controller_) config_.output_mode = controller_->status().mode; config_store_.save(config_); }
    catch (const std::exception& e) { report_error("configuration", "CONFIGURATION_SAVE_FAILED", e); }
}

void App::save_reference_thumbnail(const GrayImage& image) const {
    if (!image.valid()) return;
    std::ofstream output(config_store_.comparison_path(), std::ios::binary | std::ios::trunc);
    const std::uint32_t header[]{0x41534354u, image.size.width, image.size.height};
    output.write(reinterpret_cast<const char*>(header), sizeof(header));
    output.write(reinterpret_cast<const char*>(image.pixels.data()), static_cast<std::streamsize>(image.pixels.size()));
}

std::optional<GrayImage> App::load_reference_thumbnail() const {
    std::ifstream input(config_store_.comparison_path(), std::ios::binary);
    if (!input) return std::nullopt;
    std::uint32_t header[3]{}; input.read(reinterpret_cast<char*>(header), sizeof(header));
    if (!input || header[0] != 0x41534354u || header[1] == 0 || header[2] == 0 || header[1] > 640 || header[2] > 360) return std::nullopt;
    GrayImage image{{header[1], header[2]}, std::vector<std::uint8_t>(static_cast<std::size_t>(header[1]) * header[2])};
    input.read(reinterpret_cast<char*>(image.pixels.data()), static_cast<std::streamsize>(image.pixels.size()));
    return input && image.valid() ? std::optional<GrayImage>{std::move(image)} : std::nullopt;
}
void App::report_error(std::string component, std::string code, const std::exception& error) {
    log_.write(LogLevel::error, std::move(component), std::move(code), error.what());
}
AppConfig App::config() const { std::scoped_lock lifecycle_lock(lifecycle_mutex_); return config_; }
AppStatus App::status() const { std::scoped_lock lifecycle_lock(lifecycle_mutex_); auto result = controller_->status(); if (result.warning.empty() && !configuration_warning_.empty()) result.warning = configuration_warning_; return result; }
std::vector<LogEvent> App::recent_events() const { return log_.recent(); }
void App::exit() { exiting_ = true; if (window_) window_->close(); }

} // namespace asc::win
