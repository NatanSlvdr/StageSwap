#include <windows.h>

#include <algorithm>
#include <charconv>
#include <chrono>
#include <cstdint>
#include <iostream>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace {

enum class FixtureMode { reference, removed, duplicate, toggle };

struct Options {
    FixtureMode mode{FixtureMode::reference};
    unsigned monitor{0};
    unsigned toggle_seconds{60};
    unsigned duration_seconds{0};
};

struct FixtureWindow {
    HWND hwnd{};
    bool reference_visible{};
};

std::vector<FixtureWindow> windows;
UINT_PTR toggle_timer{};
UINT_PTR exit_timer{};
UINT_PTR animation_timer{};
unsigned animation_phase{};

std::optional<unsigned> parse_unsigned(const std::wstring_view value) {
    if (value.empty()) return std::nullopt;
    unsigned result{};
    std::string narrow;
    narrow.reserve(value.size());
    for (const wchar_t character : value) {
        if (character < L'0' || character > L'9') return std::nullopt;
        narrow.push_back(static_cast<char>(character));
    }
    const auto parsed = std::from_chars(narrow.data(), narrow.data() + narrow.size(), result);
    if (parsed.ec != std::errc{} || parsed.ptr != narrow.data() + narrow.size()) return std::nullopt;
    return result;
}

bool parse_options(const int argc, wchar_t** argv, Options& result) {
    for (int index = 1; index < argc; ++index) {
        const std::wstring_view argument{argv[index]};
        const auto require_value = [&](std::wstring_view& value) {
            if (index + 1 >= argc) return false;
            value = argv[++index];
            return true;
        };

        std::wstring_view value;
        if (argument == L"--mode") {
            if (!require_value(value)) return false;
            if (value == L"reference") result.mode = FixtureMode::reference;
            else if (value == L"removed") result.mode = FixtureMode::removed;
            else if (value == L"duplicate") result.mode = FixtureMode::duplicate;
            else if (value == L"toggle") result.mode = FixtureMode::toggle;
            else return false;
        } else if (argument == L"--monitor") {
            if (!require_value(value)) return false;
            const auto parsed = parse_unsigned(value);
            if (!parsed) return false;
            result.monitor = *parsed;
        } else if (argument == L"--toggle-seconds") {
            if (!require_value(value)) return false;
            const auto parsed = parse_unsigned(value);
            if (!parsed || *parsed == 0 || *parsed > 86'400) return false;
            result.toggle_seconds = *parsed;
        } else if (argument == L"--duration-seconds") {
            if (!require_value(value)) return false;
            const auto parsed = parse_unsigned(value);
            if (!parsed || *parsed > 604'800) return false;
            result.duration_seconds = *parsed;
        } else if (argument == L"--help" || argument == L"-h") {
            std::wcout << L"Usage: asc_test_screen_fixture [--mode reference|removed|duplicate|toggle] "
                          L"[--monitor N] [--toggle-seconds N] [--duration-seconds N]\n";
            std::exit(0);
        } else {
            return false;
        }
    }
    return true;
}

BOOL CALLBACK collect_monitor(HMONITOR, HDC, LPRECT rect, LPARAM context) {
    auto& monitors = *reinterpret_cast<std::vector<RECT>*>(context);
    monitors.push_back(*rect);
    return TRUE;
}

void paint_reference(HDC dc, const RECT& client) {
    const int width = client.right - client.left;
    const int height = client.bottom - client.top;
    const HBRUSH background = CreateSolidBrush(RGB(9, 19, 38));
    if (background == nullptr) return;
    FillRect(dc, &client, background);
    DeleteObject(background);

    constexpr COLORREF colors[]{
        RGB(239, 68, 68), RGB(245, 158, 11), RGB(34, 197, 94),
        RGB(6, 182, 212), RGB(59, 130, 246), RGB(168, 85, 247),
    };
    const int band_height = std::max(1, height / static_cast<int>(std::size(colors)));
    for (std::size_t index = 0; index < std::size(colors); ++index) {
        RECT band{width / 8, static_cast<int>(index) * band_height,
                  width * 7 / 8, std::min(height, static_cast<int>(index + 1) * band_height)};
        const HBRUSH brush = CreateSolidBrush(colors[index]);
        if (brush != nullptr) {
            FillRect(dc, &band, brush);
            DeleteObject(brush);
        }
    }

    const HBRUSH white = CreateSolidBrush(RGB(255, 255, 255));
    const HBRUSH black = CreateSolidBrush(RGB(0, 0, 0));
    if (white == nullptr || black == nullptr) {
        if (white != nullptr) DeleteObject(white);
        if (black != nullptr) DeleteObject(black);
        return;
    }
    const int cell = std::max(8, std::min(width, height) / 24);
    for (int y = height / 4; y < height * 3 / 4; y += cell) {
        for (int x = width / 4; x < width * 3 / 4; x += cell) {
            RECT square{x, y, std::min(x + cell, width), std::min(y + cell, height)};
            FillRect(dc, &square, ((x / cell + y / cell) & 1) == 0 ? white : black);
        }
    }
    DeleteObject(white);
    DeleteObject(black);
}

void paint_removed(HDC dc, const RECT& client) {
    const HBRUSH background = CreateSolidBrush(RGB(18, 18, 18));
    if (background != nullptr) {
        FillRect(dc, &client, background);
        DeleteObject(background);
    }
    const int width = client.right - client.left;
    const int height = client.bottom - client.top;
    const int marker_size = std::max(24, std::min(width, height) / 12);
    const unsigned horizontal_range = static_cast<unsigned>(std::max(1, width - marker_size));
    const unsigned vertical_range = static_cast<unsigned>(std::max(1, height - marker_size));
    const int left = static_cast<int>((static_cast<std::uint64_t>(animation_phase) * 13U) % horizontal_range);
    const int top = static_cast<int>((static_cast<std::uint64_t>(animation_phase) * 7U) % vertical_range);
    RECT marker{left, top, left + marker_size, top + marker_size};
    const HBRUSH marker_brush = CreateSolidBrush(RGB(236, 72, 153));
    if (marker_brush != nullptr) {
        FillRect(dc, &marker, marker_brush);
        DeleteObject(marker_brush);
    }
}

LRESULT CALLBACK window_proc(HWND hwnd, const UINT message, WPARAM wparam, LPARAM lparam) {
    switch (message) {
    case WM_ERASEBKGND:
        return 1;
    case WM_PAINT: {
        PAINTSTRUCT paint{};
        const HDC dc = BeginPaint(hwnd, &paint);
        if (dc != nullptr) {
            RECT client{};
            GetClientRect(hwnd, &client);
            const auto found = std::find_if(windows.begin(), windows.end(),
                                            [hwnd](const FixtureWindow& window) { return window.hwnd == hwnd; });
            if (found != windows.end() && found->reference_visible) paint_reference(dc, client);
            else paint_removed(dc, client);
            EndPaint(hwnd, &paint);
        }
        return 0;
    }
    case WM_TIMER:
        if (wparam == toggle_timer) {
            for (auto& window : windows) {
                window.reference_visible = !window.reference_visible;
                InvalidateRect(window.hwnd, nullptr, FALSE);
            }
            std::wcout << (windows.empty() || !windows.front().reference_visible ? L"removed\n" : L"reference\n") << std::flush;
            return 0;
        }
        if (wparam == exit_timer) {
            PostQuitMessage(0);
            return 0;
        }
        if (wparam == animation_timer) {
            ++animation_phase;
            for (const auto& window : windows) {
                if (!window.reference_visible) InvalidateRect(window.hwnd, nullptr, FALSE);
            }
            return 0;
        }
        break;
    case WM_CLOSE:
        DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY:
        if (hwnd == (windows.empty() ? nullptr : windows.front().hwnd)) PostQuitMessage(0);
        return 0;
    default:
        break;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

} // namespace

int wmain(const int argc, wchar_t** argv) {
    Options options;
    if (!parse_options(argc, argv, options)) {
        std::wcerr << L"Invalid arguments. Run with --help for usage.\n";
        return 2;
    }

    std::vector<RECT> monitors;
    if (!EnumDisplayMonitors(nullptr, nullptr, collect_monitor, reinterpret_cast<LPARAM>(&monitors)) || monitors.empty()) {
        std::wcerr << L"No active display monitors were found.\n";
        return 3;
    }
    if (options.monitor >= monitors.size()) {
        std::wcerr << L"Monitor index is outside the active display range.\n";
        return 2;
    }

    const HINSTANCE instance = GetModuleHandleW(nullptr);
    if (instance == nullptr) return 4;
    const wchar_t class_name[] = L"AutomaticScreenCameraReliabilityFixture";
    WNDCLASSEXW window_class{static_cast<UINT>(sizeof(window_class))};
    window_class.hInstance = instance;
    window_class.lpfnWndProc = window_proc;
    window_class.lpszClassName = class_name;
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    if (RegisterClassExW(&window_class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        std::wcerr << L"Failed to register fixture window class.\n";
        return 4;
    }

    const auto create_for_monitor = [&](const RECT& rect, const bool visible) {
        const HWND hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW, class_name, L"ASC Reliability Fixture", WS_POPUP,
            rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top,
            nullptr, nullptr, instance, nullptr);
        if (hwnd == nullptr) return false;
        windows.push_back({hwnd, visible});
        ShowWindow(hwnd, SW_SHOWNA);
        UpdateWindow(hwnd);
        return true;
    };

    if (options.mode == FixtureMode::duplicate) {
        for (const auto& rect : monitors) {
            if (!create_for_monitor(rect, true)) return 4;
        }
    } else if (!create_for_monitor(monitors[options.monitor], options.mode != FixtureMode::removed)) {
        return 4;
    }

    const HWND timer_window = windows.front().hwnd;
    if (options.mode == FixtureMode::toggle) {
        toggle_timer = SetTimer(timer_window, 1, options.toggle_seconds * 1000U, nullptr);
        if (toggle_timer == 0) return 4;
    }
    if (options.duration_seconds != 0) {
        exit_timer = SetTimer(timer_window, 2, options.duration_seconds * 1000U, nullptr);
        if (exit_timer == 0) return 4;
    }
    if (options.mode == FixtureMode::removed || options.mode == FixtureMode::toggle) {
        animation_timer = SetTimer(timer_window, 3, 33, nullptr);
        if (animation_timer == 0) return 4;
    }

    std::wcout << L"ready mode="
               << (options.mode == FixtureMode::duplicate ? L"duplicate" :
                   options.mode == FixtureMode::removed ? L"removed" :
                   options.mode == FixtureMode::toggle ? L"toggle" : L"reference")
               << L" monitor_count=" << monitors.size() << L'\n' << std::flush;

    MSG message{};
    while (true) {
        const int result = GetMessageW(&message, nullptr, 0, 0);
        if (result == 0) break;
        if (result < 0) return 5;
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    return 0;
}
