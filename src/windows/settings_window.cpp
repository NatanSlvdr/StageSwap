#include "settings_window.hpp"
#include "app.hpp"

#include <commctrl.h>
#include <algorithm>
#include <sstream>
#include <iomanip>
#include <cstdlib>
#include <stdexcept>

namespace asc::win {
namespace {
enum Id : UINT {
    video_device = 1000, input_size, input_fps, auto_reconnect, reconnect_interval, cursor, tracked_monitor, camera_scale, screen_scale,
    threshold, detection_interval, matches, mismatches, scan_interval, reassignments, missing_behavior,
    output_size, output_fps, fade, placeholder_color, start_windows, start_minimized, start_auto, close_tray, confirm_exit, diagnostic, notifications, language, log_retention, configured_log_level,
    save_button, cancel_button, restart_input_button, restart_capture_button, restart_vcam_button,
    set_reference_settings, import_reference_settings, reset_counters_button
};
void combo_add(const HWND combo, const wchar_t* text) { SendMessageW(combo, CB_ADDSTRING, 0, reinterpret_cast<LPARAM>(text)); }
int combo_selection(const HWND window, const UINT id) { return static_cast<int>(SendDlgItemMessageW(window, id, CB_GETCURSEL, 0, 0)); }
}

void SettingsWindow::show(const HWND owner, const HINSTANCE instance, App& app) {
    SettingsWindow dialog(owner, instance, app);
    EnableWindow(owner, FALSE);
    ShowWindow(dialog.window_, SW_SHOW);
    MSG message{};
    while (!dialog.finished_ && GetMessageW(&message, nullptr, 0, 0) > 0) {
        if (!IsDialogMessageW(dialog.window_, &message)) { TranslateMessage(&message); DispatchMessageW(&message); }
    }
    EnableWindow(owner, TRUE);
    SetForegroundWindow(owner);
}

SettingsWindow::SettingsWindow(const HWND owner, const HINSTANCE instance, App& app)
    : owner_(owner), instance_(instance), app_(app), working_(app.config()), devices_(app.video_devices()), monitors_(app.monitors()) {
    WNDCLASSEXW wc{sizeof(wc)}; wc.hInstance = instance_; wc.lpfnWndProc = procedure; wc.lpszClassName = L"AutomaticScreenCameraSettings";
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW); wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    RegisterClassExW(&wc);
    window_ = CreateWindowExW(WS_EX_DLGMODALFRAME, wc.lpszClassName, L"Automatic Screen Camera — Settings",
                              WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VSCROLL, CW_USEDEFAULT, CW_USEDEFAULT, 735, 680,
                              owner_, nullptr, instance_, this);
    if (!window_) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "Create settings window");
}

LRESULT CALLBACK SettingsWindow::procedure(const HWND window, const UINT message, const WPARAM wparam, const LPARAM lparam) {
    auto* self = reinterpret_cast<SettingsWindow*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) { self = static_cast<SettingsWindow*>(reinterpret_cast<CREATESTRUCTW*>(lparam)->lpCreateParams); self->window_ = window; SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self)); }
    return self ? self->handle(message, wparam, lparam) : DefWindowProcW(window, message, wparam, lparam);
}

LRESULT SettingsWindow::handle(const UINT message, const WPARAM wparam, const LPARAM lparam) {
    switch (message) {
    case WM_CREATE: {
        create_controls();
        RECT client{}; GetClientRect(window_, &client);
        SCROLLINFO info{sizeof(info), SIF_RANGE | SIF_PAGE | SIF_POS, 0, content_height_, static_cast<UINT>(client.bottom), 0, 0};
        SetScrollInfo(window_, SB_VERT, &info, TRUE);
        return 0;
    }
    case WM_SIZE: {
        SCROLLINFO info{sizeof(info), SIF_PAGE}; info.nPage = HIWORD(lparam);
        SetScrollInfo(window_, SB_VERT, &info, TRUE);
        return 0;
    }
    case WM_VSCROLL: {
        SCROLLINFO info{sizeof(info), SIF_ALL}; GetScrollInfo(window_, SB_VERT, &info);
        int next = scroll_position_;
        switch (LOWORD(wparam)) {
        case SB_LINEUP: next -= 24; break; case SB_LINEDOWN: next += 24; break;
        case SB_PAGEUP: next -= static_cast<int>(info.nPage); break; case SB_PAGEDOWN: next += static_cast<int>(info.nPage); break;
        case SB_THUMBTRACK: next = info.nTrackPos; break; case SB_TOP: next = 0; break; case SB_BOTTOM: next = info.nMax; break;
        }
        next = std::clamp(next, 0, std::max(0, info.nMax - static_cast<int>(info.nPage) + 1));
        if (next != scroll_position_) {
            const int delta = scroll_position_ - next; scroll_position_ = next;
            ScrollWindowEx(window_, 0, delta, nullptr, nullptr, nullptr, nullptr, SW_INVALIDATE | SW_ERASE | SW_SCROLLCHILDREN);
            info.fMask = SIF_POS; info.nPos = scroll_position_; SetScrollInfo(window_, SB_VERT, &info, TRUE);
            UpdateWindow(window_);
        }
        return 0;
    }
    case WM_MOUSEWHEEL: {
        const int steps = GET_WHEEL_DELTA_WPARAM(wparam) / WHEEL_DELTA;
        for (int i = 0; i < std::abs(steps) * 3; ++i)
            SendMessageW(window_, WM_VSCROLL, steps > 0 ? SB_LINEUP : SB_LINEDOWN, 0);
        return 0;
    }
    case WM_COMMAND:
        switch (LOWORD(wparam)) {
        case save_button: save(); return 0;
        case cancel_button: SendMessageW(window_, WM_CLOSE, 0, 0); return 0;
        case restart_input_button: app_.restart_video_input(); return 0;
        case restart_capture_button: app_.restart_screen_capture(); return 0;
        case restart_vcam_button: app_.restart_virtual_camera(); return 0;
        case reset_counters_button: app_.reset_diagnostic_counters(); return 0;
        case set_reference_settings: {
            const auto monitors = app_.monitors();
            std::optional<MonitorIdentity> selected_monitor;
            if (monitors.size() > 1) {
                HMENU choices = CreatePopupMenu();
                for (std::size_t i = 0; i < monitors.size(); ++i) {
                    std::wostringstream label; label << wide(monitors[i].identity.model) << L" — " << monitors[i].identity.resolution.width << L"×" << monitors[i].identity.resolution.height;
                    AppendMenuW(choices, MF_STRING, static_cast<UINT_PTR>(3000 + i), label.str().c_str());
                }
                POINT point{}; GetCursorPos(&point);
                const auto choice = TrackPopupMenu(choices, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, 0, window_, nullptr);
                DestroyMenu(choices);
                if (choice >= 3000 && static_cast<std::size_t>(choice - 3000) < monitors.size()) selected_monitor = monitors[static_cast<std::size_t>(choice - 3000)].identity;
            } else if (monitors.size() == 1) selected_monitor = monitors.front().identity;
            DestroyWindow(window_);
            if (selected_monitor) app_.set_reference_monitor(*selected_monitor);
            return 0;
        }
        case import_reference_settings: {
            wchar_t file[MAX_PATH]{}; OPENFILENAMEW dialog{sizeof(dialog)}; dialog.hwndOwner = window_; dialog.lpstrFile = file; dialog.nMaxFile = ARRAYSIZE(file);
            dialog.lpstrFilter = L"Images (*.png;*.jpg;*.jpeg;*.bmp)\0*.png;*.jpg;*.jpeg;*.bmp\0All files\0*.*\0"; dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
            if (GetOpenFileNameW(&dialog)) app_.import_reference(file);
            return 0;
        }
        }
        break;
    case WM_CLOSE: DestroyWindow(window_); return 0;
    case WM_DESTROY: finished_ = true; return 0;
    }
    return DefWindowProcW(window_, message, wparam, lparam);
}

HWND SettingsWindow::add_label(const wchar_t* value, const int x, const int y, const int width) {
    return CreateWindowExW(0, L"STATIC", value, WS_CHILD | WS_VISIBLE, x, y + 4, width, 22, window_, nullptr, instance_, nullptr);
}
HWND SettingsWindow::add_edit(const UINT id, const std::wstring& value, const int x, const int y, const int width) {
    return CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", value.c_str(), WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
                           x, y, width, 25, window_, reinterpret_cast<HMENU>(id), instance_, nullptr);
}
HWND SettingsWindow::add_checkbox(const UINT id, const wchar_t* value, const bool is_checked, const int x, const int y, const int width) {
    const auto control = CreateWindowExW(0, L"BUTTON", value, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
                                         x, y, width, 24, window_, reinterpret_cast<HMENU>(id), instance_, nullptr);
    SendMessageW(control, BM_SETCHECK, is_checked ? BST_CHECKED : BST_UNCHECKED, 0); return control;
}
HWND SettingsWindow::add_combo(const UINT id, const int x, const int y, const int width) {
    return CreateWindowExW(0, WC_COMBOBOXW, nullptr, WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
                           x, y, width, 160, window_, reinterpret_cast<HMENU>(id), instance_, nullptr);
}

void SettingsWindow::create_controls() {
    HFONT title_font = static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
    add_label(L"VIDEO INPUT", 20, 16, 300);
    add_label(L"Device", 20, 48); const auto device = add_combo(video_device, 195, 44, 490);
    int device_selected = 0; combo_add(device, L"No video input selected");
    for (std::size_t i = 0; i < devices_.size(); ++i) {
        std::wostringstream description;
        description << wide(devices_[i].name);
        if (!devices_[i].formats.empty()) {
            const auto& format = devices_[i].formats.front();
            description << L" — " << format.size.width << L"×" << format.size.height << L" @ "
                        << (format.denominator ? format.numerator / format.denominator : 0) << L" fps";
        }
        combo_add(device, description.str().c_str());
        if (devices_[i].identifier == working_.selected_video_device_id) device_selected = static_cast<int>(i + 1);
    }
    SendMessageW(device, CB_SETCURSEL, device_selected, 0);
    add_label(L"Preferred input", 20, 82); const auto in_size = add_combo(input_size, 195, 78, 160); combo_add(in_size, L"1920 × 1080"); combo_add(in_size, L"1280 × 720");
    SendMessageW(in_size, CB_SETCURSEL, working_.preferred_input_size.width == 1280 ? 1 : 0, 0);
    add_label(L"Frame rate", 380, 82, 100); const auto in_fps = add_combo(input_fps, 500, 78, 100); combo_add(in_fps, L"30 fps"); SendMessageW(in_fps, CB_SETCURSEL, 0, 0);
    CreateWindowExW(0, L"BUTTON", L"Restart input", WS_CHILD | WS_VISIBLE, 605, 78, 80, 26, window_, reinterpret_cast<HMENU>(restart_input_button), instance_, nullptr);
    add_checkbox(auto_reconnect, L"Reconnect automatically", working_.video_auto_reconnect, 20, 108, 220);
    add_label(L"Retry interval (s)", 380, 108, 150); add_edit(reconnect_interval, std::to_wstring(working_.video_reconnect_interval.count()), 575, 104, 110);

    add_label(L"SCREEN CAPTURE", 20, 142, 300);
    add_checkbox(cursor, L"Include mouse cursor", working_.cursor_visible, 20, 170);
    CreateWindowExW(0, L"BUTTON", L"Restart capture", WS_CHILD | WS_VISIBLE, 555, 166, 130, 28, window_, reinterpret_cast<HMENU>(restart_capture_button), instance_, nullptr);
    add_label(L"Preferred tracked display", 20, 204); const auto monitor_combo = add_combo(tracked_monitor, 240, 200, 445);
    combo_add(monitor_combo, L"Keep automatic/reference-based selection");
    int monitor_selected = 0;
    for (std::size_t i = 0; i < monitors_.size(); ++i) {
        std::wostringstream description; description << wide(monitors_[i].identity.model) << L" — "
            << monitors_[i].identity.resolution.width << L"×" << monitors_[i].identity.resolution.height << L" at ("
            << monitors_[i].identity.desktop_x << L", " << monitors_[i].identity.desktop_y << L")";
        combo_add(monitor_combo, description.str().c_str());
        if (working_.last_tracked_monitor && monitors_[i].identity.stable_key() == working_.last_tracked_monitor->stable_key())
            monitor_selected = static_cast<int>(i + 1);
    }
    SendMessageW(monitor_combo, CB_SETCURSEL, monitor_selected, 0);

    add_label(L"REFERENCE DETECTION", 20, 240, 300);
    CreateWindowExW(0, L"BUTTON", L"Set current screen", WS_CHILD | WS_VISIBLE, 370, 234, 145, 27, window_, reinterpret_cast<HMENU>(set_reference_settings), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Import image…", WS_CHILD | WS_VISIBLE, 525, 234, 160, 27, window_, reinterpret_cast<HMENU>(import_reference_settings), instance_, nullptr);
    add_label(L"Similarity threshold (%)", 20, 272); add_edit(threshold, std::to_wstring(working_.detector.threshold * 100.0), 240, 268);
    add_label(L"Detection interval (ms)", 380, 272, 180); add_edit(detection_interval, std::to_wstring(working_.detection_interval.count()), 575, 268, 110);
    add_label(L"Matching confirmations", 20, 306); add_edit(matches, std::to_wstring(working_.detector.matches_required), 240, 302);
    add_label(L"Mismatch confirmations", 380, 306, 180); add_edit(mismatches, std::to_wstring(working_.detector.mismatches_required), 575, 302, 110);
    add_label(L"Full rescan interval (s)", 20, 340); add_edit(scan_interval, std::to_wstring(working_.full_scan_interval.count()), 240, 336);
    add_label(L"Reassignment confirmations", 380, 340, 190); add_edit(reassignments, std::to_wstring(working_.monitor_tracker.confirmations_required), 575, 336, 110);
    add_label(L"When reference is missing", 20, 374); const auto missing = add_combo(missing_behavior, 240, 370, 445);
    combo_add(missing, L"Use webcam/video (safe default)"); combo_add(missing, L"Keep current output"); combo_add(missing, L"Use last tracked screen"); combo_add(missing, L"Use safe placeholder");
    SendMessageW(missing, CB_SETCURSEL, static_cast<int>(working_.missing_behavior), 0);

    add_label(L"OUTPUT", 20, 414, 300);
    add_label(L"Resolution", 20, 446); const auto out_size = add_combo(output_size, 195, 442, 160); combo_add(out_size, L"1920 × 1080"); combo_add(out_size, L"1280 × 720");
    SendMessageW(out_size, CB_SETCURSEL, working_.output_size.width == 1280 ? 1 : 0, 0);
    add_label(L"Frame rate", 380, 446, 100); const auto out_fps = add_combo(output_fps, 500, 442, 90); combo_add(out_fps, L"30 fps"); SendMessageW(out_fps, CB_SETCURSEL, 0, 0);
    CreateWindowExW(0, L"BUTTON", L"Restart virtual camera", WS_CHILD | WS_VISIBLE, 596, 442, 90, 27, window_, reinterpret_cast<HMENU>(restart_vcam_button), instance_, nullptr);
    add_label(L"Fade duration (0–2000 ms)", 20, 480); add_edit(fade, std::to_wstring(working_.fade_duration.count()), 240, 476);
    add_label(L"Webcam scaling", 380, 480, 130); const auto camera = add_combo(camera_scale, 515, 476, 170);
    combo_add(camera, L"Fit with letterboxing"); combo_add(camera, L"Fill and crop"); combo_add(camera, L"Stretch"); SendMessageW(camera, CB_SETCURSEL, static_cast<int>(working_.camera_scaling), 0);
    add_label(L"Screen scaling", 380, 514, 130); const auto screen = add_combo(screen_scale, 515, 510, 170);
    combo_add(screen, L"Fit with letterboxing"); combo_add(screen, L"Fill and crop"); combo_add(screen, L"Stretch"); SendMessageW(screen, CB_SETCURSEL, static_cast<int>(working_.screen_scaling), 0);
    std::wostringstream color_text; color_text << L'#' << std::hex << std::setfill(L'0') << std::setw(6) << (working_.placeholder_color_bgra & 0x00ffffffu);
    add_label(L"Placeholder color", 20, 514); add_edit(placeholder_color, color_text.str(), 195, 510, 120);

    add_label(L"GENERAL AND LOGGING", 20, 550, 300);
    add_checkbox(start_windows, L"Start with Windows", working_.start_with_windows, 20, 578, 190);
    add_checkbox(start_minimized, L"Start minimized to tray", working_.start_minimized, 220, 578, 210);
    add_checkbox(start_auto, L"Start detection automatically", working_.start_automatically, 440, 578, 245);
    add_checkbox(close_tray, L"Close button minimizes to tray", working_.close_to_tray, 20, 604, 240);
    add_checkbox(confirm_exit, L"Confirm before exiting", working_.confirm_exit, 270, 604, 190);
    add_checkbox(diagnostic, L"Diagnostic logging", working_.diagnostic_logging, 470, 604, 190);
    add_checkbox(notifications, L"Show Windows notifications", working_.show_notifications, 20, 630, 240);
    add_label(L"Interface language", 280, 630, 150); const auto languages = add_combo(language, 440, 626, 245);
    combo_add(languages, L"English (United States)"); SendMessageW(languages, CB_SETCURSEL, 0, 0);
    add_label(L"Log retention (days)", 20, 660, 180); add_edit(log_retention, std::to_wstring(working_.log_retention_days), 205, 656, 90);
    add_label(L"Log level", 380, 660, 100); const auto levels = add_combo(configured_log_level, 500, 656, 185);
    combo_add(levels, L"Trace"); combo_add(levels, L"Debug"); combo_add(levels, L"Info"); combo_add(levels, L"Warning"); combo_add(levels, L"Error");
    SendMessageW(levels, CB_SETCURSEL, static_cast<int>(working_.log_level), 0);

    CreateWindowExW(0, L"BUTTON", L"Reset diagnostic counters", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 20, 705, 210, 30, window_, reinterpret_cast<HMENU>(reset_counters_button), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Save", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON, 505, 705, 85, 30, window_, reinterpret_cast<HMENU>(save_button), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Cancel", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 600, 705, 85, 30, window_, reinterpret_cast<HMENU>(cancel_button), instance_, nullptr);
    SendMessageW(window_, WM_SETFONT, reinterpret_cast<WPARAM>(title_font), TRUE);
}

std::wstring SettingsWindow::text(const UINT id) const { wchar_t value[256]{}; GetDlgItemTextW(window_, id, value, ARRAYSIZE(value)); return value; }
std::uint32_t SettingsWindow::integer(const UINT id, const std::uint32_t minimum, const std::uint32_t maximum) const {
    std::size_t consumed = 0; const auto value = std::stoul(text(id), &consumed);
    if (consumed == 0 || value < minimum || value > maximum) throw std::invalid_argument("A numeric setting is outside its allowed range");
    return static_cast<std::uint32_t>(value);
}
double SettingsWindow::decimal(const UINT id, const double minimum, const double maximum) const {
    std::size_t consumed = 0; const auto value = std::stod(text(id), &consumed);
    if (consumed == 0 || value < minimum || value > maximum) throw std::invalid_argument("A decimal setting is outside its allowed range");
    return value;
}
bool SettingsWindow::checked(const UINT id) const { return SendDlgItemMessageW(window_, id, BM_GETCHECK, 0, 0) == BST_CHECKED; }

void SettingsWindow::save() {
    try {
        const int device = combo_selection(window_, video_device);
        working_.selected_video_device_id = device > 0 && static_cast<std::size_t>(device) <= devices_.size() ? devices_[static_cast<std::size_t>(device - 1)].identifier : "";
        working_.preferred_input_size = combo_selection(window_, input_size) == 1 ? Size{1280, 720} : Size{1920, 1080};
        working_.preferred_input_fps = 30;
        working_.video_auto_reconnect = checked(auto_reconnect);
        working_.video_reconnect_interval = std::chrono::seconds{integer(reconnect_interval, 1, 60)};
        working_.cursor_visible = checked(cursor);
        const int monitor = combo_selection(window_, tracked_monitor);
        if (monitor > 0 && static_cast<std::size_t>(monitor) <= monitors_.size())
            working_.last_tracked_monitor = monitors_[static_cast<std::size_t>(monitor - 1)].identity;
        working_.detector.threshold = decimal(threshold, 0.0, 100.0) / 100.0;
        working_.detection_interval = std::chrono::milliseconds{integer(detection_interval, 100, 1000)};
        working_.detector.matches_required = integer(matches, 1, 30);
        working_.detector.mismatches_required = integer(mismatches, 1, 30);
        working_.full_scan_interval = std::chrono::seconds{integer(scan_interval, 5, 3600)};
        working_.monitor_tracker.confirmations_required = integer(reassignments, 1, 10);
        working_.missing_behavior = static_cast<MissingReferenceBehavior>(std::max(0, combo_selection(window_, missing_behavior)));
        working_.output_size = combo_selection(window_, output_size) == 1 ? Size{1280, 720} : Size{1920, 1080};
        working_.output_fps = 30;
        working_.fade_duration = std::chrono::milliseconds{integer(fade, 0, 2000)};
        working_.camera_scaling = static_cast<ScalingMode>(std::max(0, combo_selection(window_, camera_scale)));
        working_.screen_scaling = static_cast<ScalingMode>(std::max(0, combo_selection(window_, screen_scale)));
        { auto color = text(placeholder_color); if (!color.empty() && color.front() == L'#') color.erase(color.begin());
          std::size_t used = 0; const auto rgb = std::stoul(color, &used, 16); if (used != 6 || color.size() != 6) throw std::invalid_argument("Placeholder color must use #RRGGBB format");
          working_.placeholder_color_bgra = 0xff000000u | static_cast<std::uint32_t>(rgb); }
        working_.start_with_windows = checked(start_windows); working_.start_minimized = checked(start_minimized);
        working_.start_automatically = checked(start_auto); working_.close_to_tray = checked(close_tray);
        working_.confirm_exit = checked(confirm_exit); working_.diagnostic_logging = checked(diagnostic);
        working_.show_notifications = checked(notifications); working_.interface_language = "en-US";
        working_.log_retention_days = integer(log_retention, 1, 365);
        working_.log_level = static_cast<LogLevel>(std::max(0, combo_selection(window_, configured_log_level)));
        app_.apply_settings(working_);
        DestroyWindow(window_);
    } catch (const std::exception& error) { MessageBoxA(window_, error.what(), "Invalid settings", MB_OK | MB_ICONWARNING); }
}

} // namespace asc::win
