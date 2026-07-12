#pragma once

#include <windows.h>
#include <combaseapi.h>
#include <wrl/client.h>
#include <cstdio>

#include <stdexcept>
#include <string>
#include <string_view>

namespace asc::win {

using Microsoft::WRL::ComPtr;

class HResultError final : public std::runtime_error {
public:
    HResultError(const HRESULT value, const std::string& operation)
        : std::runtime_error(operation + " failed with HRESULT 0x" + hex(value)), value_(value) {}
    [[nodiscard]] HRESULT value() const noexcept { return value_; }
private:
    static std::string hex(const HRESULT value) {
        char text[16]{};
        std::snprintf(text, sizeof(text), "%08lX", static_cast<unsigned long>(value));
        return text;
    }
    HRESULT value_;
};

inline void check_hresult(const HRESULT result, const std::string& operation) {
    if (FAILED(result)) throw HResultError(result, operation);
}

inline std::string utf8(const std::wstring_view value) {
    if (value.empty()) return {};
    const int count = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    if (count <= 0) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "WideCharToMultiByte");
    std::string result(static_cast<std::size_t>(count), '\0');
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), result.data(), count, nullptr, nullptr);
    return result;
}

inline std::wstring wide(const std::string_view value) {
    if (value.empty()) return {};
    const int count = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), nullptr, 0);
    if (count <= 0) throw HResultError(HRESULT_FROM_WIN32(GetLastError()), "MultiByteToWideChar");
    std::wstring result(static_cast<std::size_t>(count), L'\0');
    MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), result.data(), count);
    return result;
}

struct CoTaskMemString {
    wchar_t* value{nullptr};
    ~CoTaskMemString() { CoTaskMemFree(value); }
    CoTaskMemString(const CoTaskMemString&) = delete;
    CoTaskMemString& operator=(const CoTaskMemString&) = delete;
    CoTaskMemString() = default;
};

} // namespace asc::win
