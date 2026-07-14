#include "settings_window.hpp"
#include "app.hpp"
#include "preview_window.hpp"
#include "status_presentation.hpp"
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
    open_log_button, export_log_button, clear_log_button, toggle_device_details_button, restart_all_button
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
    : owner_(owner), instance_(instance), app_(app), working_(app.config()), devices_(app.video_devices()), monitors_(app.monitors()),
      monitor_observations_(app.status().monitor_observations) {
    WNDCLASSEXW wc{sizeof(wc)}; wc.hInstance = instance_; wc.lpfnWndProc = procedure; wc.lpszClassName = L"AutomaticScreenCameraSettings";
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW); wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    wc.hIcon = reinterpret_cast<HICON>(SendMessageW(owner_, WM_GETICON, ICON_BIG, 0));
    wc.hIconSm = reinterpret_cast<HICON>(SendMessageW(owner_, WM_GETICON, ICON_SMALL, 0));
    RegisterClassExW(&wc);
    constexpr DWORD window_style = WS_OVERLAPPEDWINDOW;
    constexpr DWORD window_ex_style = WS_EX_DLGMODALFRAME;
    const UINT owner_dpi = GetDpiForWindow(owner_);
    const UINT dpi = owner_dpi == 0 ? 96U : owner_dpi;
    RECT initial_bounds{0, 0, MulDiv(800, static_cast<int>(dpi), 96),
                        MulDiv(680, static_cast<int>(dpi), 96)};
    if (!AdjustWindowRectExForDpi(&initial_bounds, window_style, FALSE, window_ex_style, dpi))
        AdjustWindowRectEx(&initial_bounds, window_style, FALSE, window_ex_style);
    window_ = CreateWindowExW(window_ex_style, wc.lpszClassName, L"Automatic Screen Camera — Settings",
                              window_style, CW_USEDEFAULT, CW_USEDEFAULT,
                              initial_bounds.right - initial_bounds.left, initial_bounds.bottom - initial_bounds.top,
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
        return 0;
    }
    case WM_SIZE:
        layout_controls();
        return 0;
    case WM_GETMINMAXINFO: {
        auto* bounds = reinterpret_cast<MINMAXINFO*>(lparam);
        bounds->ptMinTrackSize.x = dip(window_, 760);
        bounds->ptMinTrackSize.y = dip(window_, 600);
        return 0;
    }
    case WM_DPICHANGED: {
        const auto* suggested = reinterpret_cast<const RECT*>(lparam);
        if (suggested) SetWindowPos(window_, nullptr, suggested->left, suggested->top,
                                    suggested->right - suggested->left, suggested->bottom - suggested->top,
                                    SWP_NOACTIVATE | SWP_NOZORDER);
        if (fonts_) {
            fonts_->recreate(window_);
            set_children_font(window_, fonts_->body());
            for (const HWND heading : section_labels_) set_control_font(heading, fonts_->section());
        }
        layout_controls();
        return 0;
    }
    case WM_NOTIFY:
        if (reinterpret_cast<NMHDR*>(lparam)->hwndFrom == tabs_ && reinterpret_cast<NMHDR*>(lparam)->code == TCN_SELCHANGE)
            select_tab(TabCtrl_GetCurSel(tabs_));
        return 0;
    case WM_COMMAND:
        if (const HWND control = reinterpret_cast<HWND>(lparam)) {
            const UINT notification = HIWORD(wparam);
            if (notification == BN_SETFOCUS || notification == EN_SETFOCUS || notification == CBN_SETFOCUS)
                tooltips_.show_for_focus(control, true);
            else if (notification == BN_KILLFOCUS || notification == EN_KILLFOCUS || notification == CBN_KILLFOCUS)
                tooltips_.show_for_focus(control, false);
        }
        switch (LOWORD(wparam)) {
        case video_device:
            if (HIWORD(wparam) == CBN_SELCHANGE) update_device_details();
            return 0;
        case auto_reconnect:
            if (HIWORD(wparam) == BN_CLICKED) update_device_details();
            return 0;
        case save_button: save(); return 0;
        case cancel_button: SendMessageW(window_, WM_CLOSE, 0, 0); return 0;
        case restart_input_button: app_.restart_video_input(); return 0;
        case restart_capture_button: app_.restart_screen_capture(); return 0;
        case restart_vcam_button: app_.restart_virtual_camera(); return 0;
        case restart_all_button: app_.restart_all(); return 0;
        case reset_counters_button: app_.reset_diagnostic_counters(); return 0;
        case preview_button: show_previews(); return 0;
        case toggle_device_details_button: set_device_details_expanded(!device_details_expanded_); return 0;
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

void SettingsWindow::register_control(const HWND control, const int page, const int x, const int y,
                                      const int width, const int height, const bool stretch) {
    placements_[control] = {page, x, y, width, height, stretch};
    page_controls_[static_cast<std::size_t>(page)].push_back(control);
}

HWND SettingsWindow::add_label(const wchar_t* value, const int page, const int x, const int y,
                               const int width, const int height, const bool stretch) {
    const HWND control = CreateWindowExW(0, L"STATIC", value, WS_CHILD | WS_VISIBLE,
                                         0, 0, 0, 0, window_, nullptr, instance_, nullptr);
    register_control(control, page, x, y, width, height, stretch);
    return control;
}
HWND SettingsWindow::add_edit(const UINT id, const std::wstring& value, const int page, const int x, const int y,
                              const int width, const bool stretch) {
    const HWND control = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", value.c_str(),
                                         WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
                                         0, 0, 0, 0, window_, control_id(id), instance_, nullptr);
    register_control(control, page, x, y, width, 26, stretch);
    return control;
}
HWND SettingsWindow::add_checkbox(const UINT id, const wchar_t* value, const bool is_checked,
                                  const int page, const int x, const int y, const int width) {
    const auto control = CreateWindowExW(0, L"BUTTON", value, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
                                         0, 0, 0, 0, window_, control_id(id), instance_, nullptr);
    SendMessageW(control, BM_SETCHECK, is_checked ? BST_CHECKED : BST_UNCHECKED, 0);
    register_control(control, page, x, y, width, 26);
    return control;
}
HWND SettingsWindow::add_combo(const UINT id, const int page, const int x, const int y,
                               const int width, const bool stretch) {
    const HWND control = CreateWindowExW(0, WC_COMBOBOXW, nullptr,
                                         WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
                                         0, 0, 0, 0, window_, control_id(id), instance_, nullptr);
    register_control(control, page, x, y, width, 160, stretch);
    return control;
}
HWND SettingsWindow::add_button(const UINT id, const wchar_t* value, const int page, const int x, const int y,
                                const int width, const int height) {
    const HWND control = CreateWindowExW(0, L"BUTTON", value, WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                                         0, 0, 0, 0, window_, control_id(id), instance_, nullptr);
    register_control(control, page, x, y, width, height);
    return control;
}

void SettingsWindow::update_device_details() {
    const int selected = combo_selection(window_, video_device);
    std::wostringstream details;
    std::wstring status_text;
    if (selected <= 0 || static_cast<std::size_t>(selected) >= video_option_ids_.size()) {
        details << L"Status: No video input selected";
        status_text = L"No video input selected";
    } else {
        const auto& identifier = video_option_ids_[static_cast<std::size_t>(selected)];
        const auto device = std::find_if(devices_.begin(), devices_.end(), [&identifier](const VideoDevice& candidate) {
            return candidate.identifier == identifier;
        });
        if (device == devices_.end()) {
            details << L"Status: Unavailable (saved selection)\r\nIdentifier: " << wide(identifier)
                    << L"\r\nSupported formats: unavailable until the device reconnects";
            status_text = L"⚠ ";
            status_text += wide(unavailable_video_source_status(checked(auto_reconnect)));
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
            status_text = device->connected ? L"✓ Connected" : L"⚠ Device unavailable";
        }
    }
    SetWindowTextW(GetDlgItem(window_, video_details), details.str().c_str());
    if (device_status_) SetWindowTextW(device_status_, status_text.c_str());
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
    constexpr int general_page = 0;
    constexpr int sources_page = 1;
    constexpr int detection_page = 2;
    constexpr int output_page = 3;
    constexpr int advanced_page = 4;

    tabs_ = CreateWindowExW(0, WC_TABCONTROLW, nullptr,
                            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS,
                            0, 0, 0, 0, window_, nullptr, instance_, nullptr);
    for (const wchar_t* title : {L"General", L"Sources", L"Detection", L"Output", L"Advanced & diagnostics"}) {
        TCITEMW item{};
        item.mask = TCIF_TEXT;
        item.pszText = const_cast<wchar_t*>(title);
        TabCtrl_InsertItem(tabs_, TabCtrl_GetItemCount(tabs_), &item);
    }

    const auto heading = [this](const wchar_t* text, const int page, const int y) {
        const HWND control = add_label(text, page, 16, y, 300, 24);
        section_labels_.push_back(control);
        return control;
    };

    // General
    heading(L"STARTUP", general_page, 12);
    add_checkbox(start_windows, L"Start with Windows", working_.start_with_windows, general_page, 16, 46, 240);
    add_checkbox(start_minimized, L"Start minimized to tray", working_.start_minimized, general_page, 300, 46, 240);
    add_checkbox(start_auto, L"Start detection automatically", working_.start_automatically, general_page, 16, 80, 260);
    heading(L"WINDOW BEHAVIOR", general_page, 126);
    add_checkbox(close_tray, L"Close button minimizes to tray", working_.close_to_tray, general_page, 16, 160, 270);
    add_checkbox(confirm_exit, L"Confirm before exiting", working_.confirm_exit, general_page, 300, 160, 230);
    add_checkbox(notifications, L"Show Windows notifications", working_.show_notifications, general_page, 16, 194, 270);
    add_label(L"Interface language", general_page, 16, 244, 160);
    const auto languages = add_combo(language, general_page, 190, 238, 250);
    combo_add(languages, L"English (United States)");
    SendMessageW(languages, CB_SETCURSEL, 0, 0);

    // Sources
    heading(L"VIDEO SOURCE", sources_page, 12);
    add_label(L"Device", sources_page, 16, 48, 140);
    const auto device = add_combo(video_device, sources_page, 170, 42, 20, true);
    std::vector<std::string> available_device_ids;
    available_device_ids.reserve(devices_.size());
    for (const auto& available : devices_) available_device_ids.push_back(available.identifier);
    const auto choices = build_persistent_device_choices(available_device_ids, working_.selected_video_device_id);
    video_option_ids_ = choices.identifiers;
    combo_add(device, L"No video input selected");
    for (const auto& available : devices_) {
        std::wostringstream description;
        description << wide(available.name);
        if (!available.formats.empty()) {
            const auto& format = available.formats.front();
            description << L" — " << format.size.width << L"×" << format.size.height << L" @ "
                        << (format.denominator ? format.numerator / format.denominator : 0) << L" fps";
        }
        combo_add(device, description.str().c_str());
    }
    if (choices.configured_device_unavailable)
        combo_add(device, (std::wstring(L"Unavailable saved source — ") + wide(working_.selected_video_device_id)).c_str());
    SendMessageW(device, CB_SETCURSEL, static_cast<WPARAM>(choices.selected_index), 0);
    device_status_ = add_label(L"Checking connection…", sources_page, 170, 78, 360, 24);
    device_details_button_ = add_button(toggle_device_details_button, L"Show details", sources_page, 560, 72, 130, 28);
    const HWND details = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", nullptr,
                                         WS_CHILD | WS_TABSTOP | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL,
                                         0, 0, 0, 0, window_, control_id(video_details), instance_, nullptr);
    register_control(details, sources_page, 170, 104, 20, 72, true);
    ShowWindow(details, SW_HIDE);

    add_label(L"Preferred input", sources_page, 16, 122, 140);
    const auto in_size = add_combo(input_size, sources_page, 170, 116, 170);
    combo_add(in_size, L"1920 × 1080"); combo_add(in_size, L"1280 × 720");
    SendMessageW(in_size, CB_SETCURSEL, working_.preferred_input_size.width == 1280 ? 1 : 0, 0);
    add_label(L"Frame rate", sources_page, 370, 122, 100);
    const auto in_fps = add_combo(input_fps, sources_page, 470, 116, 100);
    combo_add(in_fps, L"30 fps"); SendMessageW(in_fps, CB_SETCURSEL, 0, 0);
    add_button(restart_input_button, L"Restart input", sources_page, 590, 116, 120);
    add_checkbox(auto_reconnect, L"Reconnect automatically", working_.video_auto_reconnect, sources_page, 16, 158, 230);
    add_label(L"Retry interval (seconds)", sources_page, 270, 160, 180);
    add_edit(reconnect_interval, std::to_wstring(working_.video_reconnect_interval.count()), sources_page, 455, 154, 80);

    heading(L"SCREEN CAPTURE", sources_page, 210);
    add_checkbox(cursor, L"Include mouse cursor", working_.cursor_visible, sources_page, 16, 244, 230);
    add_button(restart_capture_button, L"Restart capture", sources_page, 560, 238, 150);
    add_label(L"Preferred tracked display", sources_page, 16, 286, 180);
    const auto monitor_combo = add_combo(tracked_monitor, sources_page, 210, 280, 20, true);
    combo_add(monitor_combo, L"Keep automatic/reference-based selection");
    int monitor_selected = 0;
    const auto now = Clock::now();
    for (std::size_t i = 0; i < monitors_.size(); ++i) {
        std::wostringstream description;
        description << wide(monitors_[i].identity.model) << L" — " << monitors_[i].identity.resolution.width << L"×"
                    << monitors_[i].identity.resolution.height << L" at (" << monitors_[i].identity.desktop_x << L", "
                    << monitors_[i].identity.desktop_y << L")";
        const auto observation = std::find_if(monitor_observations_.begin(), monitor_observations_.end(), [&](const MonitorObservation& item) {
            return item.identity.stable_key() == monitors_[i].identity.stable_key();
        });
        if (observation != monitor_observations_.end()) {
            description << L" — last similarity " << std::fixed << std::setprecision(1) << observation->last_similarity * 100.0 << L'%';
            if (!observation->capture_valid) description << L" (latest scan unavailable)";
            if (observation->last_reference_detected_at != TimePoint{}) {
                const auto age = std::max(std::chrono::seconds{0}, std::chrono::duration_cast<std::chrono::seconds>(now - observation->last_reference_detected_at));
                description << L", reference seen " << age.count() << L" s ago";
            } else description << L", reference not yet seen";
            if (observation->previously_tracked) description << L", previously tracked";
        }
        combo_add(monitor_combo, description.str().c_str());
        if (working_.last_tracked_monitor && monitors_[i].identity.stable_key() == working_.last_tracked_monitor->stable_key())
            monitor_selected = static_cast<int>(i + 1);
    }
    SendMessageW(monitor_combo, CB_SETCURSEL, monitor_selected, 0);
    add_button(preview_button, L"Open previews", sources_page, 16, 330, 140);

    // Detection
    heading(L"REFERENCE", detection_page, 12);
    add_label(L"Capture the visual that keeps the webcam active in Automatic mode.", detection_page, 16, 44, 20, 24, true);
    add_button(set_reference_settings, L"Set current screen", detection_page, 16, 78, 170);
    add_button(import_reference_settings, L"Import image…", detection_page, 198, 78, 150);
    heading(L"DETECTION BEHAVIOR", detection_page, 132);
    add_label(L"Similarity threshold (%)", detection_page, 16, 170, 190);
    add_edit(threshold, std::to_wstring(working_.detector.threshold * 100.0), detection_page, 220, 164, 100);
    add_label(L"Detection interval (ms)", detection_page, 370, 170, 190);
    add_edit(detection_interval, std::to_wstring(working_.detection_interval.count()), detection_page, 570, 164, 110);
    add_label(L"Matching confirmations", detection_page, 16, 208, 190);
    add_edit(matches, std::to_wstring(working_.detector.matches_required), detection_page, 220, 202, 100);
    add_label(L"Mismatch confirmations", detection_page, 370, 208, 190);
    add_edit(mismatches, std::to_wstring(working_.detector.mismatches_required), detection_page, 570, 202, 110);
    add_label(L"Full rescan interval (seconds)", detection_page, 16, 246, 210);
    add_edit(scan_interval, std::to_wstring(working_.full_scan_interval.count()), detection_page, 240, 240, 100);
    add_label(L"Reassignment confirmations", detection_page, 370, 246, 200);
    add_edit(reassignments, std::to_wstring(working_.monitor_tracker.confirmations_required), detection_page, 570, 240, 110);
    add_label(L"When reference is missing", detection_page, 16, 292, 200);
    const auto missing = add_combo(missing_behavior, detection_page, 230, 286, 20, true);
    combo_add(missing, L"Use webcam/video (safe default)"); combo_add(missing, L"Keep current output");
    combo_add(missing, L"Use last tracked screen"); combo_add(missing, L"Use safe placeholder");
    SendMessageW(missing, CB_SETCURSEL, static_cast<int>(working_.missing_behavior), 0);

    // Output
    heading(L"VIRTUAL CAMERA OUTPUT", output_page, 12);
    add_label(L"Resolution", output_page, 16, 52, 140);
    const auto out_size = add_combo(output_size, output_page, 170, 46, 180);
    combo_add(out_size, L"1920 × 1080"); combo_add(out_size, L"1280 × 720");
    SendMessageW(out_size, CB_SETCURSEL, working_.output_size.width == 1280 ? 1 : 0, 0);
    add_label(L"Frame rate", output_page, 380, 52, 110);
    const auto out_fps = add_combo(output_fps, output_page, 500, 46, 100);
    combo_add(out_fps, L"30 fps"); SendMessageW(out_fps, CB_SETCURSEL, 0, 0);
    add_button(restart_vcam_button, L"Restart virtual camera", output_page, 16, 92, 190);
    heading(L"TRANSITION AND SCALING", output_page, 148);
    add_label(L"Fade duration (0–2000 ms)", output_page, 16, 188, 210);
    add_edit(fade, std::to_wstring(working_.fade_duration.count()), output_page, 240, 182, 110);
    add_label(L"Webcam scaling", output_page, 16, 230, 180);
    const auto camera = add_combo(camera_scale, output_page, 210, 224, 240);
    combo_add(camera, L"Fit with letterboxing"); combo_add(camera, L"Fill and crop"); combo_add(camera, L"Stretch");
    SendMessageW(camera, CB_SETCURSEL, static_cast<int>(working_.camera_scaling), 0);
    add_label(L"Screen scaling", output_page, 16, 272, 180);
    const auto screen = add_combo(screen_scale, output_page, 210, 266, 240);
    combo_add(screen, L"Fit with letterboxing"); combo_add(screen, L"Fill and crop"); combo_add(screen, L"Stretch");
    SendMessageW(screen, CB_SETCURSEL, static_cast<int>(working_.screen_scaling), 0);

    // Advanced & diagnostics
    heading(L"SAFE FALLBACK", advanced_page, 12);
    std::wostringstream color_text;
    color_text << L'#' << std::hex << std::setfill(L'0') << std::setw(6) << (working_.placeholder_color_bgra & 0x00ffffffu);
    add_label(L"Placeholder color", advanced_page, 16, 50, 160);
    add_edit(placeholder_color, color_text.str(), advanced_page, 190, 44, 130);
    heading(L"RECOVERY", advanced_page, 96);
    add_button(restart_all_button, L"Restart all video components", advanced_page, 16, 130, 230);
    heading(L"LOGGING", advanced_page, 184);
    add_checkbox(diagnostic, L"Diagnostic logging", working_.diagnostic_logging, advanced_page, 16, 218, 220);
    add_label(L"Log retention (days)", advanced_page, 280, 220, 170);
    add_edit(log_retention, std::to_wstring(working_.log_retention_days), advanced_page, 460, 214, 90);
    add_label(L"Log level", advanced_page, 16, 260, 150);
    const auto levels = add_combo(configured_log_level, advanced_page, 180, 254, 190);
    combo_add(levels, L"Trace"); combo_add(levels, L"Debug"); combo_add(levels, L"Info"); combo_add(levels, L"Warning"); combo_add(levels, L"Error");
    SendMessageW(levels, CB_SETCURSEL, static_cast<int>(working_.log_level), 0);
    add_button(open_log_button, L"Open log folder", advanced_page, 16, 308, 150);
    add_button(export_log_button, L"Export logs…", advanced_page, 178, 308, 130);
    add_button(clear_log_button, L"Clear logs", advanced_page, 320, 308, 120);
    add_button(reset_counters_button, L"Reset diagnostic counters", advanced_page, 16, 354, 220);

    save_button_ = CreateWindowExW(0, L"BUTTON", L"Save changes", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                                   0, 0, 0, 0, window_, control_id(save_button), instance_, nullptr);
    cancel_button_ = CreateWindowExW(0, L"BUTTON", L"Cancel", WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                                     0, 0, 0, 0, window_, control_id(cancel_button), instance_, nullptr);

    fonts_ = std::make_unique<UiFonts>(window_);
    set_children_font(window_, fonts_->body());
    for (const HWND section : section_labels_) set_control_font(section, fonts_->section());
    tooltips_.create(window_, instance_);
    tooltips_.set(GetDlgItem(window_, reconnect_interval), L"How often the app retries a disconnected saved video source. Accepted range: 1–60 seconds.", true);
    tooltips_.set(GetDlgItem(window_, tracked_monitor), L"Choose a physical display or keep reference-based automatic selection.", true);
    tooltips_.set(GetDlgItem(window_, threshold), L"Minimum image similarity counted as a reference match. Accepted range: 0–100%.", true);
    tooltips_.set(GetDlgItem(window_, detection_interval), L"Time between fast reference checks. Accepted range: 100–1000 ms.", true);
    tooltips_.set(GetDlgItem(window_, matches), L"Consecutive matches required before returning to webcam/video. Accepted range: 1–30.", true);
    tooltips_.set(GetDlgItem(window_, mismatches), L"Consecutive mismatches required before switching to screen. Accepted range: 1–30.", true);
    tooltips_.set(GetDlgItem(window_, scan_interval), L"Time between full scans of every connected display. Accepted range: 5–3600 seconds.", true);
    tooltips_.set(GetDlgItem(window_, reassignments), L"Repeated unambiguous scans required before moving tracking to another display. Accepted range: 1–10.", true);
    tooltips_.set(GetDlgItem(window_, missing_behavior), L"Privacy-safe behavior used when the reference or tracked capture is unavailable.", true);
    tooltips_.set(GetDlgItem(window_, fade), L"Crossfade duration for output changes. Accepted range: 0–2000 ms.", true);
    tooltips_.set(GetDlgItem(window_, camera_scale), L"Controls how webcam frames fit the virtual-camera resolution.", true);
    tooltips_.set(GetDlgItem(window_, screen_scale), L"Controls how screen frames fit the virtual-camera resolution.", true);
    tooltips_.set(GetDlgItem(window_, placeholder_color), L"Safe fallback color in #RRGGBB format.", true);
    tooltips_.set(GetDlgItem(window_, log_retention), L"Number of days to keep rotating logs. Accepted range: 1–365.", true);
    tooltips_.set(GetDlgItem(window_, configured_log_level), L"Minimum severity written when diagnostic logging is off.", true);

    update_device_details();
    select_tab(general_page);
    layout_controls();
}

void SettingsWindow::layout_controls() {
    if (!tabs_) return;
    RECT client{};
    GetClientRect(window_, &client);
    const int pad = dip(window_, 16);
    const int footer_height = dip(window_, 58);
    const int tab_width = std::max(1, static_cast<int>(client.right) - pad * 2);
    const int tab_height = std::max(dip(window_, 420),
                                    static_cast<int>(client.bottom) - pad - footer_height);
    MoveWindow(tabs_, pad, pad, tab_width, tab_height, TRUE);

    const int page_x = pad + dip(window_, 12);
    const int page_y = pad + dip(window_, 38);
    const int page_width = std::max(1, tab_width - dip(window_, 24));
    const HWND details = GetDlgItem(window_, video_details);
    for (const auto& [control, placement] : placements_) {
        int y = placement.y;
        if (placement.page == 1 && control != details && placement.y >= 116 && device_details_expanded_) y += 78;
        const int control_width = placement.stretch ? std::max(dip(window_, 60), page_width - dip(window_, placement.x + placement.width))
                                                    : dip(window_, placement.width);
        MoveWindow(control, page_x + dip(window_, placement.x), page_y + dip(window_, y),
                   control_width, dip(window_, placement.height), TRUE);
    }

    const int footer_y = client.bottom - pad - dip(window_, 34);
    MoveWindow(cancel_button_, client.right - pad - dip(window_, 100), footer_y, dip(window_, 100), dip(window_, 32), TRUE);
    MoveWindow(save_button_, client.right - pad - dip(window_, 100) - dip(window_, 132), footer_y,
               dip(window_, 120), dip(window_, 32), TRUE);
}

void SettingsWindow::select_tab(const int index) {
    selected_tab_ = std::clamp(index, 0, static_cast<int>(page_controls_.size()) - 1);
    TabCtrl_SetCurSel(tabs_, selected_tab_);
    for (std::size_t page = 0; page < page_controls_.size(); ++page) {
        const bool visible = static_cast<int>(page) == selected_tab_;
        for (const HWND control : page_controls_[page]) {
            bool show = visible;
            if (control == GetDlgItem(window_, video_details) && !device_details_expanded_) show = false;
            ShowWindow(control, show ? SW_SHOW : SW_HIDE);
        }
    }
    layout_controls();
}

void SettingsWindow::set_device_details_expanded(const bool expanded) {
    device_details_expanded_ = expanded;
    SetWindowTextW(device_details_button_, expanded ? L"Hide details" : L"Show details");
    ShowWindow(GetDlgItem(window_, video_details), expanded && selected_tab_ == 1 ? SW_SHOW : SW_HIDE);
    layout_controls();
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
