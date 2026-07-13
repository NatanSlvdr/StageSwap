#pragma once

#include "common.hpp"
#include "asc/core/config.hpp"
#include "device_enumerator.hpp"

#include <memory>
#include <vector>

namespace asc::win {

class App;
class PreviewWindow;
class SettingsWindow {
public:
    static void show(HWND owner, HINSTANCE instance, App& app);
private:
    SettingsWindow(HWND owner, HINSTANCE instance, App& app);
    ~SettingsWindow();
    static LRESULT CALLBACK procedure(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
    LRESULT handle(UINT message, WPARAM wparam, LPARAM lparam);
    void create_controls();
    void update_device_details();
    void show_previews();
    void export_logs();
    void save();
    HWND add_label(const wchar_t* text, int x, int y, int width = 210);
    HWND add_edit(UINT id, const std::wstring& value, int x, int y, int width = 90);
    HWND add_checkbox(UINT id, const wchar_t* text, bool checked, int x, int y, int width = 250);
    HWND add_combo(UINT id, int x, int y, int width = 250);
    [[nodiscard]] std::wstring text(UINT id) const;
    [[nodiscard]] std::uint32_t integer(UINT id, std::uint32_t minimum, std::uint32_t maximum) const;
    [[nodiscard]] double decimal(UINT id, double minimum, double maximum) const;
    [[nodiscard]] bool checked(UINT id) const;
    HWND owner_;
    HINSTANCE instance_;
    App& app_;
    HWND window_{nullptr};
    AppConfig working_;
    std::vector<VideoDevice> devices_;
    std::vector<std::string> video_option_ids_;
    std::vector<MonitorDevice> monitors_;
    std::vector<MonitorObservation> monitor_observations_;
    std::unique_ptr<PreviewWindow> previews_;
    bool finished_{false};
    int scroll_position_{0};
    static constexpr int content_height_{875};
};

} // namespace asc::win
