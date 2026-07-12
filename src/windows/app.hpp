#pragma once

#include "compositor.hpp"
#include "device_enumerator.hpp"
#include "reference_store.hpp"
#include "screen_capture.hpp"
#include "shared_frame.hpp"
#include "video_input.hpp"
#include "virtual_camera.hpp"
#include "asc/core/controller.hpp"

#include <atomic>
#include <filesystem>
#include <memory>
#include <mutex>
#include <stop_token>
#include <thread>
#include <vector>
#include <optional>
#include <cstdint>

namespace asc::win {

class TrayWindow;

enum class PreviewKind { camera, screen, output, reference };
struct PreviewImage { Size size; std::vector<std::uint8_t> bgra; };
struct DiagnosticCounters {
    std::uint64_t frames_published{0};
    std::uint64_t detection_checks{0};
    std::uint64_t full_scans{0};
    std::uint64_t recovery_attempts{0};
    std::uint64_t failures{0};
};

class App {
public:
    explicit App(HINSTANCE instance);
    ~App();
    int run();

    void start_automation();
    void stop_automation();
    void set_mode(OutputMode mode);
    void set_current_screen_reference();
    void set_reference_monitor(const MonitorIdentity& monitor);
    void select_tracked_monitor(const MonitorIdentity& monitor);
    void import_reference(const std::filesystem::path& source);
    void request_rescan();
    void restart_video_input();
    void restart_screen_capture();
    void restart_virtual_camera();
    void restart_all();
    void export_logs(const std::filesystem::path& destination) const;
    void clear_logs();
    [[nodiscard]] std::optional<PreviewImage> preview(PreviewKind kind, Size target = {240, 135});
    [[nodiscard]] DiagnosticCounters diagnostic_counters() const noexcept;
    void reset_diagnostic_counters();
    void log_system_event(std::string code, std::string message);
    void exit();

    [[nodiscard]] AppStatus status() const;
    [[nodiscard]] std::vector<LogEvent> recent_events() const;
    [[nodiscard]] std::filesystem::path data_directory() const { return data_directory_; }
    // Return a snapshot: settings may be replaced while windows are rendering.
    [[nodiscard]] AppConfig config() const;
    [[nodiscard]] bool automation_running() const noexcept { return automation_running_; }
    [[nodiscard]] std::vector<VideoDevice> video_devices() const;
    [[nodiscard]] std::vector<MonitorDevice> monitors() const;
    void apply_settings(AppConfig updated);

private:
    static std::filesystem::path local_data_directory();
    static std::wstring frame_pipe_name();
    void initialize_components();
    void compositor_loop(std::stop_token stop);
    void detector_loop(std::stop_token stop);
    void rescan_loop(std::stop_token stop);
    void recovery_loop(std::stop_token stop);
    void full_monitor_scan();
    [[nodiscard]] std::optional<MonitorDevice> resolve_tracked_monitor(const std::vector<MonitorDevice>& monitors) const;
    void save_config();
    void save_reference_thumbnail(const GrayImage& image) const;
    [[nodiscard]] std::optional<GrayImage> load_reference_thumbnail() const;
    void report_error(std::string component, std::string code, const std::exception& error);

    HINSTANCE instance_;
    std::filesystem::path data_directory_;
    ConfigStore config_store_;
    ConfigLoadResult loaded_config_;
    AppConfig config_;
    EventLog log_;
    std::unique_ptr<AppController> controller_;
    D3DDevice d3d_;
    ReferenceStore reference_store_;
    VideoInput video_input_;
    ScreenCapture screen_capture_;
    Compositor compositor_;
    SharedFramePublisher publisher_;
    VirtualCamera virtual_camera_;
    std::unique_ptr<TrayWindow> window_;
    // All start/stop/reconfigure/device-loss operations pass through this gate.
    // Recursive acquisition is intentional for composite operations such as
    // restart_all() and settings application.
    mutable std::recursive_mutex lifecycle_mutex_;
    mutable std::mutex component_mutex_;
    mutable std::mutex compositor_mutex_;
    GrayImage reference_thumbnail_;
    mutable std::mutex reference_mutex_;
    std::atomic<bool> has_reference_{false};
    std::atomic<bool> automation_running_{false};
    std::atomic<bool> rescan_requested_{false};
    std::atomic<bool> exiting_{false};
    std::jthread compositor_thread_;
    std::jthread detector_thread_;
    std::jthread rescan_thread_;
    std::jthread recovery_thread_;
    std::jthread reference_worker_;
    std::vector<MonitorIdentity> known_monitors_;
    std::string configuration_warning_;
    VideoFrame previous_screen_frame_;
    VideoFrame safe_screen_frame_;
    VideoFrame final_output_frame_;
    std::atomic<bool> raw_reference_matching_{false};
    TimePoint screen_switch_started_{};
    std::atomic<std::uint64_t> frames_published_{0};
    std::atomic<std::uint64_t> detection_checks_{0};
    std::atomic<std::uint64_t> full_scans_{0};
    std::atomic<std::uint64_t> recovery_attempts_{0};
    std::atomic<std::uint64_t> failures_{0};
};

} // namespace asc::win
