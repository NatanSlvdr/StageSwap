#pragma once

#include "common.hpp"
#include "ui_helpers.hpp"
#include "asc/core/config.hpp"
#include "device_enumerator.hpp"

#include <array>
#include <map>
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
    void layout_controls();
    void select_tab(int index);
    void set_device_details_expanded(bool expanded);
    void update_device_details();
    void show_previews();
    void export_logs();
    void save();
    HWND add_label(const wchar_t* text, int page, int x, int y, int width = 210, int height = 24, bool stretch = false);
    HWND add_edit(UINT id, const std::wstring& value, int page, int x, int y, int width = 90, bool stretch = false);
    HWND add_checkbox(UINT id, const wchar_t* text, bool checked, int page, int x, int y, int width = 250);
    HWND add_combo(UINT id, int page, int x, int y, int width = 250, bool stretch = false);
    HWND add_button(UINT id, const wchar_t* text, int page, int x, int y, int width, int height = 30);
    void register_control(HWND control, int page, int x, int y, int width, int height, bool stretch = false);
    [[nodiscard]] std::wstring text(UINT id) const;
    [[nodiscard]] std::uint32_t integer(UINT id, std::uint32_t minimum, std::uint32_t maximum) const;
    [[nodiscard]] double decimal(UINT id, double minimum, double maximum) const;
    [[nodiscard]] bool checked(UINT id) const;
    HWND owner_;
    HINSTANCE instance_;
    App& app_;
    HWND window_{nullptr};
    HWND tabs_{nullptr};
    HWND save_button_{nullptr};
    HWND cancel_button_{nullptr};
    HWND device_status_{nullptr};
    HWND device_details_button_{nullptr};
    AppConfig working_;
    std::vector<VideoDevice> devices_;
    std::vector<std::string> video_option_ids_;
    std::vector<MonitorDevice> monitors_;
    std::vector<MonitorObservation> monitor_observations_;
    std::unique_ptr<PreviewWindow> previews_;
    std::unique_ptr<UiFonts> fonts_;
    TooltipHost tooltips_;
    struct Placement { int page; int x; int y; int width; int height; bool stretch; };
    std::map<HWND, Placement> placements_;
    std::array<std::vector<HWND>, 5> page_controls_;
    std::vector<HWND> section_labels_;
    int selected_tab_{0};
    bool device_details_expanded_{false};
    bool finished_{false};
};

} // namespace asc::win
