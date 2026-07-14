#include "preview_window.hpp"

#include <cstring>

namespace asc::win {
namespace { constexpr UINT preview_timer = 1; }

PreviewWindow::PreviewWindow(const HINSTANCE instance, App& app) : instance_(instance), app_(app) {
    WNDCLASSEXW wc{sizeof(wc)};
    wc.hInstance = instance_; wc.lpfnWndProc = procedure; wc.lpszClassName = L"AutomaticScreenCameraPreviews";
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW); wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    icon_large_ = create_app_icon(32); icon_small_ = create_app_icon(16);
    wc.hIcon = icon_large_; wc.hIconSm = icon_small_;
    RegisterClassExW(&wc);
    constexpr DWORD window_style = WS_OVERLAPPEDWINDOW;
    constexpr DWORD window_ex_style = WS_EX_TOOLWINDOW;
    const UINT system_dpi = GetDpiForSystem();
    const UINT dpi = system_dpi == 0 ? 96U : system_dpi;
    RECT initial_bounds{0, 0, MulDiv(620, static_cast<int>(dpi), 96),
                        MulDiv(460, static_cast<int>(dpi), 96)};
    if (!AdjustWindowRectExForDpi(&initial_bounds, window_style, FALSE, window_ex_style, dpi))
        AdjustWindowRectEx(&initial_bounds, window_style, FALSE, window_ex_style);
    window_ = CreateWindowExW(window_ex_style, wc.lpszClassName, L"Automatic Screen Camera — Previews",
                              window_style, CW_USEDEFAULT, CW_USEDEFAULT,
                              initial_bounds.right - initial_bounds.left, initial_bounds.bottom - initial_bounds.top,
                              nullptr, nullptr, instance_, this);
    if (!window_) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "Create preview window");
}

PreviewWindow::~PreviewWindow() {
    if (window_) DestroyWindow(window_);
    for (const auto bitmap : bitmaps_) if (bitmap) DeleteObject(bitmap);
    if (icon_large_) DestroyIcon(icon_large_);
    if (icon_small_) DestroyIcon(icon_small_);
}

LRESULT CALLBACK PreviewWindow::procedure(const HWND window, const UINT message, const WPARAM wparam, const LPARAM lparam) {
    auto* self = reinterpret_cast<PreviewWindow*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
        self = static_cast<PreviewWindow*>(reinterpret_cast<CREATESTRUCTW*>(lparam)->lpCreateParams);
        self->window_ = window; SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    }
    return self ? self->handle(message, wparam, lparam) : DefWindowProcW(window, message, wparam, lparam);
}

LRESULT PreviewWindow::handle(const UINT message, const WPARAM wparam, const LPARAM lparam) {
    switch (message) {
    case WM_CREATE: {
        constexpr const wchar_t* labels[]{L"Webcam / video input", L"Tracked monitor", L"Final virtual camera output", L"Saved reference (detection only)"};
        for (std::size_t i = 0; i < 4; ++i) {
            labels_[i] = CreateWindowExW(0, L"STATIC", labels[i], WS_CHILD | WS_VISIBLE,
                                         0, 0, 0, 0, window_, nullptr, instance_, nullptr);
            image_controls_[i] = CreateWindowExW(WS_EX_CLIENTEDGE, L"STATIC", nullptr,
                                                  WS_CHILD | WS_VISIBLE | SS_BITMAP | SS_CENTERIMAGE,
                                                  0, 0, 0, 0, window_, nullptr, instance_, nullptr);
        }
        fonts_ = std::make_unique<UiFonts>(window_);
        set_children_font(window_, fonts_->body());
        layout_controls();
        return 0;
    }
    case WM_TIMER: refresh(); return 0;
    case WM_SIZE:
        layout_controls();
        if (wparam == SIZE_MINIMIZED) KillTimer(window_, preview_timer);
        else if (IsWindowVisible(window_)) SetTimer(window_, preview_timer, 1000, nullptr);
        return 0;
    case WM_GETMINMAXINFO: {
        auto* bounds = reinterpret_cast<MINMAXINFO*>(lparam);
        bounds->ptMinTrackSize.x = dip(window_, 530);
        bounds->ptMinTrackSize.y = dip(window_, 380);
        return 0;
    }
    case WM_DPICHANGED: {
        const auto* suggested = reinterpret_cast<const RECT*>(lparam);
        if (suggested) SetWindowPos(window_, nullptr, suggested->left, suggested->top,
                                    suggested->right - suggested->left, suggested->bottom - suggested->top,
                                    SWP_NOACTIVATE | SWP_NOZORDER);
        if (fonts_) { fonts_->recreate(window_); set_children_font(window_, fonts_->body()); }
        layout_controls();
        return 0;
    }
    case WM_CLOSE: KillTimer(window_, preview_timer); ShowWindow(window_, SW_HIDE); return 0;
    default: return DefWindowProcW(window_, message, wparam, lparam);
    }
}

void PreviewWindow::layout_controls() {
    if (!labels_[0]) return;
    RECT client{};
    GetClientRect(window_, &client);
    const int pad = dip(window_, 14);
    const int gap = dip(window_, 16);
    const int label_height = dip(window_, 24);
    const int cell_width = std::max(1, (static_cast<int>(client.right) - pad * 2 - gap) / 2);
    const int cell_height = std::max(1, (static_cast<int>(client.bottom) - pad * 2 - gap) / 2);
    for (std::size_t index = 0; index < labels_.size(); ++index) {
        const int column = static_cast<int>(index % 2);
        const int row = static_cast<int>(index / 2);
        const int x = pad + column * (cell_width + gap);
        const int y = pad + row * (cell_height + gap);
        MoveWindow(labels_[index], x, y, cell_width, label_height, TRUE);
        MoveWindow(image_controls_[index], x, y + label_height, cell_width,
                   std::max(1, cell_height - label_height), TRUE);
    }
}

HBITMAP PreviewWindow::create_bitmap(const PreviewImage& image) {
    if (image.bgra.empty()) return nullptr;
    BITMAPINFO info{};
    info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = static_cast<LONG>(image.size.width);
    info.bmiHeader.biHeight = -static_cast<LONG>(image.size.height);
    info.bmiHeader.biPlanes = 1; info.bmiHeader.biBitCount = 32; info.bmiHeader.biCompression = BI_RGB;
    void* pixels = nullptr;
    HDC dc = GetDC(nullptr);
    const auto bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &pixels, nullptr, 0);
    ReleaseDC(nullptr, dc);
    if (bitmap && pixels) std::memcpy(pixels, image.bgra.data(), image.bgra.size());
    return bitmap;
}

void PreviewWindow::update_preview(const std::size_t index, const PreviewKind kind) {
    HBITMAP bitmap = nullptr;
    RECT bounds{}; GetClientRect(image_controls_[index], &bounds);
    const Size target{static_cast<std::uint32_t>(std::max(1L, bounds.right)),
                      static_cast<std::uint32_t>(std::max(1L, bounds.bottom))};
    try { if (const auto image = app_.preview(kind, target)) bitmap = create_bitmap(*image); }
    catch (...) {}
    const auto old = reinterpret_cast<HBITMAP>(SendMessageW(image_controls_[index], STM_SETIMAGE, IMAGE_BITMAP, reinterpret_cast<LPARAM>(bitmap)));
    if (old) DeleteObject(old);
    bitmaps_[index] = bitmap;
}

void PreviewWindow::refresh() {
    update_preview(0, PreviewKind::camera);
    update_preview(1, PreviewKind::screen);
    update_preview(2, PreviewKind::output);
    update_preview(3, PreviewKind::reference);
}

void PreviewWindow::show() {
    ShowWindow(window_, SW_SHOW); SetForegroundWindow(window_);
    refresh(); SetTimer(window_, preview_timer, 1000, nullptr);
}

} // namespace asc::win
