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
    video_device = 1000, video_details, cursor, tracked_monitor, threshold, placeholder_color,
    start_windows, start_minimized, start_auto, close_tray, confirm_exit, notifications,
    save_button, cancel_button, restart_input_button, restart_capture_button, restart_vcam_button,
    set_reference_settings, import_reference_settings, preview_button,
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
    : owner_(owner), instance_(instance), app_(app), working_(app.config()), devices_(app.video_devices()), monitors_(app.monitors()) {
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
        case save_button: save(); return 0;
        case cancel_button: SendMessageW(window_, WM_CLOSE, 0, 0); return 0;
        case restart_input_button: app_.restart_video_input(); return 0;
        case restart_capture_button: app_.restart_screen_capture(); return 0;
        case restart_vcam_button: app_.restart_virtual_camera(); return 0;
        case restart_all_button: app_.restart_all(); return 0;
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
            std::optional<RuntimeMonitorDescriptor> selected_monitor;
            if (monitors.size() > 1) {
                HMENU choices = CreatePopupMenu();
                for (std::size_t i = 0; i < monitors.size(); ++i) {
                    std::wostringstream label; label << wide(monitors[i].descriptor.label) << L" — " << monitors[i].descriptor.geometry.width << L"×" << monitors[i].descriptor.geometry.height;
                    AppendMenuW(choices, MF_STRING, static_cast<UINT_PTR>(3000 + i), label.str().c_str());
                }
                POINT point{}; GetCursorPos(&point);
                const auto choice = TrackPopupMenu(choices, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, 0, window_, nullptr);
                DestroyMenu(choices);
                if (choice >= 3000 && static_cast<std::size_t>(choice - 3000) < monitors.size()) selected_monitor = monitors[static_cast<std::size_t>(choice - 3000)].descriptor;
            } else if (monitors.size() == 1) selected_monitor = monitors.front().descriptor;
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
    std::wstring status = L"No video input selected";
    std::wstring details = status;
    if (selected > 0 && static_cast<std::size_t>(selected) < video_option_ids_.size()) {
        const auto& identifier = video_option_ids_[static_cast<std::size_t>(selected)];
        const auto device = std::find_if(devices_.begin(), devices_.end(), [&](const auto& candidate) {
            return candidate.identifier == identifier;
        });
        if (device == devices_.end()) {
            status = L"⚠ Saved source unavailable";
            details = L"Status: Unavailable\r\nIdentifier: " + wide(identifier) +
                      L"\r\nCapture contract: 1280×720, 30 fps, RGB32";
        } else {
            status = L"✓ Connected";
            details = L"Status: Connected\r\nIdentifier: " + wide(identifier) +
                      L"\r\nCapture contract: 1280×720, 30 fps, RGB32";
        }
    }
    SetWindowTextW(GetDlgItem(window_, video_details), details.c_str());
    if (device_status_) SetWindowTextW(device_status_, status.c_str());
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
    constexpr int logs_page = 4;

    tabs_ = CreateWindowExW(0, WC_TABCONTROLW, nullptr, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS,
                            0, 0, 0, 0, window_, nullptr, instance_, nullptr);
    for (const wchar_t* title : {L"General", L"Sources", L"Detection", L"Output", L"Logs"}) {
        TCITEMW item{}; item.mask = TCIF_TEXT; item.pszText = const_cast<wchar_t*>(title);
        TabCtrl_InsertItem(tabs_, TabCtrl_GetItemCount(tabs_), &item);
    }
    const auto heading = [this](const wchar_t* value, const int page, const int y) {
        const auto control = add_label(value, page, 16, y, 320, 24);
        section_labels_.push_back(control);
    };

    heading(L"STARTUP", general_page, 12);
    add_checkbox(start_windows, L"Start with Windows", working_.start_with_windows, general_page, 16, 46, 240);
    add_checkbox(start_minimized, L"Start minimized to tray", working_.start_minimized, general_page, 300, 46, 240);
    add_checkbox(start_auto, L"Start detection automatically", working_.start_automatically, general_page, 16, 82, 280);
    heading(L"WINDOW BEHAVIOR", general_page, 130);
    add_checkbox(close_tray, L"Close main window to tray", working_.close_to_tray, general_page, 16, 164, 260);
    add_checkbox(confirm_exit, L"Confirm before exiting", working_.confirm_exit, general_page, 300, 164, 240);
    add_checkbox(notifications, L"Show status notifications", working_.show_notifications, general_page, 16, 200, 260);

    heading(L"WEBCAM / VIDEO", sources_page, 12);
    add_label(L"Video source", sources_page, 16, 52, 140);
    const auto device = add_combo(video_device, sources_page, 170, 46, 20, true);
    std::vector<std::string> identifiers;
    for (const auto& available : devices_) identifiers.push_back(available.identifier);
    const auto choices = build_persistent_device_choices(identifiers, working_.selected_video_device_id);
    video_option_ids_ = choices.identifiers;
    combo_add(device, L"No video input selected");
    for (const auto& available : devices_) combo_add(device, wide(available.name).c_str());
    if (choices.configured_device_unavailable)
        combo_add(device, (std::wstring(L"Unavailable saved source — ") + wide(working_.selected_video_device_id)).c_str());
    SendMessageW(device, CB_SETCURSEL, static_cast<WPARAM>(choices.selected_index), 0);
    device_status_ = add_label(L"Checking connection…", sources_page, 170, 82, 350, 24);
    device_details_button_ = add_button(toggle_device_details_button, L"Show details", sources_page, 550, 78, 140, 28);
    const auto details = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", nullptr,
        WS_CHILD | WS_TABSTOP | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL,
        0, 0, 0, 0, window_, control_id(video_details), instance_, nullptr);
    register_control(details, sources_page, 170, 110, 20, 72, true);
    ShowWindow(details, SW_HIDE);
    add_label(L"Fixed capture: 1280 × 720, 30 fps, RGB32", sources_page, 16, 128, 440);
    add_button(restart_input_button, L"Restart webcam", sources_page, 540, 120, 150);

    heading(L"SCREEN CAPTURE", sources_page, 198);
    add_checkbox(cursor, L"Include mouse cursor", working_.cursor_visible, sources_page, 16, 234, 230);
    add_button(restart_capture_button, L"Restart screen capture", sources_page, 510, 228, 180);
    add_label(L"Current monitor", sources_page, 16, 276, 160);
    const auto monitor_combo = add_combo(tracked_monitor, sources_page, 180, 270, 20, true);
    const auto tracked = app_.status().tracked_monitor;
    int monitor_selected = -1;
    for (std::size_t index = 0; index < monitors_.size(); ++index) {
        const auto& monitor = monitors_[index].descriptor;
        std::wostringstream description;
        description << wide(monitor.label) << L" — " << monitor.geometry.width << L"×" << monitor.geometry.height
                    << L" at (" << monitor.geometry.x << L", " << monitor.geometry.y << L")";
        combo_add(monitor_combo, description.str().c_str());
        if (tracked && tracked->runtime_key() == monitor.runtime_key()) monitor_selected = static_cast<int>(index);
    }
    if (!monitors_.empty()) SendMessageW(monitor_combo, CB_SETCURSEL, std::max(0, monitor_selected), 0);
    add_button(preview_button, L"Open four previews", sources_page, 16, 320, 170);

    heading(L"REFERENCE", detection_page, 12);
    add_label(L"Capture or import the visual that keeps the webcam active in Automatic mode.", detection_page, 16, 48, 20, 24, true);
    add_button(set_reference_settings, L"Set current screen", detection_page, 16, 82, 170);
    add_button(import_reference_settings, L"Import image…", detection_page, 200, 82, 150);
    heading(L"SIMILARITY", detection_page, 142);
    add_label(L"Threshold (%)", detection_page, 16, 180, 160);
    add_edit(threshold, std::to_wstring(working_.similarity_threshold * 100.0), detection_page, 180, 174, 100);
    add_label(L"Fixed behavior: check every 250 ms; 5 matches; 3 mismatches; scan all monitors every 30 seconds; confirm a new monitor twice.",
              detection_page, 16, 224, 20, 72, true);

    heading(L"FIXED FRAME PIPELINE", output_page, 12);
    add_label(L"CPU BGRA composition at 1280 × 720 and 30 fps. Sources use aspect-fit scaling with black letterboxing.",
              output_page, 16, 48, 20, 48, true);
    add_label(L"Switches use a reversible 500 ms fade with the live screen frame.", output_page, 16, 104, 20, 28, true);
    add_button(restart_vcam_button, L"Restart virtual camera", output_page, 16, 154, 190);
    heading(L"SAFE FALLBACK", output_page, 218);
    std::wostringstream color_text;
    color_text << L'#' << std::hex << std::setfill(L'0') << std::setw(6) << (working_.placeholder_color_bgra & 0x00ffffffu);
    add_label(L"Placeholder color", output_page, 16, 258, 160);
    add_edit(placeholder_color, color_text.str(), output_page, 180, 252, 130);
    add_button(restart_all_button, L"Restart all", output_page, 16, 312, 150);

    heading(L"LOGGING", logs_page, 12);
    add_label(L"Logs are retained for 14 days.", logs_page, 16, 50, 300);
    add_button(open_log_button, L"Open log folder", logs_page, 16, 92, 150);
    add_button(export_log_button, L"Export logs…", logs_page, 180, 92, 140);
    add_button(clear_log_button, L"Clear logs", logs_page, 334, 92, 120);

    save_button_ = CreateWindowExW(0, L"BUTTON", L"Save changes", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                                   0, 0, 0, 0, window_, control_id(save_button), instance_, nullptr);
    cancel_button_ = CreateWindowExW(0, L"BUTTON", L"Cancel", WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                                     0, 0, 0, 0, window_, control_id(cancel_button), instance_, nullptr);
    fonts_ = std::make_unique<UiFonts>(window_);
    set_children_font(window_, fonts_->body());
    for (const auto section : section_labels_) set_control_font(section, fonts_->section());
    tooltips_.create(window_, instance_);
    tooltips_.set(GetDlgItem(window_, threshold), L"Minimum reference-image similarity counted as a match. Accepted range: 0–100%.", true);
    tooltips_.set(GetDlgItem(window_, placeholder_color), L"Safe fallback color in #RRGGBB format.", true);
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
        working_.cursor_visible = checked(cursor);
        working_.similarity_threshold = decimal(threshold, 0.0, 100.0) / 100.0;
        auto color = text(placeholder_color);
        if (!color.empty() && color.front() == L'#') color.erase(color.begin());
        std::size_t used = 0;
        const auto rgb = std::stoul(color, &used, 16);
        if (used != 6 || color.size() != 6) throw std::invalid_argument("Placeholder color must use #RRGGBB format");
        working_.placeholder_color_bgra = 0xff000000u | static_cast<std::uint32_t>(rgb);
        working_.start_with_windows = checked(start_windows);
        working_.start_minimized = checked(start_minimized);
        working_.start_automatically = checked(start_auto);
        working_.close_to_tray = checked(close_tray);
        working_.confirm_exit = checked(confirm_exit);
        working_.show_notifications = checked(notifications);
        app_.apply_settings(working_);
        const auto monitor_index = combo_selection(window_, tracked_monitor);
        if (monitor_index >= 0 && static_cast<std::size_t>(monitor_index) < monitors_.size())
            app_.select_tracked_monitor(monitors_[static_cast<std::size_t>(monitor_index)].descriptor);
        DestroyWindow(window_);
    } catch (const std::exception& error) {
        MessageBoxA(window_, error.what(), "Invalid settings", MB_OK | MB_ICONWARNING);
    }
}

} // namespace asc::win
