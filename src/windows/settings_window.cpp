#include "settings_window.hpp"
#include "app.hpp"
#include "preview_window.hpp"
#include "asc/core/device_choices.hpp"

#include <commctrl.h>
#include <commdlg.h>
#include <shellapi.h>
#include <algorithm>
#include <sstream>
#include <iomanip>
#include <cstdlib>
#include <stdexcept>

namespace asc::win {
namespace {
enum Id : UINT {
    video_device = 1000, video_details, input_size, input_fps, auto_reconnect, reconnect_interval, cursor, tracked_monitor, camera_scale, screen_scale,
    threshold, detection_interval, matches, mismatches, scan_interval, reassignments, missing_behavior,
    output_size, output_fps, fade, placeholder_color, start_windows, start_minimized, start_auto, close_tray, confirm_exit, diagnostic, notifications, language, log_retention, configured_log_level,
    save_button, cancel_button, restart_input_button, restart_capture_button, restart_vcam_button,
    set_reference_settings, import_reference_settings, reset_counters_button, preview_button,
    open_log_button, export_log_button, clear_log_button
};
void combo_add(const HWND combo, const wchar_t* text) { SendMessageW(combo, CB_ADDSTRING, 0, reinterpret_cast<LPARAM>(text)); }
int combo_selection(const HWND window, const UINT id) { return static_cast<int>(SendDlgItemMessageW(window, id, CB_GETCURSEL, 0, 0)); }
HMENU control_id(const UINT id) noexcept { return reinterpret_cast<HMENU>(static_cast<INT_PTR>(id)); }
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

SettingsWindow::~SettingsWindow() = default;

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
        case video_device:
            if (HIWORD(wparam) == CBN_SELCHANGE) update_device_details();
            return 0;
        case save_button: save(); return 0;
        case cancel_button: SendMessageW(window_, WM_CLOSE, 0, 0); return 0;
        case restart_input_button: app_.restart_video_input(); return 0;
        case restart_capture_button: app_.restart_screen_capture(); return 0;
        case restart_vcam_button: app_.restart_virtual_camera(); return 0;
        case reset_counters_button: app_.reset_diagnostic_counters(); return 0;
        case preview_button: show_previews(); return 0;
        case open_log_button:
            ShellExecuteW(window_, L"open", (app_.data_directory() / L"logs").c_str(), nullptr, nullptr, SW_SHOWNORMAL);
            return 0;
        case export_log_button: export_logs(); return 0;
        case clear_log_button:
            if (MessageBoxW(window_, L"Clear all diagnostic logs?", L"Clear logs", MB_YESNO | MB_ICONWARNING) == IDYES)
                app_.clear_logs();
            return 0;
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
                           x, y, width, 25, window_, control_id(id), instance_, nullptr);
}
HWND SettingsWindow::add_checkbox(const UINT id, const wchar_t* value, const bool is_checked, const int x, const int y, const int width) {
    const auto control = CreateWindowExW(0, L"BUTTON", value, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
                                         x, y, width, 24, window_, control_id(id), instance_, nullptr);
    SendMessageW(control, BM_SETCHECK, is_checked ? BST_CHECKED : BST_UNCHECKED, 0); return control;
}
HWND SettingsWindow::add_combo(const UINT id, const int x, const int y, const int width) {
    return CreateWindowExW(0, WC_COMBOBOXW, nullptr, WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
                           x, y, width, 160, window_, control_id(id), instance_, nullptr);
}

void SettingsWindow::update_device_details() {
    const int selected = combo_selection(window_, video_device);
    std::wostringstream details;
    if (selected <= 0 || static_cast<std::size_t>(selected) >= video_option_ids_.size()) {
        details << L"Status: No video input selected";
    } else {
        const auto& identifier = video_option_ids_[static_cast<std::size_t>(selected)];
        const auto device = std::find_if(devices_.begin(), devices_.end(), [&identifier](const VideoDevice& candidate) {
            return candidate.identifier == identifier;
        });
        if (device == devices_.end()) {
            details << L"Status: Unavailable (saved selection)\r\nIdentifier: " << wide(identifier)
                    << L"\r\nSupported formats: unavailable until the device reconnects";
        } else {
            details << L"Status: " << (device->connected ? L"Connected" : L"Unavailable")
                    << L"\r\nIdentifier: " << wide(device->identifier) << L"\r\nSupported formats: ";
            if (device->formats.empty()) {
                details << L"Not reported by the device";
            } else {
                constexpr std::size_t displayed_format_limit = 8;
                for (std::size_t i = 0; i < std::min(device->formats.size(), displayed_format_limit); ++i) {
                    const auto& format = device->formats[i];
                    if (i != 0) details << L"; ";
                    details << format.size.width << L'\u00d7' << format.size.height << L" @ ";
                    if (format.denominator != 0) {
                        const double fps = static_cast<double>(format.numerator) / static_cast<double>(format.denominator);
                        details << std::fixed << std::setprecision(fps == static_cast<std::uint32_t>(fps) ? 0 : 2) << fps;
                    } else {
                        details << L'?';
                    }
                    details << L" fps (" << wide(format.subtype) << L')';
                }
                if (device->formats.size() > displayed_format_limit)
                    details << L"; +" << (device->formats.size() - displayed_format_limit) << L" more";
            }
        }
    }
    SetWindowTextW(GetDlgItem(window_, video_details), details.str().c_str());
}

void SettingsWindow::show_previews() {
    if (!previews_) previews_ = std::make_unique<PreviewWindow>(instance_, app_);
    previews_->show();
}

void SettingsWindow::export_logs() {
    wchar_t file[MAX_PATH] = L"AutomaticScreenCamera-logs.jsonl";
    OPENFILENAMEW dialog{sizeof(dialog)};
    dialog.hwndOwner = window_;
    dialog.lpstrFile = file;
    dialog.nMaxFile = ARRAYSIZE(file);
    dialog.lpstrFilter = L"JSON Lines (*.jsonl)\0*.jsonl\0All files\0*.*\0";
    dialog.lpstrDefExt = L"jsonl";
    dialog.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
    if (!GetSaveFileNameW(&dialog)) return;
    try {
        app_.export_logs(file);
    } catch (const std::exception& error) {
        MessageBoxA(window_, error.what(), "Log export failed", MB_OK | MB_ICONERROR);
    }
}

void SettingsWindow::create_controls() {
    HFONT title_font = static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
    add_label(L"VIDEO INPUT", 20, 16, 300);
    add_label(L"Device", 20, 48); const auto device = add_combo(video_device, 195, 44, 490);
    std::vector<std::string> available_device_ids;
    available_device_ids.reserve(devices_.size());
    for (const auto& available : devices_) available_device_ids.push_back(available.identifier);
    const auto choices = build_persistent_device_choices(available_device_ids, working_.selected_video_device_id);
    video_option_ids_ = choices.identifiers;
    combo_add(device, L"No video input selected");
    for (std::size_t i = 0; i < devices_.size(); ++i) {
        std::wostringstream description;
        description << wide(devices_[i].name);
        if (!devices_[i].formats.empty()) {
            const auto& format = devices_[i].formats.front();
            description << L" — " << format.size.width << L"×" << format.size.height << L" @ "
                        << (format.denominator ? format.numerator / format.denominator : 0) << L" fps";
        }
        combo_add(device, description.str().c_str());
    }
    if (choices.configured_device_unavailable) {
        const auto unavailable = std::wstring(L"Unavailable saved source — ") + wide(working_.selected_video_device_id);
        combo_add(device, unavailable.c_str());
    }
    SendMessageW(device, CB_SETCURSEL, static_cast<WPARAM>(choices.selected_index), 0);
    add_label(L"Device details", 20, 78);
    CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", nullptr,
                    WS_CHILD | WS_VISIBLE | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL,
                    195, 74, 490, 72, window_, control_id(video_details), instance_, nullptr);
    update_device_details();
    add_label(L"Preferred input", 20, 162); const auto in_size = add_combo(input_size, 195, 158, 160); combo_add(in_size, L"1920 × 1080"); combo_add(in_size, L"1280 × 720");
    SendMessageW(in_size, CB_SETCURSEL, working_.preferred_input_size.width == 1280 ? 1 : 0, 0);
    add_label(L"Frame rate", 380, 162, 100); const auto in_fps = add_combo(input_fps, 500, 158, 100); combo_add(in_fps, L"30 fps"); SendMessageW(in_fps, CB_SETCURSEL, 0, 0);
    CreateWindowExW(0, L"BUTTON", L"Restart input", WS_CHILD | WS_VISIBLE, 605, 158, 80, 26, window_, control_id(restart_input_button), instance_, nullptr);
    add_checkbox(auto_reconnect, L"Reconnect automatically", working_.video_auto_reconnect, 20, 188, 220);
    add_label(L"Retry interval (s)", 265, 188, 135); add_edit(reconnect_interval, std::to_wstring(working_.video_reconnect_interval.count()), 400, 184, 70);
    CreateWindowExW(0, L"BUTTON", L"Open previews", WS_CHILD | WS_VISIBLE, 480, 184, 110, 27, window_, control_id(preview_button), instance_, nullptr);

    add_label(L"SCREEN CAPTURE", 20, 222, 300);
    add_checkbox(cursor, L"Include mouse cursor", working_.cursor_visible, 20, 250);
    CreateWindowExW(0, L"BUTTON", L"Restart capture", WS_CHILD | WS_VISIBLE, 555, 246, 130, 28, window_, control_id(restart_capture_button), instance_, nullptr);
    add_label(L"Preferred tracked display", 20, 284); const auto monitor_combo = add_combo(tracked_monitor, 240, 280, 445);
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

    add_label(L"REFERENCE DETECTION", 20, 320, 300);
    CreateWindowExW(0, L"BUTTON", L"Set current screen", WS_CHILD | WS_VISIBLE, 370, 314, 145, 27, window_, control_id(set_reference_settings), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Import image…", WS_CHILD | WS_VISIBLE, 525, 314, 160, 27, window_, control_id(import_reference_settings), instance_, nullptr);
    add_label(L"Similarity threshold (%)", 20, 352); add_edit(threshold, std::to_wstring(working_.detector.threshold * 100.0), 240, 348);
    add_label(L"Detection interval (ms)", 380, 352, 180); add_edit(detection_interval, std::to_wstring(working_.detection_interval.count()), 575, 348, 110);
    add_label(L"Matching confirmations", 20, 386); add_edit(matches, std::to_wstring(working_.detector.matches_required), 240, 382);
    add_label(L"Mismatch confirmations", 380, 386, 180); add_edit(mismatches, std::to_wstring(working_.detector.mismatches_required), 575, 382, 110);
    add_label(L"Full rescan interval (s)", 20, 420); add_edit(scan_interval, std::to_wstring(working_.full_scan_interval.count()), 240, 416);
    add_label(L"Reassignment confirmations", 380, 420, 190); add_edit(reassignments, std::to_wstring(working_.monitor_tracker.confirmations_required), 575, 416, 110);
    add_label(L"When reference is missing", 20, 454); const auto missing = add_combo(missing_behavior, 240, 450, 445);
    combo_add(missing, L"Use webcam/video (safe default)"); combo_add(missing, L"Keep current output"); combo_add(missing, L"Use last tracked screen"); combo_add(missing, L"Use safe placeholder");
    SendMessageW(missing, CB_SETCURSEL, static_cast<int>(working_.missing_behavior), 0);

    add_label(L"OUTPUT", 20, 494, 300);
    add_label(L"Resolution", 20, 526); const auto out_size = add_combo(output_size, 195, 522, 160); combo_add(out_size, L"1920 × 1080"); combo_add(out_size, L"1280 × 720");
    SendMessageW(out_size, CB_SETCURSEL, working_.output_size.width == 1280 ? 1 : 0, 0);
    add_label(L"Frame rate", 380, 526, 100); const auto out_fps = add_combo(output_fps, 500, 522, 90); combo_add(out_fps, L"30 fps"); SendMessageW(out_fps, CB_SETCURSEL, 0, 0);
    CreateWindowExW(0, L"BUTTON", L"Restart virtual camera", WS_CHILD | WS_VISIBLE, 596, 522, 90, 27, window_, control_id(restart_vcam_button), instance_, nullptr);
    add_label(L"Fade duration (0–2000 ms)", 20, 560); add_edit(fade, std::to_wstring(working_.fade_duration.count()), 240, 556);
    add_label(L"Webcam scaling", 380, 560, 130); const auto camera = add_combo(camera_scale, 515, 556, 170);
    combo_add(camera, L"Fit with letterboxing"); combo_add(camera, L"Fill and crop"); combo_add(camera, L"Stretch"); SendMessageW(camera, CB_SETCURSEL, static_cast<int>(working_.camera_scaling), 0);
    add_label(L"Screen scaling", 380, 594, 130); const auto screen = add_combo(screen_scale, 515, 590, 170);
    combo_add(screen, L"Fit with letterboxing"); combo_add(screen, L"Fill and crop"); combo_add(screen, L"Stretch"); SendMessageW(screen, CB_SETCURSEL, static_cast<int>(working_.screen_scaling), 0);
    std::wostringstream color_text; color_text << L'#' << std::hex << std::setfill(L'0') << std::setw(6) << (working_.placeholder_color_bgra & 0x00ffffffu);
    add_label(L"Placeholder color", 20, 594); add_edit(placeholder_color, color_text.str(), 195, 590, 120);

    add_label(L"GENERAL AND LOGGING", 20, 630, 300);
    add_checkbox(start_windows, L"Start with Windows", working_.start_with_windows, 20, 658, 190);
    add_checkbox(start_minimized, L"Start minimized to tray", working_.start_minimized, 220, 658, 210);
    add_checkbox(start_auto, L"Start detection automatically", working_.start_automatically, 440, 658, 245);
    add_checkbox(close_tray, L"Close button minimizes to tray", working_.close_to_tray, 20, 684, 240);
    add_checkbox(confirm_exit, L"Confirm before exiting", working_.confirm_exit, 270, 684, 190);
    add_checkbox(diagnostic, L"Diagnostic logging", working_.diagnostic_logging, 470, 684, 190);
    add_checkbox(notifications, L"Show Windows notifications", working_.show_notifications, 20, 710, 240);
    add_label(L"Interface language", 280, 710, 150); const auto languages = add_combo(language, 440, 706, 245);
    combo_add(languages, L"English (United States)"); SendMessageW(languages, CB_SETCURSEL, 0, 0);
    add_label(L"Log retention (days)", 20, 740, 180); add_edit(log_retention, std::to_wstring(working_.log_retention_days), 205, 736, 90);
    add_label(L"Log level", 380, 740, 100); const auto levels = add_combo(configured_log_level, 500, 736, 185);
    combo_add(levels, L"Trace"); combo_add(levels, L"Debug"); combo_add(levels, L"Info"); combo_add(levels, L"Warning"); combo_add(levels, L"Error");
    SendMessageW(levels, CB_SETCURSEL, static_cast<int>(working_.log_level), 0);

    CreateWindowExW(0, L"BUTTON", L"Open log folder", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 20, 774, 145, 30, window_, control_id(open_log_button), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Export logs…", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 175, 774, 125, 30, window_, control_id(export_log_button), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Clear logs", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 310, 774, 110, 30, window_, control_id(clear_log_button), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Reset diagnostic counters", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 20, 816, 210, 30, window_, control_id(reset_counters_button), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Save", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON, 505, 816, 85, 30, window_, control_id(save_button), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Cancel", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 600, 816, 85, 30, window_, control_id(cancel_button), instance_, nullptr);
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
        if (device < 0 || static_cast<std::size_t>(device) >= video_option_ids_.size())
            throw std::invalid_argument("The selected video source is no longer available in the settings list");
        working_.selected_video_device_id = video_option_ids_[static_cast<std::size_t>(device)];
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
