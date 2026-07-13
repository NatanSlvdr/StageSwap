#pragma once

#include "common.hpp"
#include "asc/core/deferred_trigger.hpp"

#include <shellapi.h>
#include <memory>

namespace asc::win {

class App;
class PreviewWindow;

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
    void refresh();
    void show_tray_menu(POINT point);
    void dispatch_command(UINT command);
    void update_tray_icon();
    void schedule_lifecycle_recovery(std::string code, std::string message);
    [[nodiscard]] HICON make_status_icon(COLORREF color) const;
    HINSTANCE instance_;
    App& app_;
    HWND window_{nullptr};
    HWND override_banner_{nullptr};
    HWND status_text_{nullptr};
    HWND recent_list_{nullptr};
    HWND start_stop_button_{nullptr};
    NOTIFYICONDATAW tray_{};
    HICON status_icon_{nullptr};
    bool exiting_{false};
    DeferredTrigger lifecycle_recovery_;
    std::string last_notified_warning_;
    std::unique_ptr<PreviewWindow> previews_;
};

} // namespace asc::win
