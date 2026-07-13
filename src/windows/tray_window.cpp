#include "tray_window.hpp"
#include "app.hpp"
#include "settings_window.hpp"
#include "preview_window.hpp"

#include <commctrl.h>
#include <commdlg.h>
#include <shellapi.h>
#include <wtsapi32.h>
#include <array>
#include <chrono>
#include <iomanip>
#include <sstream>
#include <cstring>
#include <utility>

namespace asc::win {
namespace {
constexpr UINT tray_message = WM_APP + 1;
constexpr UINT timer_id = 1;
constexpr auto lifecycle_recovery_delay = std::chrono::seconds{2};
HMENU control_id(const UINT id) noexcept { return reinterpret_cast<HMENU>(static_cast<INT_PTR>(id)); }
enum Command : UINT {
    open = 100, start, stop, automatic, force_camera, force_screen, set_reference, import_reference,
    rescan, restart_video, restart_capture, restart_camera, restart_all, startup, show_previews, open_log, export_log, copy_recent, clear_log, settings, exit_app,
    return_automatic
};
const wchar_t* detection_name(const DetectionState state) {
    switch (state) { case DetectionState::unknown: return L"Unknown"; case DetectionState::matching: return L"Detected";
    case DetectionState::not_matching: return L"Absent"; case DetectionState::reference_missing: return L"Missing"; case DetectionState::ambiguous: return L"Ambiguous"; }
    return L"Unknown";
}
std::wstring selected_video_name(const SelectedVideoSourceInfo& source) {
    if (source.identifier.empty()) return L"None selected";
    if (!source.display_name.empty()) return wide(source.display_name);
    return L"Unavailable saved source";
}
std::wstring monitor_name(const std::optional<MonitorIdentity>& monitor) {
    if (!monitor) return L"Not identified";
    if (!monitor->model.empty()) return wide(monitor->model);
    if (!monitor->device_path.empty()) return wide(monitor->device_path);
    return L"Unidentified display";
}
std::wstring output_name(const Source source, const SelectedVideoSourceInfo& video,
                         const std::optional<MonitorIdentity>& monitor) {
    switch (source) {
    case Source::camera: return selected_video_name(video);
    case Source::screen: return monitor_name(monitor);
    case Source::placeholder: return L"Safe placeholder";
    }
    return L"Unknown";
}
std::wstring compact_label(std::wstring value, const std::size_t limit = 20) {
    if (value.size() <= limit) return value;
    value.resize(limit - 1);
    value += L'\u2026';
    return value;
}
const wchar_t* mode_name(const OutputMode mode) {
    switch (mode) { case OutputMode::automatic: return L"Automatic"; case OutputMode::force_camera: return L"Force webcam/video"; case OutputMode::force_screen: return L"Force screen capture"; }
    return L"Unknown";
}
const wchar_t* device_name(const DeviceState state) {
    switch (state) { case DeviceState::unavailable: return L"Unavailable"; case DeviceState::initializing: return L"Initializing";
    case DeviceState::ready: return L"Ready"; case DeviceState::recovering: return L"Recovering"; case DeviceState::failed: return L"Failed"; }
    return L"Unknown";
}
std::wstring event_line(const LogEvent& event) {
    return wide(format_event_summary(event));
}
}

TrayWindow::TrayWindow(const HINSTANCE instance, App& app) : instance_(instance), app_(app) {
    INITCOMMONCONTROLSEX controls{sizeof(controls), ICC_STANDARD_CLASSES};
    InitCommonControlsEx(&controls);
    WNDCLASSEXW window_class{sizeof(window_class)};
    window_class.hInstance = instance_;
    window_class.lpfnWndProc = window_proc;
    window_class.lpszClassName = L"AutomaticScreenCameraWindow";
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    window_class.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    window_class.hIcon = LoadIconW(nullptr, IDI_APPLICATION);
    RegisterClassExW(&window_class);
    window_ = CreateWindowExW(0, window_class.lpszClassName, L"Automatic Screen Camera", WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                              CW_USEDEFAULT, CW_USEDEFAULT, 650, 620, nullptr, nullptr, instance_, this);
    if (!window_) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "Create main window");
    tray_.cbSize = sizeof(tray_); tray_.hWnd = window_; tray_.uID = 1;
    tray_.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP; tray_.uCallbackMessage = tray_message;
    update_tray_icon();
    Shell_NotifyIconW(NIM_ADD, &tray_);
    SetTimer(window_, timer_id, 250, nullptr);
}

TrayWindow::~TrayWindow() {
    Shell_NotifyIconW(NIM_DELETE, &tray_);
    if (status_icon_) DestroyIcon(status_icon_);
}

LRESULT CALLBACK TrayWindow::window_proc(const HWND window, const UINT message, const WPARAM wparam, const LPARAM lparam) {
    TrayWindow* self = reinterpret_cast<TrayWindow*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
        self = static_cast<TrayWindow*>(reinterpret_cast<CREATESTRUCTW*>(lparam)->lpCreateParams);
        self->window_ = window;
        SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    }
    return self ? self->handle_message(message, wparam, lparam) : DefWindowProcW(window, message, wparam, lparam);
}

LRESULT TrayWindow::handle_message(const UINT message, const WPARAM wparam, const LPARAM lparam) {
    switch (message) {
    case WM_CREATE: create_controls(); return 0;
    case WM_TIMER:
        if (wparam == timer_id && lifecycle_recovery_.consume_if_due(Clock::now())) app_.restart_all();
        refresh();
        return 0;
    case WM_COMMAND: dispatch_command(LOWORD(wparam)); return 0;
    case WM_DISPLAYCHANGE: app_.log_system_event("DISPLAY_LAYOUT_CHANGED", "Display layout or resolution changed"); app_.restart_screen_capture(); app_.request_rescan(); return 0;
    case WM_DEVICECHANGE: app_.log_system_event("DISPLAY_DEVICE_CHANGED", "A display or video device changed"); app_.restart_screen_capture(); app_.request_rescan(); return 0;
    case WM_DPICHANGED: {
        const auto* suggested = reinterpret_cast<const RECT*>(lparam);
        if (suggested) SetWindowPos(window_, nullptr, suggested->left, suggested->top,
                                    suggested->right - suggested->left, suggested->bottom - suggested->top,
                                    SWP_NOACTIVATE | SWP_NOZORDER);
        app_.log_system_event("DISPLAY_SCALING_CHANGED", "Display scaling changed");
        app_.restart_screen_capture();
        app_.request_rescan();
        return 0;
    }
    case WM_POWERBROADCAST:
        if (wparam == PBT_APMRESUMEAUTOMATIC) schedule_lifecycle_recovery("PC_RESUMED", "PC resumed from sleep");
        return TRUE;
    case WM_WTSSESSION_CHANGE:
        if (wparam == WTS_SESSION_UNLOCK) schedule_lifecycle_recovery("SESSION_UNLOCKED", "Windows session unlocked");
        else if (wparam == WTS_REMOTE_DISCONNECT) schedule_lifecycle_recovery("REMOTE_SESSION_ENDED", "Remote desktop session disconnected");
        return 0;
    case tray_message:
        if (LOWORD(lparam) == WM_LBUTTONUP || LOWORD(lparam) == WM_LBUTTONDBLCLK) show();
        else if (LOWORD(lparam) == WM_RBUTTONUP || LOWORD(lparam) == WM_CONTEXTMENU) { POINT p{}; GetCursorPos(&p); show_tray_menu(p); }
        return 0;
    case WM_CLOSE:
        if (!exiting_ && app_.config().close_to_tray) { hide(); return 0; }
        if (!exiting_ && app_.config().confirm_exit &&
            MessageBoxW(window_, L"Exit Automatic Screen Camera? The virtual camera will stop.", L"Confirm exit", MB_YESNO | MB_ICONQUESTION) != IDYES) return 0;
        DestroyWindow(window_); return 0;
    case WM_DESTROY: WTSUnRegisterSessionNotification(window_); PostQuitMessage(0); return 0;
    default: return DefWindowProcW(window_, message, wparam, lparam);
    }
}

void TrayWindow::schedule_lifecycle_recovery(std::string code, std::string message) {
    app_.log_system_event(std::move(code), std::move(message));
    lifecycle_recovery_.schedule(Clock::now(), lifecycle_recovery_delay);
    app_.log_system_event("RECOVERY_SCHEDULED", "Full video recovery scheduled after the device-return delay");
}

void TrayWindow::create_controls() {
    const auto label = [this](const wchar_t* text, int x, int y, int width, int height, DWORD style = SS_LEFT) {
        return CreateWindowExW(0, L"STATIC", text, WS_CHILD | WS_VISIBLE | style, x, y, width, height, window_, nullptr, instance_, nullptr);
    };
    label(L"Automatic Screen Camera", 18, 14, 420, 28, SS_LEFT);
    override_banner_ = label(L"MANUAL OVERRIDE ACTIVE — Automatic source switching is disabled.", 18, 48, 590, 26, SS_CENTER);
    CreateWindowExW(0, L"BUTTON", L"Return to Automatic", WS_CHILD | WS_VISIBLE, 440, 76, 168, 28, window_, control_id(return_automatic), instance_, nullptr);
    status_text_ = label(L"Starting…", 20, 112, 590, 230, SS_LEFT);
    start_stop_button_ = CreateWindowExW(0, L"BUTTON", L"Start", WS_CHILD | WS_VISIBLE, 20, 348, 90, 32, window_, control_id(start), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Set current screen as reference", WS_CHILD | WS_VISIBLE, 118, 348, 245, 32, window_, control_id(set_reference), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Rescan", WS_CHILD | WS_VISIBLE, 371, 348, 90, 32, window_, control_id(rescan), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Previews", WS_CHILD | WS_VISIBLE, 469, 348, 72, 32, window_, control_id(show_previews), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Settings", WS_CHILD | WS_VISIBLE, 547, 348, 62, 32, window_, control_id(settings), instance_, nullptr);
    label(L"Output mode", 20, 394, 120, 22);
    CreateWindowExW(0, L"BUTTON", L"Automatic", WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON, 20, 420, 140, 26, window_, control_id(automatic), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Force webcam/video", WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON, 170, 420, 180, 26, window_, control_id(force_camera), instance_, nullptr);
    CreateWindowExW(0, L"BUTTON", L"Force screen", WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON, 360, 420, 150, 26, window_, control_id(force_screen), instance_, nullptr);
    label(L"Recent activity", 20, 458, 180, 22);
    recent_list_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"LISTBOX", nullptr, WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_HSCROLL | LBS_NOINTEGRALHEIGHT,
                                   20, 484, 589, 82, window_, nullptr, instance_, nullptr);
    WTSRegisterSessionNotification(window_, NOTIFY_FOR_THIS_SESSION);
    refresh();
}

void TrayWindow::refresh() {
    const auto state = app_.status();
    const auto config = app_.config();
    const auto selected_video = app_.selected_video_source();
    const bool override = state.mode != OutputMode::automatic;
    ShowWindow(override_banner_, override ? SW_SHOW : SW_HIDE);
    ShowWindow(GetDlgItem(window_, return_automatic), override ? SW_SHOW : SW_HIDE);
    std::wostringstream text;
    text << L"Status: " << (app_.automation_running() ? L"Running" : L"Stopped") << L"\r\n"
         << L"Mode: " << mode_name(state.mode) << L"\r\n"
         << L"Reference: " << detection_name(state.detection.state) << L"    Similarity: " << std::fixed << std::setprecision(1) << state.detection.similarity * 100.0 << L"%"
         << L"    Threshold: " << config.detector.threshold * 100.0 << L"%\r\n"
         << L"Confirmations: " << state.detection.consecutive_matches << L" matching / " << state.detection.consecutive_mismatches << L" mismatching\r\n"
         << L"Tracked display: " << monitor_name(state.tracked_monitor);
    if (state.tracked_monitor) text << L" — " << state.tracked_monitor->resolution.width << L"×" << state.tracked_monitor->resolution.height;
    text << L"\r\n"
         << L"Selected video source: " << selected_video_name(selected_video) << L"\r\n"
         << L"Automatic target: " << output_name(state.automatic_target, selected_video, state.tracked_monitor)
         << L"    Actual output: " << output_name(state.actual_output, selected_video, state.tracked_monitor) << L"\r\n"
         << L"Transition: " << (state.transition.active ? L"In progress" : L"Idle") << L"    " << static_cast<int>(state.transition.screen_mix * 100) << L"% screen    " << state.transition.remaining.count() << L" ms remaining\r\n"
         << L"Video input: " << device_name(state.video_input) << L"    Screen capture: " << device_name(state.screen_capture) << L"    Virtual camera: " << device_name(state.virtual_camera) << L"\r\n";
    const auto now = Clock::now();
    text << L"Last detection: " << (state.detection.measured_at == TimePoint{} ? L"Never" : std::to_wstring(std::chrono::duration_cast<std::chrono::milliseconds>(now - state.detection.measured_at).count()) + L" ms ago")
         << L"    Last full scan: " << (state.last_full_scan == TimePoint{} ? L"Never" : std::to_wstring(std::chrono::duration_cast<std::chrono::seconds>(now - state.last_full_scan).count()) + L" s ago");
    if (!state.warning.empty()) text << L"\r\nWarning: " << wide(state.warning);
    SetWindowTextW(status_text_, text.str().c_str());
    SetWindowTextW(start_stop_button_, app_.automation_running() ? L"Stop" : L"Start");
    SetWindowLongPtrW(start_stop_button_, GWLP_ID, app_.automation_running() ? stop : start);
    CheckRadioButton(window_, automatic, force_screen, state.mode == OutputMode::automatic ? automatic : state.mode == OutputMode::force_camera ? force_camera : force_screen);
    SendMessageW(recent_list_, LB_RESETCONTENT, 0, 0);
    for (const auto& event : app_.recent_events()) { const auto line = event_line(event); SendMessageW(recent_list_, LB_ADDSTRING, 0, reinterpret_cast<LPARAM>(line.c_str())); }
    if (config.show_notifications && !state.warning.empty() && state.warning != last_notified_warning_) {
        last_notified_warning_ = state.warning;
        tray_.uFlags = NIF_INFO;
        wcsncpy_s(tray_.szInfoTitle, L"Automatic Screen Camera warning", _TRUNCATE);
        wcsncpy_s(tray_.szInfo, wide(state.warning).c_str(), _TRUNCATE);
        tray_.dwInfoFlags = NIIF_WARNING;
        Shell_NotifyIconW(NIM_MODIFY, &tray_);
        tray_.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    }
    if (state.warning.empty()) last_notified_warning_.clear();
    update_tray_icon();
}

HICON TrayWindow::make_status_icon(const COLORREF color) const {
    constexpr int size = 16;
    BITMAPV5HEADER header{}; header.bV5Size = sizeof(header); header.bV5Width = size; header.bV5Height = -size;
    header.bV5Planes = 1; header.bV5BitCount = 32; header.bV5Compression = BI_BITFIELDS;
    header.bV5RedMask = 0x00ff0000; header.bV5GreenMask = 0x0000ff00; header.bV5BlueMask = 0x000000ff; header.bV5AlphaMask = 0xff000000;
    void* bits = nullptr; HDC dc = GetDC(nullptr);
    HBITMAP bitmap = CreateDIBSection(dc, reinterpret_cast<BITMAPINFO*>(&header), DIB_RGB_COLORS, &bits, nullptr, 0); ReleaseDC(nullptr, dc);
    auto* pixels = static_cast<std::uint32_t*>(bits);
    const auto argb = 0xff000000u | (GetRValue(color) << 16) | (GetGValue(color) << 8) | GetBValue(color);
    for (int y = 0; y < size; ++y) for (int x = 0; x < size; ++x) pixels[y * size + x] = ((x - 7) * (x - 7) + (y - 7) * (y - 7) <= 42) ? argb : 0;
    HBITMAP mask = CreateBitmap(size, size, 1, 1, nullptr);
    ICONINFO info{TRUE, 0, 0, mask, bitmap};
    HICON icon = CreateIconIndirect(&info); DeleteObject(mask); DeleteObject(bitmap); return icon;
}

void TrayWindow::update_tray_icon() {
    const auto state = app_.status();
    const auto selected_video = app_.selected_video_source();
    COLORREF color = RGB(128, 128, 128);
    if (state.run_state == RunState::stopped || state.run_state == RunState::stopping) color = RGB(128, 128, 128);
    else if (state.mode != OutputMode::automatic) color = RGB(155, 80, 200);
    else if (state.run_state == RunState::error || state.detection.state == DetectionState::reference_missing) color = RGB(210, 55, 55);
    else if (state.transition.active || state.run_state == RunState::recovering) color = RGB(220, 180, 30);
    else if (state.actual_output == Source::screen) color = RGB(45, 115, 220);
    else if (state.actual_output == Source::camera) color = RGB(45, 175, 85);
    if (status_icon_) DestroyIcon(status_icon_);
    status_icon_ = make_status_icon(color); tray_.hIcon = status_icon_;
    std::wostringstream tip;
    tip << L"Automatic Screen Camera\n"
        << (app_.automation_running() ? mode_name(state.mode) : L"Stopped") << L" | " << detection_name(state.detection.state)
        << L"\nOut: " << compact_label(output_name(state.actual_output, selected_video, state.tracked_monitor))
        << L"\nScreen: " << compact_label(monitor_name(state.tracked_monitor));
    wcsncpy_s(tray_.szTip, tip.str().c_str(), _TRUNCATE);
    if (tray_.hWnd) Shell_NotifyIconW(NIM_MODIFY, &tray_);
}

void TrayWindow::show_tray_menu(const POINT point) {
    HMENU menu = CreatePopupMenu();
    AppendMenuW(menu, MF_STRING, open, L"Open"); AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, app_.automation_running() ? MF_GRAYED : MF_STRING, start, L"Start");
    AppendMenuW(menu, app_.automation_running() ? MF_STRING : MF_GRAYED, stop, L"Stop");
    HMENU modes = CreatePopupMenu(); AppendMenuW(modes, MF_STRING, automatic, L"Automatic"); AppendMenuW(modes, MF_STRING, force_camera, L"Force webcam/video"); AppendMenuW(modes, MF_STRING, force_screen, L"Force screen capture");
    AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(modes), L"Output mode");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr); AppendMenuW(menu, MF_STRING, set_reference, L"Set current screen as reference");
    AppendMenuW(menu, MF_STRING, import_reference, L"Import reference image…"); AppendMenuW(menu, MF_STRING, rescan, L"Rescan displays");
    HMENU recovery = CreatePopupMenu(); AppendMenuW(recovery, MF_STRING, restart_video, L"Restart video input"); AppendMenuW(recovery, MF_STRING, restart_capture, L"Restart screen capture");
    AppendMenuW(recovery, MF_STRING, restart_camera, L"Restart virtual camera"); AppendMenuW(recovery, MF_STRING, restart_all, L"Restart all video components");
    AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(recovery), L"Recovery"); AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, MF_STRING | (app_.config().start_with_windows ? MF_CHECKED : MF_UNCHECKED), startup, L"Start with Windows");
    AppendMenuW(menu, MF_STRING, show_previews, L"Open previews");
    AppendMenuW(menu, MF_STRING, open_log, L"Open full log"); AppendMenuW(menu, MF_STRING, export_log, L"Export logs…");
    AppendMenuW(menu, MF_STRING, copy_recent, L"Copy recent logs"); AppendMenuW(menu, MF_STRING, clear_log, L"Clear logs");
    AppendMenuW(menu, MF_STRING, settings, L"Settings"); AppendMenuW(menu, MF_STRING, exit_app, L"Exit");
    SetForegroundWindow(window_); TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, 0, window_, nullptr); DestroyMenu(menu);
}

void TrayWindow::dispatch_command(const UINT command) {
    switch (command) {
    case open: show(); break; case start: app_.start_automation(); break; case stop: app_.stop_automation(); break;
    case automatic: case return_automatic: app_.set_mode(OutputMode::automatic); break;
    case force_camera: app_.set_mode(OutputMode::force_camera); break; case force_screen: app_.set_mode(OutputMode::force_screen); break;
    case set_reference: {
        const auto monitors = app_.monitors();
        if (monitors.size() > 1) {
            HMENU choices = CreatePopupMenu();
            for (std::size_t i = 0; i < monitors.size(); ++i) {
                std::wostringstream label;
                label << wide(monitors[i].identity.model) << L" — " << monitors[i].identity.resolution.width << L" × "
                      << monitors[i].identity.resolution.height << L" at (" << monitors[i].identity.desktop_x << L", " << monitors[i].identity.desktop_y << L")";
                AppendMenuW(choices, MF_STRING, static_cast<UINT_PTR>(2000 + i), label.str().c_str());
            }
            POINT point{}; GetCursorPos(&point);
            const auto selected = TrackPopupMenu(choices, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, 0, window_, nullptr);
            DestroyMenu(choices);
            if (selected >= 2000 && static_cast<std::size_t>(selected - 2000) < monitors.size()) {
                hide(); app_.set_reference_monitor(monitors[static_cast<std::size_t>(selected - 2000)].identity);
            }
        } else if (monitors.size() == 1) {
            // This is an explicit user choice, so it may replace a persisted
            // monitor that is currently disconnected.
            hide(); app_.set_reference_monitor(monitors.front().identity);
        } else {
            hide(); app_.set_current_screen_reference();
        }
        break;
    }
    case import_reference: {
        wchar_t file[MAX_PATH]{}; OPENFILENAMEW dialog{sizeof(dialog)}; dialog.hwndOwner = window_; dialog.lpstrFile = file; dialog.nMaxFile = ARRAYSIZE(file);
        dialog.lpstrFilter = L"Images (*.png;*.jpg;*.jpeg;*.bmp)\0*.png;*.jpg;*.jpeg;*.bmp\0All files\0*.*\0"; dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
        if (GetOpenFileNameW(&dialog)) app_.import_reference(file); break;
    }
    case rescan: app_.request_rescan(); break; case restart_video: app_.restart_video_input(); break;
    case restart_capture: app_.restart_screen_capture(); break; case restart_camera: app_.restart_virtual_camera(); break; case restart_all: app_.restart_all(); break;
    case startup: { auto updated = app_.config(); updated.start_with_windows = !updated.start_with_windows; app_.apply_settings(std::move(updated)); break; }
    case show_previews:
        if (!previews_) previews_ = std::make_unique<PreviewWindow>(instance_, app_);
        previews_->show();
        break;
    case open_log: ShellExecuteW(window_, L"open", (app_.data_directory() / L"logs").c_str(), nullptr, nullptr, SW_SHOWNORMAL); break;
    case export_log: {
        wchar_t file[MAX_PATH] = L"AutomaticScreenCamera-logs.jsonl"; OPENFILENAMEW dialog{sizeof(dialog)}; dialog.hwndOwner = window_;
        dialog.lpstrFile = file; dialog.nMaxFile = ARRAYSIZE(file); dialog.lpstrFilter = L"JSON Lines (*.jsonl)\0*.jsonl\0All files\0*.*\0";
        dialog.lpstrDefExt = L"jsonl"; dialog.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
        if (GetSaveFileNameW(&dialog)) { try { app_.export_logs(file); } catch (const std::exception& e) { MessageBoxA(window_, e.what(), "Log export failed", MB_OK | MB_ICONERROR); } }
        break;
    }
    case copy_recent: {
        std::wstring content; for (const auto& event : app_.recent_events()) content += event_line(event) + L"\r\n";
        if (OpenClipboard(window_)) { EmptyClipboard(); const auto bytes = (content.size() + 1) * sizeof(wchar_t); HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if (memory) { auto* target = GlobalLock(memory); std::memcpy(target, content.c_str(), bytes); GlobalUnlock(memory); SetClipboardData(CF_UNICODETEXT, memory); }
            CloseClipboard(); }
        break;
    }
    case clear_log: if (MessageBoxW(window_, L"Clear all diagnostic logs?", L"Clear logs", MB_YESNO | MB_ICONWARNING) == IDYES) app_.clear_logs(); break;
    case settings: SettingsWindow::show(window_, instance_, app_); break;
    case exit_app:
        if (!app_.config().confirm_exit || MessageBoxW(window_, L"Exit Automatic Screen Camera? The virtual camera will stop.", L"Confirm exit", MB_YESNO | MB_ICONQUESTION) == IDYES) {
            exiting_ = true; app_.exit();
        }
        break;
    }
    refresh();
}

int TrayWindow::message_loop() { MSG message{}; while (GetMessageW(&message, nullptr, 0, 0) > 0) { TranslateMessage(&message); DispatchMessageW(&message); } return static_cast<int>(message.wParam); }
void TrayWindow::show() { ShowWindow(window_, SW_SHOW); SetForegroundWindow(window_); }
void TrayWindow::hide() { ShowWindow(window_, SW_HIDE); }
void TrayWindow::close() { exiting_ = true; PostMessageW(window_, WM_CLOSE, 0, 0); }

} // namespace asc::win
