#pragma once

#include "common.hpp"
#include "status_presentation.hpp"
#include "ui_helpers.hpp"
#include "asc/core/deferred_trigger.hpp"

#include <shellapi.h>
#include <array>
#include <memory>
#include <vector>

namespace asc::win {

class App;
class PreviewWindow;
struct PreviewImage;

class TrayWindow {
public:
    TrayWindow(HINSTANCE instance, App& app);
    ~TrayWindow();
    int message_loop();
    void show();
    void hide();
    void close();

private:
    static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
    LRESULT handle_message(UINT message, WPARAM wparam, LPARAM lparam);
    void create_controls();
    void layout_controls();
    void refresh();
    void refresh_preview();
    void update_technical_details();
    void set_details_expanded(bool expanded, bool resize_window = true);
    void show_tray_menu(POINT point);
    void dispatch_command(UINT command);
    void update_tray_icon();
    void schedule_lifecycle_recovery(std::string code, std::string message);
    [[nodiscard]] HICON make_status_icon(COLORREF color) const;
    [[nodiscard]] HBITMAP create_preview_bitmap(const PreviewImage& image) const;
    HINSTANCE instance_;
    App& app_;
    HWND window_{nullptr};
    HWND title_label_{nullptr};
    HWND run_label_{nullptr};
    HWND warning_banner_{nullptr};
    HWND override_banner_{nullptr};
    HWND return_automatic_button_{nullptr};
    HWND start_stop_button_{nullptr};
    HWND output_heading_{nullptr};
    HWND output_preview_{nullptr};
    HWND output_kind_{nullptr};
    HWND output_name_{nullptr};
    HWND output_info_{nullptr};
    HWND mode_heading_{nullptr};
    std::array<HWND, 3> mode_buttons_{};
    HWND reference_caption_{nullptr};
    HWND reference_value_{nullptr};
    HWND reference_info_{nullptr};
    HWND display_caption_{nullptr};
    HWND display_value_{nullptr};
    HWND display_info_{nullptr};
    HWND health_caption_{nullptr};
    HWND health_value_{nullptr};
    HWND health_info_{nullptr};
    HWND set_reference_button_{nullptr};
    HWND rescan_button_{nullptr};
    HWND activity_heading_{nullptr};
    HWND view_all_button_{nullptr};
    std::array<HWND, 3> activity_lines_{};
    HWND previews_button_{nullptr};
    HWND settings_button_{nullptr};
    HWND details_button_{nullptr};
    HWND technical_text_{nullptr};
    HWND full_activity_list_{nullptr};
    NOTIFYICONDATAW tray_{};
    HICON status_icon_{nullptr};
    HICON app_icon_large_{nullptr};
    HICON app_icon_small_{nullptr};
    HBITMAP output_bitmap_{nullptr};
    std::unique_ptr<UiFonts> fonts_;
    TooltipHost tooltips_;
    DashboardPresentation presentation_;
    std::vector<std::string> displayed_activity_;
    std::string displayed_technical_details_;
    bool details_expanded_{false};
    bool exiting_{false};
    DeferredTrigger lifecycle_recovery_;
    std::string last_notified_warning_;
    std::unique_ptr<PreviewWindow> previews_;
};

} // namespace asc::win
