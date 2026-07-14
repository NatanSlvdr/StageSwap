#pragma once

#include "common.hpp"

#include <commctrl.h>
#include <map>
#include <string>

namespace asc::win {

[[nodiscard]] UINT window_dpi(HWND window) noexcept;
[[nodiscard]] int dip(HWND window, int value) noexcept;
void set_control_font(HWND control, HFONT font);
void set_children_font(HWND window, HFONT font);

class UiFonts {
public:
    explicit UiFonts(HWND window);
    ~UiFonts();
    UiFonts(const UiFonts&) = delete;
    UiFonts& operator=(const UiFonts&) = delete;
    void recreate(HWND window);
    [[nodiscard]] HFONT body() const noexcept { return body_; }
    [[nodiscard]] HFONT title() const noexcept { return title_; }
    [[nodiscard]] HFONT section() const noexcept { return section_; }

private:
    void clear() noexcept;
    HFONT body_{nullptr};
    HFONT title_{nullptr};
    HFONT section_{nullptr};
};

class TooltipHost {
public:
    TooltipHost() = default;
    ~TooltipHost();
    TooltipHost(const TooltipHost&) = delete;
    TooltipHost& operator=(const TooltipHost&) = delete;
    void create(HWND owner, HINSTANCE instance);
    void set(HWND control, std::wstring text, bool focus_tracking = false);
    void show_for_focus(HWND control, bool show);

private:
    void update_tool(HWND tooltip, HWND control, const std::wstring& text, UINT flags);
    HWND owner_{nullptr};
    HWND hover_{nullptr};
    HWND focus_{nullptr};
    std::map<HWND, std::wstring> text_;
    std::map<HWND, bool> focus_tracking_;
};

[[nodiscard]] HICON create_app_icon(int pixels);

} // namespace asc::win
