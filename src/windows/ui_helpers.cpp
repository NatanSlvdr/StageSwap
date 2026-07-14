#include "ui_helpers.hpp"

#include <algorithm>
#include <cstdint>
#include <cstdlib>
#include <utility>

namespace asc::win {
namespace {

HFONT create_font_for_dpi(const UINT dpi, const int point_delta, const LONG weight) {
    NONCLIENTMETRICSW metrics{sizeof(metrics)};
    if (!SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, sizeof(metrics), &metrics, 0, dpi)) {
        SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, sizeof(metrics), &metrics, 0);
    }
    LOGFONTW font = metrics.lfMessageFont;
    font.lfHeight -= MulDiv(point_delta, static_cast<int>(dpi), 72);
    font.lfWeight = weight;
    return CreateFontIndirectW(&font);
}

BOOL CALLBACK apply_font(HWND child, const LPARAM font) {
    SendMessageW(child, WM_SETFONT, static_cast<WPARAM>(font), TRUE);
    return TRUE;
}

} // namespace

UINT window_dpi(const HWND window) noexcept {
    const UINT value = GetDpiForWindow(window);
    return value == 0 ? 96U : value;
}

int dip(const HWND window, const int value) noexcept {
    return MulDiv(value, static_cast<int>(window_dpi(window)), 96);
}

void set_control_font(const HWND control, const HFONT font) {
    if (control && font) SendMessageW(control, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
}

void set_children_font(const HWND window, const HFONT font) {
    if (!window || !font) return;
    EnumChildWindows(window, apply_font, reinterpret_cast<LPARAM>(font));
}

UiFonts::UiFonts(const HWND window) { recreate(window); }

UiFonts::~UiFonts() { clear(); }

void UiFonts::clear() noexcept {
    if (body_) DeleteObject(body_);
    if (title_) DeleteObject(title_);
    if (section_) DeleteObject(section_);
    body_ = title_ = section_ = nullptr;
}

void UiFonts::recreate(const HWND window) {
    clear();
    const UINT dpi = window_dpi(window);
    body_ = create_font_for_dpi(dpi, 0, FW_NORMAL);
    title_ = create_font_for_dpi(dpi, 6, FW_SEMIBOLD);
    section_ = create_font_for_dpi(dpi, 1, FW_SEMIBOLD);
}

TooltipHost::~TooltipHost() {
    if (hover_) DestroyWindow(hover_);
    if (focus_) DestroyWindow(focus_);
}

void TooltipHost::create(const HWND owner, const HINSTANCE instance) {
    owner_ = owner;
    hover_ = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, nullptr,
                             WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                             CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
                             owner, nullptr, instance, nullptr);
    focus_ = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, nullptr,
                             WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                             CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
                             owner, nullptr, instance, nullptr);
    if (hover_) {
        SendMessageW(hover_, TTM_SETMAXTIPWIDTH, 0, dip(owner, 380));
        SendMessageW(hover_, TTM_SETDELAYTIME, TTDT_INITIAL, 450);
        SetWindowPos(hover_, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
    if (focus_) {
        SendMessageW(focus_, TTM_SETMAXTIPWIDTH, 0, dip(owner, 380));
        SetWindowPos(focus_, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

void TooltipHost::update_tool(const HWND tooltip, const HWND control, const std::wstring& value, const UINT flags) {
    if (!tooltip || !control) return;
    TOOLINFOW tool{sizeof(tool)};
    tool.uFlags = flags;
    tool.hwnd = owner_;
    tool.uId = reinterpret_cast<UINT_PTR>(control);
    tool.lpszText = const_cast<wchar_t*>(value.c_str());
    if (SendMessageW(tooltip, TTM_GETTOOLINFOW, 0, reinterpret_cast<LPARAM>(&tool))) {
        tool.lpszText = const_cast<wchar_t*>(value.c_str());
        SendMessageW(tooltip, TTM_UPDATETIPTEXTW, 0, reinterpret_cast<LPARAM>(&tool));
    } else {
        SendMessageW(tooltip, TTM_ADDTOOLW, 0, reinterpret_cast<LPARAM>(&tool));
    }
}

void TooltipHost::set(const HWND control, std::wstring value, const bool focus_tracking) {
    text_[control] = std::move(value);
    focus_tracking_[control] = focus_tracking;
    const auto& stored = text_.at(control);
    update_tool(hover_, control, stored, TTF_IDISHWND | TTF_SUBCLASS);
    if (focus_tracking) update_tool(focus_, control, stored, TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE);
}

void TooltipHost::show_for_focus(const HWND control, const bool show) {
    if (!focus_ || !control || !focus_tracking_[control]) return;
    TOOLINFOW tool{sizeof(tool)};
    tool.uFlags = TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE;
    tool.hwnd = owner_;
    tool.uId = reinterpret_cast<UINT_PTR>(control);
    if (show) {
        RECT bounds{};
        GetWindowRect(control, &bounds);
        SendMessageW(focus_, TTM_TRACKPOSITION, 0,
                     MAKELPARAM(bounds.left, bounds.bottom + dip(owner_, 4)));
    }
    SendMessageW(focus_, TTM_TRACKACTIVATE, show ? TRUE : FALSE, reinterpret_cast<LPARAM>(&tool));
}

HICON create_app_icon(const int pixels) {
    const int size = std::max(16, pixels);
    BITMAPV5HEADER header{};
    header.bV5Size = sizeof(header);
    header.bV5Width = size;
    header.bV5Height = -size;
    header.bV5Planes = 1;
    header.bV5BitCount = 32;
    header.bV5Compression = BI_BITFIELDS;
    header.bV5RedMask = 0x00ff0000;
    header.bV5GreenMask = 0x0000ff00;
    header.bV5BlueMask = 0x000000ff;
    header.bV5AlphaMask = 0xff000000;
    void* raw = nullptr;
    const HDC dc = GetDC(nullptr);
    const HBITMAP color = CreateDIBSection(dc, reinterpret_cast<BITMAPINFO*>(&header), DIB_RGB_COLORS, &raw, nullptr, 0);
    ReleaseDC(nullptr, dc);
    if (!color || !raw) {
        if (color) DeleteObject(color);
        return CopyIcon(LoadIconW(nullptr, IDI_APPLICATION));
    }
    auto* pixels_data = static_cast<std::uint32_t*>(raw);
    std::fill(pixels_data, pixels_data + static_cast<std::size_t>(size) * size, 0U);
    const auto put = [&](const int x, const int y, const std::uint32_t value) {
        if (x >= 0 && x < size && y >= 0 && y < size) pixels_data[y * size + x] = value;
    };
    const int margin = std::max(2, size / 7);
    const int radius = std::max(2, size / 8);
    const std::uint32_t body = 0xff2d73dcu;
    const std::uint32_t lens = 0xffffffffu;
    for (int y = margin + radius; y < size - margin; ++y) {
        for (int x = margin; x < size - margin - size / 5; ++x) put(x, y, body);
    }
    for (int y = margin + radius + size / 8; y < size - margin - size / 8; ++y) {
        for (int x = size - margin - size / 5; x < size - margin; ++x) {
            const int inset = std::abs(y - size / 2) / 2;
            if (x >= size - margin - size / 5 + inset) put(x, y, body);
        }
    }
    const int center_x = size / 2 - size / 10;
    const int center_y = size / 2 + size / 12;
    const int lens_radius = std::max(2, size / 7);
    for (int y = -lens_radius; y <= lens_radius; ++y)
        for (int x = -lens_radius; x <= lens_radius; ++x)
            if (x * x + y * y <= lens_radius * lens_radius) put(center_x + x, center_y + y, lens);
    const HBITMAP mask = CreateBitmap(size, size, 1, 1, nullptr);
    ICONINFO info{TRUE, 0, 0, mask, color};
    const HICON icon = CreateIconIndirect(&info);
    DeleteObject(mask);
    DeleteObject(color);
    return icon;
}

} // namespace asc::win
