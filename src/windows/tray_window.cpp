#include "tray_window.hpp"
#include "app.hpp"
#include "settings_window.hpp"
#include "preview_window.hpp"

#include <commctrl.h>
#include <commdlg.h>
#include <shellapi.h>
#include <array>
#include <chrono>
#include <iomanip>
#include <sstream>
#include <cstring>
#include <algorithm>
#include <utility>

namespace asc::win {
namespace {
constexpr UINT tray_message = WM_APP + 1;
constexpr UINT timer_id = 1;
constexpr UINT preview_timer_id = 2;
HMENU control_id(const UINT id) noexcept { return reinterpret_cast<HMENU>(static_cast<INT_PTR>(id)); }
enum Command : UINT {
    open = 100, start, stop, automatic, force_camera, force_screen, set_reference, import_reference,
    rescan, restart_video, restart_capture, restart_camera, restart_all, startup, show_previews, open_log, export_log, copy_recent, clear_log, settings, exit_app,
    return_automatic, toggle_details, view_all, output_info, reference_info, display_info, health_info
};
const wchar_t* detection_name(const DetectionState state) {
    switch (state) { case DetectionState::unknown: return L"Unknown"; case DetectionState::matching: return L"Detected";
    case DetectionState::not_matching: return L"Absent"; case DetectionState::reference_missing: return L"Missing"; }
    return L"Unknown";
}
std::wstring selected_video_name(const SelectedVideoSourceInfo& source) {
    if (source.identifier.empty()) return L"None selected";
    if (!source.display_name.empty()) return wide(source.display_name);
    return L"Unavailable saved source";
}
std::wstring monitor_name(const std::optional<RuntimeMonitorDescriptor>& monitor) {
    if (!monitor) return L"Not identified";
    if (!monitor->label.empty()) return wide(monitor->label);
    if (!monitor->gdi_display_name.empty()) return wide(monitor->gdi_display_name);
    return L"Unidentified display";
}
std::wstring output_name(const Source source, const SelectedVideoSourceInfo& video,
                         const std::optional<RuntimeMonitorDescriptor>& monitor) {
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
std::wstring event_line(const LogEvent& event) {
    return wide(format_event_summary(event));
}
}

TrayWindow::TrayWindow(const HINSTANCE instance, App& app) : instance_(instance), app_(app) {
    INITCOMMONCONTROLSEX controls{sizeof(controls), ICC_STANDARD_CLASSES | ICC_TAB_CLASSES};
    InitCommonControlsEx(&controls);
    WNDCLASSEXW window_class{sizeof(window_class)};
    window_class.hInstance = instance_;
    window_class.lpfnWndProc = window_proc;
    window_class.lpszClassName = L"AutomaticScreenCameraWindow";
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    window_class.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    app_icon_large_ = create_app_icon(32);
    app_icon_small_ = create_app_icon(16);
    window_class.hIcon = app_icon_large_;
    window_class.hIconSm = app_icon_small_;
    RegisterClassExW(&window_class);
    constexpr DWORD window_style = WS_OVERLAPPEDWINDOW;
    constexpr DWORD window_ex_style = WS_EX_CONTROLPARENT;
    const UINT system_dpi = GetDpiForSystem();
    const UINT dpi = system_dpi == 0 ? 96U : system_dpi;
    RECT initial_bounds{0, 0, MulDiv(790, static_cast<int>(dpi), 96),
                        MulDiv(720, static_cast<int>(dpi), 96)};
    if (!AdjustWindowRectExForDpi(&initial_bounds, window_style, FALSE, window_ex_style, dpi))
        AdjustWindowRectEx(&initial_bounds, window_style, FALSE, window_ex_style);
    window_ = CreateWindowExW(window_ex_style, window_class.lpszClassName, L"Automatic Screen Camera", window_style,
                              CW_USEDEFAULT, CW_USEDEFAULT,
                              initial_bounds.right - initial_bounds.left, initial_bounds.bottom - initial_bounds.top,
                              nullptr, nullptr, instance_, this);
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
    if (output_bitmap_) DeleteObject(output_bitmap_);
    if (app_icon_large_) DestroyIcon(app_icon_large_);
    if (app_icon_small_) DestroyIcon(app_icon_small_);
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
        if (wparam == timer_id) {
            refresh();
        } else if (wparam == preview_timer_id) {
            refresh_preview();
        }
        return 0;
    case WM_COMMAND: {
        const UINT id = LOWORD(wparam);
        const UINT notification = HIWORD(wparam);
        const HWND control = reinterpret_cast<HWND>(lparam);
        if (id == output_info || id == reference_info || id == display_info || id == health_info) {
            if (notification == BN_SETFOCUS) tooltips_.show_for_focus(control, true);
            else if (notification == BN_KILLFOCUS) tooltips_.show_for_focus(control, false);
        }
        if (lparam == 0 || notification == BN_CLICKED) dispatch_command(id);
        return 0;
    }
    case WM_SIZE:
        layout_controls();
        if (wparam == SIZE_MINIMIZED) KillTimer(window_, preview_timer_id);
        else if (IsWindowVisible(window_)) SetTimer(window_, preview_timer_id, 1000, nullptr);
        return 0;
    case WM_GETMINMAXINFO: {
        auto* bounds = reinterpret_cast<MINMAXINFO*>(lparam);
        const UINT dpi = window_dpi(window_);
        const int minimum_client_height = details_expanded_ ? 804 : 692;
        RECT minimum{0, 0, dip(window_, 720), dip(window_, minimum_client_height)};
        const auto style = static_cast<DWORD>(GetWindowLongPtrW(window_, GWL_STYLE));
        const auto ex_style = static_cast<DWORD>(GetWindowLongPtrW(window_, GWL_EXSTYLE));
        if (!AdjustWindowRectExForDpi(&minimum, style, FALSE, ex_style, dpi))
            AdjustWindowRectEx(&minimum, style, FALSE, ex_style);
        bounds->ptMinTrackSize.x = minimum.right - minimum.left;
        bounds->ptMinTrackSize.y = minimum.bottom - minimum.top;
        return 0;
    }
    case WM_CTLCOLORSTATIC: {
        const HDC dc = reinterpret_cast<HDC>(wparam);
        const HWND control = reinterpret_cast<HWND>(lparam);
        HIGHCONTRASTW contrast{sizeof(contrast)};
        const bool high_contrast = SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(contrast), &contrast, 0) &&
                                   (contrast.dwFlags & HCF_HIGHCONTRASTON) != 0;
        if (!high_contrast) {
            if (control == warning_banner_) SetTextColor(dc, RGB(180, 35, 35));
            else if (control == override_banner_) SetTextColor(dc, RGB(112, 55, 155));
            else if (control == run_label_) {
                SetTextColor(dc, presentation_.run_label == "Running" ? RGB(28, 128, 66) :
                                 presentation_.run_label == "Error" ? RGB(180, 35, 35) : RGB(95, 95, 95));
            } else if (control == reference_value_) {
                SetTextColor(dc, presentation_.reference_label == "Detected" ? RGB(28, 128, 66) :
                                 presentation_.reference_label == "Missing" ? RGB(180, 35, 35) : RGB(115, 95, 25));
            } else if (control == health_value_) {
                SetTextColor(dc, presentation_.health_label == "All components ready" ? RGB(28, 128, 66) : RGB(180, 70, 35));
            }
        }
        SetBkMode(dc, TRANSPARENT);
        return reinterpret_cast<LRESULT>(GetSysColorBrush(COLOR_WINDOW));
    }
    case WM_DEVICECHANGE: app_.log_system_event("DISPLAY_DEVICE_CHANGED", "A display or video device changed"); app_.restart_screen_capture(); app_.request_rescan(); return 0;
    case WM_DPICHANGED: {
        const auto* suggested = reinterpret_cast<const RECT*>(lparam);
        if (suggested) SetWindowPos(window_, nullptr, suggested->left, suggested->top,
                                    suggested->right - suggested->left, suggested->bottom - suggested->top,
                                    SWP_NOACTIVATE | SWP_NOZORDER);
        if (fonts_) {
            fonts_->recreate(window_);
            set_children_font(window_, fonts_->body());
            set_control_font(title_label_, fonts_->title());
            for (const HWND heading : {output_heading_, mode_heading_, activity_heading_}) set_control_font(heading, fonts_->section());
        }
        layout_controls();
        app_.log_system_event("DISPLAY_SCALING_CHANGED", "Display scaling changed");
        app_.restart_screen_capture();
        app_.request_rescan();
        return 0;
    }
    case tray_message:
        if (LOWORD(lparam) == WM_LBUTTONUP || LOWORD(lparam) == WM_LBUTTONDBLCLK) show();
        else if (LOWORD(lparam) == WM_RBUTTONUP || LOWORD(lparam) == WM_CONTEXTMENU) { POINT p{}; GetCursorPos(&p); show_tray_menu(p); }
        return 0;
    case WM_CLOSE:
        if (!exiting_ && app_.config().close_to_tray) { hide(); return 0; }
        if (!exiting_ && app_.config().confirm_exit &&
            MessageBoxW(window_, L"Exit Automatic Screen Camera? The virtual camera will stop.", L"Confirm exit", MB_YESNO | MB_ICONQUESTION) != IDYES) return 0;
        DestroyWindow(window_); return 0;
    case WM_DESTROY:
        KillTimer(window_, timer_id);
        KillTimer(window_, preview_timer_id);
        PostQuitMessage(0);
        return 0;
    default: return DefWindowProcW(window_, message, wparam, lparam);
    }
}

void TrayWindow::create_controls() {
    const auto label = [this](const wchar_t* text, const DWORD style = SS_LEFT) {
        return CreateWindowExW(0, L"STATIC", text, WS_CHILD | WS_VISIBLE | style,
                               0, 0, 0, 0, window_, nullptr, instance_, nullptr);
    };
    const auto button = [this](const wchar_t* text, const UINT id, const DWORD extra = 0U,
                               const bool tab_stop = true) {
        const DWORD style = WS_CHILD | WS_VISIBLE | extra | (tab_stop ? WS_TABSTOP : 0U);
        return CreateWindowExW(0, L"BUTTON", text, style,
                               0, 0, 0, 0, window_, control_id(id), instance_, nullptr);
    };

    title_label_ = label(L"Automatic Screen Camera");
    run_label_ = label(L"● Starting");
    start_stop_button_ = button(L"Start", start);
    warning_banner_ = label(L"");
    override_banner_ = label(L"MANUAL OVERRIDE — Automatic switching is paused.");
    return_automatic_button_ = button(L"Return to Automatic", return_automatic);

    output_heading_ = label(L"CURRENT OUTPUT");
    output_preview_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"STATIC", nullptr,
                                      WS_CHILD | WS_VISIBLE | SS_BITMAP | SS_CENTERIMAGE,
                                      0, 0, 0, 0, window_, nullptr, instance_, nullptr);
    output_kind_ = label(L"Preparing output…");
    output_name_ = label(L"", SS_LEFT | SS_EDITCONTROL);
    output_info_ = button(L"ⓘ", output_info, BS_NOTIFY);

    mode_heading_ = label(L"OUTPUT MODE");
    mode_buttons_[0] = button(L"Automatic", automatic, BS_AUTORADIOBUTTON | WS_GROUP);
    mode_buttons_[1] = button(L"Webcam / video", force_camera, BS_AUTORADIOBUTTON, false);
    mode_buttons_[2] = button(L"Screen", force_screen, BS_AUTORADIOBUTTON, false);

    reference_caption_ = label(L"Reference");
    reference_value_ = label(L"Unknown");
    reference_info_ = button(L"ⓘ", reference_info, BS_NOTIFY | WS_GROUP);
    display_caption_ = label(L"Tracked display");
    display_value_ = label(L"Not identified", SS_LEFT | SS_ENDELLIPSIS);
    display_info_ = button(L"ⓘ", display_info, BS_NOTIFY);
    health_caption_ = label(L"System health");
    health_value_ = label(L"Starting");
    health_info_ = button(L"ⓘ", health_info, BS_NOTIFY);

    set_reference_button_ = button(L"Set current screen as reference", set_reference);
    rescan_button_ = button(L"Rescan displays", rescan);

    activity_heading_ = label(L"RECENT ACTIVITY");
    view_all_button_ = button(L"View all", view_all, BS_FLAT);
    for (auto& line : activity_lines_) line = label(L"No recent activity", SS_LEFT | SS_ENDELLIPSIS);

    previews_button_ = button(L"Previews", show_previews);
    settings_button_ = button(L"Settings", settings);
    details_button_ = button(L"Technical details ▾", toggle_details);
    technical_text_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", nullptr,
                                      WS_CHILD | WS_TABSTOP | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL,
                                      0, 0, 0, 0, window_, nullptr, instance_, nullptr);
    full_activity_list_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"LISTBOX", nullptr,
                                          WS_CHILD | WS_TABSTOP | WS_VSCROLL | WS_HSCROLL | LBS_NOINTEGRALHEIGHT | LBS_NOTIFY,
                                          0, 0, 0, 0, window_, nullptr, instance_, nullptr);

    fonts_ = std::make_unique<UiFonts>(window_);
    set_children_font(window_, fonts_->body());
    set_control_font(title_label_, fonts_->title());
    for (const HWND heading : {output_heading_, mode_heading_, activity_heading_}) set_control_font(heading, fonts_->section());
    set_control_font(output_kind_, fonts_->section());

    tooltips_.create(window_, instance_);
    ShowWindow(warning_banner_, SW_HIDE);
    ShowWindow(override_banner_, SW_HIDE);
    ShowWindow(return_automatic_button_, SW_HIDE);
    ShowWindow(technical_text_, SW_HIDE);
    ShowWindow(full_activity_list_, SW_HIDE);
    layout_controls();
    refresh();
}

void TrayWindow::layout_controls() {
    if (!title_label_) return;
    RECT client{};
    GetClientRect(window_, &client);
    const int pad = dip(window_, 20);
    const int gap = dip(window_, 16);
    const int width = client.right - client.left;
    const int content_width = std::max(1, width - pad * 2);
    const auto banners = dashboard_banner_visibility(presentation_);
    const int header_y = dip(window_, 14);
    const int header_height = dip(window_, 34);
    MoveWindow(title_label_, pad, header_y, content_width - dip(window_, 260), header_height, TRUE);
    MoveWindow(run_label_, width - pad - dip(window_, 205), header_y + dip(window_, 5), dip(window_, 105), dip(window_, 24), TRUE);
    MoveWindow(start_stop_button_, width - pad - dip(window_, 92), header_y, dip(window_, 92), dip(window_, 32), TRUE);

    const int banner_y = dip(window_, 54);
    MoveWindow(warning_banner_, pad, banner_y, content_width, dip(window_, 30), TRUE);
    const int override_y = banner_y + (banners.show_warning ? dip(window_, 36) : 0);
    MoveWindow(override_banner_, pad, override_y, content_width - dip(window_, 180), dip(window_, 30), TRUE);
    MoveWindow(return_automatic_button_, width - pad - dip(window_, 172), override_y - dip(window_, 3), dip(window_, 172), dip(window_, 30), TRUE);
    const int top = banners.row_count == 0 ? dip(window_, 64)
                                           : banner_y + dip(window_, 42 + (banners.row_count - 1) * 36);

    MoveWindow(output_heading_, pad, top, content_width, dip(window_, 22), TRUE);
    const int preview_y = top + dip(window_, 28);
    const int preview_width = std::clamp(content_width / 2 - gap / 2, dip(window_, 280), dip(window_, 320));
    const int preview_height = preview_width * 9 / 16;
    MoveWindow(output_preview_, pad, preview_y, preview_width, preview_height, TRUE);

    const int right_x = pad + preview_width + gap;
    const int right_width = width - pad - right_x;
    MoveWindow(output_kind_, right_x, preview_y + dip(window_, 12), right_width - dip(window_, 36), dip(window_, 26), TRUE);
    MoveWindow(output_info_, width - pad - dip(window_, 30), preview_y + dip(window_, 7), dip(window_, 30), dip(window_, 28), TRUE);
    MoveWindow(output_name_, right_x, preview_y + dip(window_, 48), right_width, dip(window_, 58), TRUE);

    const int mode_y = preview_y + preview_height + dip(window_, 12);
    MoveWindow(mode_heading_, pad, mode_y, content_width, dip(window_, 22), TRUE);
    const int radio_y = mode_y + dip(window_, 22);
    MoveWindow(mode_buttons_[0], pad, radio_y, dip(window_, 130), dip(window_, 26), TRUE);
    MoveWindow(mode_buttons_[1], pad + dip(window_, 142), radio_y, dip(window_, 170), dip(window_, 26), TRUE);
    MoveWindow(mode_buttons_[2], pad + dip(window_, 324), radio_y, dip(window_, 120), dip(window_, 26), TRUE);

    const int status_y = radio_y + dip(window_, 34);
    const int caption_width = dip(window_, 118);
    const int info_width = dip(window_, 30);
    const auto move_status_row = [&](const HWND caption, const HWND value, const HWND info, const int y) {
        MoveWindow(caption, pad, y, caption_width, dip(window_, 26), TRUE);
        MoveWindow(value, pad + caption_width, y, content_width - caption_width - info_width, dip(window_, 26), TRUE);
        MoveWindow(info, width - pad - info_width, y - dip(window_, 3), info_width, dip(window_, 28), TRUE);
    };
    move_status_row(reference_caption_, reference_value_, reference_info_, status_y);
    move_status_row(display_caption_, display_value_, display_info_, status_y + dip(window_, 29));
    move_status_row(health_caption_, health_value_, health_info_, status_y + dip(window_, 58));

    const int action_y = status_y + dip(window_, 90);
    MoveWindow(set_reference_button_, pad, action_y, dip(window_, 246), dip(window_, 32), TRUE);
    MoveWindow(rescan_button_, pad + dip(window_, 258), action_y, dip(window_, 130), dip(window_, 32), TRUE);

    const int activity_y = action_y + dip(window_, 42);
    MoveWindow(activity_heading_, pad, activity_y, dip(window_, 220), dip(window_, 22), TRUE);
    MoveWindow(view_all_button_, width - pad - dip(window_, 82), activity_y - dip(window_, 5), dip(window_, 82), dip(window_, 28), TRUE);
    for (std::size_t index = 0; index < activity_lines_.size(); ++index)
        MoveWindow(activity_lines_[index], pad, activity_y + dip(window_, 26 + static_cast<int>(index) * 24), content_width, dip(window_, 22), TRUE);

    const int footer_y = activity_y + dip(window_, 100);
    MoveWindow(previews_button_, pad, footer_y, dip(window_, 100), dip(window_, 32), TRUE);
    MoveWindow(settings_button_, pad + dip(window_, 112), footer_y, dip(window_, 100), dip(window_, 32), TRUE);
    MoveWindow(details_button_, width - pad - dip(window_, 170), footer_y, dip(window_, 170), dip(window_, 32), TRUE);

    if (details_expanded_) {
        const int details_y = footer_y + dip(window_, 44);
        const int available_height = std::max(
            dip(window_, 100), static_cast<int>(client.bottom) - details_y - pad);
        const int half = (content_width - gap) / 2;
        MoveWindow(technical_text_, pad, details_y, half, available_height, TRUE);
        MoveWindow(full_activity_list_, pad + half + gap, details_y, content_width - half - gap, available_height, TRUE);
    }
}

void TrayWindow::set_details_expanded(const bool expanded, const bool resize_window) {
    if (details_expanded_ == expanded) return;
    details_expanded_ = expanded;
    ShowWindow(technical_text_, expanded ? SW_SHOW : SW_HIDE);
    ShowWindow(full_activity_list_, expanded ? SW_SHOW : SW_HIDE);
    SetWindowTextW(details_button_, expanded ? L"Technical details ▴" : L"Technical details ▾");
    if (resize_window) {
        RECT bounds{};
        GetWindowRect(window_, &bounds);
        const int delta = dip(window_, 250) * (expanded ? 1 : -1);
        int height = std::max(dip(window_, 680),
                              static_cast<int>(bounds.bottom - bounds.top) + delta);
        if (expanded) {
            MONITORINFO monitor{sizeof(monitor)};
            if (GetMonitorInfoW(MonitorFromWindow(window_, MONITOR_DEFAULTTONEAREST), &monitor))
                height = std::min(height,
                                  static_cast<int>(monitor.rcWork.bottom - bounds.top));
        }
        SetWindowPos(window_, nullptr, 0, 0, bounds.right - bounds.left, height,
                     SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    layout_controls();
}

void TrayWindow::update_technical_details() {
    if (!technical_text_ || displayed_technical_details_ == presentation_.technical_details) return;
    if (GetFocus() == technical_text_ || GetCapture() == technical_text_) return;

    const auto text = wide(presentation_.technical_details);
    DWORD selection_start = 0;
    DWORD selection_end = 0;
    SendMessageW(technical_text_, EM_GETSEL, reinterpret_cast<WPARAM>(&selection_start),
                 reinterpret_cast<LPARAM>(&selection_end));
    const int first_visible_line = static_cast<int>(SendMessageW(technical_text_, EM_GETFIRSTVISIBLELINE, 0, 0));
    SendMessageW(technical_text_, WM_SETREDRAW, FALSE, 0);
    SetWindowTextW(technical_text_, text.c_str());
    displayed_technical_details_ = presentation_.technical_details;
    SendMessageW(technical_text_, EM_SETSEL, selection_start, selection_end);
    const int current_first_line = static_cast<int>(SendMessageW(technical_text_, EM_GETFIRSTVISIBLELINE, 0, 0));
    if (current_first_line != first_visible_line)
        SendMessageW(technical_text_, EM_LINESCROLL, 0, first_visible_line - current_first_line);
    SendMessageW(technical_text_, WM_SETREDRAW, TRUE, 0);
    InvalidateRect(technical_text_, nullptr, FALSE);
}

void TrayWindow::refresh() {
    const auto state = app_.status();
    const auto config = app_.config();
    const auto selected_video = app_.selected_video_source();
    presentation_ = build_dashboard_presentation(state, config,
                                                  {selected_video.identifier, selected_video.display_name},
                                                  app_.recent_events());

    const auto banners = dashboard_banner_visibility(presentation_);
    const bool show_warning = banners.show_warning;
    const bool show_override = banners.show_override;
    const bool warning_visible = IsWindowVisible(warning_banner_) != FALSE;
    const bool override_visible = IsWindowVisible(override_banner_) != FALSE;
    const bool banner_changed = warning_visible != show_warning || override_visible != show_override;
    ShowWindow(warning_banner_, show_warning ? SW_SHOW : SW_HIDE);
    ShowWindow(override_banner_, show_override ? SW_SHOW : SW_HIDE);
    ShowWindow(return_automatic_button_, show_override ? SW_SHOW : SW_HIDE);
    if (show_warning) {
        SetWindowTextW(warning_banner_, wide("WARNING — " + presentation_.warning).c_str());
        tooltips_.set(warning_banner_, wide(presentation_.warning));
    }

    SetWindowTextW(run_label_, wide("● " + presentation_.run_label).c_str());
    SetWindowTextW(output_kind_, wide(presentation_.output_kind).c_str());
    SetWindowTextW(output_name_, wide(presentation_.output_name).c_str());
    SetWindowTextW(reference_value_, wide(presentation_.reference_label).c_str());
    SetWindowTextW(display_value_, wide(presentation_.display_label).c_str());
    SetWindowTextW(health_value_, wide(presentation_.health_label).c_str());
    update_technical_details();
    SetWindowTextW(start_stop_button_, app_.automation_running() ? L"Stop" : L"Start");
    SetWindowLongPtrW(start_stop_button_, GWLP_ID, app_.automation_running() ? stop : start);
    CheckRadioButton(window_, automatic, force_screen, state.mode == OutputMode::automatic ? automatic : state.mode == OutputMode::force_camera ? force_camera : force_screen);

    for (std::size_t index = 0; index < activity_lines_.size(); ++index) {
        const std::string line = index < presentation_.recent_activity.size() ? presentation_.recent_activity[index] :
                                 index == 0 ? "No recent activity" : "";
        SetWindowTextW(activity_lines_[index], wide(line).c_str());
        const std::string detail = index < presentation_.full_activity.size() ? presentation_.full_activity[index] : line;
        tooltips_.set(activity_lines_[index], wide(detail));
    }
    if (displayed_activity_ != presentation_.full_activity) {
        displayed_activity_ = presentation_.full_activity;
        SendMessageW(full_activity_list_, LB_RESETCONTENT, 0, 0);
        for (const auto& event : displayed_activity_) {
            const auto line = wide(event);
            SendMessageW(full_activity_list_, LB_ADDSTRING, 0, reinterpret_cast<LPARAM>(line.c_str()));
        }
    }

    tooltips_.set(output_kind_, wide(presentation_.output_tooltip));
    tooltips_.set(output_name_, wide(presentation_.output_tooltip));
    tooltips_.set(output_info_, wide(presentation_.output_tooltip), true);
    tooltips_.set(reference_value_, wide(presentation_.reference_tooltip));
    tooltips_.set(reference_info_, wide(presentation_.reference_tooltip), true);
    tooltips_.set(display_value_, wide(presentation_.display_tooltip));
    tooltips_.set(display_info_, wide(presentation_.display_tooltip), true);
    tooltips_.set(health_value_, wide(presentation_.health_tooltip));
    tooltips_.set(health_info_, wide(presentation_.health_tooltip), true);

    if (banner_changed) layout_controls();
    InvalidateRect(window_, nullptr, FALSE);
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

HBITMAP TrayWindow::create_preview_bitmap(const PreviewImage& image) const {
    if (image.bgra.empty()) return nullptr;
    BITMAPINFO info{};
    info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = static_cast<LONG>(image.size.width);
    info.bmiHeader.biHeight = -static_cast<LONG>(image.size.height);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    void* pixels = nullptr;
    const HDC dc = GetDC(nullptr);
    const HBITMAP bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &pixels, nullptr, 0);
    ReleaseDC(nullptr, dc);
    if (bitmap && pixels) std::memcpy(pixels, image.bgra.data(), image.bgra.size());
    return bitmap;
}

void TrayWindow::refresh_preview() {
    if (!IsWindowVisible(window_) || IsIconic(window_) || !output_preview_) return;
    RECT bounds{};
    GetClientRect(output_preview_, &bounds);
    const auto target = Size{static_cast<std::uint32_t>(std::max(1L, bounds.right)),
                             static_cast<std::uint32_t>(std::max(1L, bounds.bottom))};
    HBITMAP bitmap = nullptr;
    try {
        if (const auto image = app_.preview(PreviewKind::output, target)) bitmap = create_preview_bitmap(*image);
    } catch (...) {}
    const auto old = reinterpret_cast<HBITMAP>(SendMessageW(output_preview_, STM_SETIMAGE, IMAGE_BITMAP,
                                                             reinterpret_cast<LPARAM>(bitmap)));
    if (old) DeleteObject(old);
    output_bitmap_ = bitmap;
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
                label << wide(monitors[i].descriptor.label) << L" — " << monitors[i].descriptor.geometry.width << L" × "
                      << monitors[i].descriptor.geometry.height << L" at (" << monitors[i].descriptor.geometry.x << L", " << monitors[i].descriptor.geometry.y << L")";
                AppendMenuW(choices, MF_STRING, static_cast<UINT_PTR>(2000 + i), label.str().c_str());
            }
            POINT point{}; GetCursorPos(&point);
            const auto selected = TrackPopupMenu(choices, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, 0, window_, nullptr);
            DestroyMenu(choices);
            if (selected >= 2000 && static_cast<std::size_t>(selected - 2000) < monitors.size()) {
                hide(); app_.set_reference_monitor(monitors[static_cast<std::size_t>(selected - 2000)].descriptor);
            }
        } else if (monitors.size() == 1) {
            // This is an explicit user choice, so it may replace a persisted
            // monitor that is currently disconnected.
            hide(); app_.set_reference_monitor(monitors.front().descriptor);
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
    case toggle_details:
        set_details_expanded(!details_expanded_);
        break;
    case view_all:
        set_details_expanded(true);
        SetFocus(full_activity_list_);
        break;
    case output_info: case reference_info: case display_info: case health_info:
        set_details_expanded(true);
        SetFocus(technical_text_);
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

int TrayWindow::message_loop() {
    MSG message{};
    while (GetMessageW(&message, nullptr, 0, 0) > 0) {
        if (!IsDialogMessageW(window_, &message)) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    return static_cast<int>(message.wParam);
}
void TrayWindow::show() {
    ShowWindow(window_, SW_SHOW);
    SetForegroundWindow(window_);
    refresh();
    refresh_preview();
    SetTimer(window_, preview_timer_id, 1000, nullptr);
}
void TrayWindow::hide() {
    KillTimer(window_, preview_timer_id);
    ShowWindow(window_, SW_HIDE);
}
void TrayWindow::close() { exiting_ = true; PostMessageW(window_, WM_CLOSE, 0, 0); }

} // namespace asc::win
