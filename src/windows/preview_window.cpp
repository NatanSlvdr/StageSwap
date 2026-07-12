#include "preview_window.hpp"

#include <cstring>

namespace asc::win {
namespace { constexpr UINT preview_timer = 1; }

PreviewWindow::PreviewWindow(const HINSTANCE instance, App& app) : instance_(instance), app_(app) {
    WNDCLASSEXW wc{sizeof(wc)};
    wc.hInstance = instance_; wc.lpfnWndProc = procedure; wc.lpszClassName = L"AutomaticScreenCameraPreviews";
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW); wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    RegisterClassExW(&wc);
    window_ = CreateWindowExW(WS_EX_TOOLWINDOW, wc.lpszClassName, L"Automatic Screen Camera — Previews",
                              WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                              CW_USEDEFAULT, CW_USEDEFAULT, 530, 380, nullptr, nullptr, instance_, this);
    if (!window_) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "Create preview window");
}

PreviewWindow::~PreviewWindow() {
    if (window_) DestroyWindow(window_);
    for (const auto bitmap : bitmaps_) if (bitmap) DeleteObject(bitmap);
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
            const int column = static_cast<int>(i % 2); const int row = static_cast<int>(i / 2);
            const int x = 12 + column * 254; const int y = 10 + row * 168;
            CreateWindowExW(0, L"STATIC", labels[i], WS_CHILD | WS_VISIBLE, x, y, 240, 22, window_, nullptr, instance_, nullptr);
            image_controls_[i] = CreateWindowExW(WS_EX_CLIENTEDGE, L"STATIC", nullptr,
                                                  WS_CHILD | WS_VISIBLE | SS_BITMAP | SS_CENTERIMAGE,
                                                  x, y + 24, 240, 135, window_, nullptr, instance_, nullptr);
        }
        return 0;
    }
    case WM_TIMER: refresh(); return 0;
    case WM_SIZE:
        if (wparam == SIZE_MINIMIZED) KillTimer(window_, preview_timer);
        else if (IsWindowVisible(window_)) SetTimer(window_, preview_timer, 1000, nullptr);
        return 0;
    case WM_CLOSE: KillTimer(window_, preview_timer); ShowWindow(window_, SW_HIDE); return 0;
    default: return DefWindowProcW(window_, message, wparam, lparam);
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
    try { if (const auto image = app_.preview(kind)) bitmap = create_bitmap(*image); }
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
