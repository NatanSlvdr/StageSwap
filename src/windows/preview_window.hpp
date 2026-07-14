#pragma once

#include "common.hpp"
#include "app.hpp"
#include "ui_helpers.hpp"

#include <array>
#include <memory>

namespace asc::win {

class PreviewWindow {
public:
    PreviewWindow(HINSTANCE instance, App& app);
    ~PreviewWindow();
    void show();
private:
    static LRESULT CALLBACK procedure(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
    LRESULT handle(UINT message, WPARAM wparam, LPARAM lparam);
    void layout_controls();
    void refresh();
    void update_preview(std::size_t index, PreviewKind kind);
    [[nodiscard]] static HBITMAP create_bitmap(const PreviewImage& image);
    HINSTANCE instance_;
    App& app_;
    HWND window_{nullptr};
    std::array<HWND, 4> labels_{};
    std::array<HWND, 4> image_controls_{};
    std::array<HBITMAP, 4> bitmaps_{};
    std::unique_ptr<UiFonts> fonts_;
    HICON icon_large_{nullptr};
    HICON icon_small_{nullptr};
};

} // namespace asc::win
