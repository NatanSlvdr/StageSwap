#include "deployment.hpp"

#include "common.hpp"
#include "media_source/ids.hpp"

#ifdef ASC_PORTABLE_BUILD
#include "portable_payload.hpp"
#endif

#include <bcrypt.h>
#include <knownfolders.h>
#include <shellapi.h>
#include <shlobj.h>

#include <algorithm>
#include <array>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <cwctype>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

namespace asc::win::deployment {
namespace {
constexpr wchar_t deployment_key_path[] = L"SOFTWARE\\AutomaticScreenCamera\\Deployment";
constexpr wchar_t portable_directory_name[] = L"Automatic Screen Camera Portable";
constexpr wchar_t source_file_name[] = L"AutomaticScreenCameraSource.dll";
#ifdef ASC_PORTABLE_BUILD
constexpr int portable_source_resource = 201;

struct RegistryKey {
    HKEY value{nullptr};
    ~RegistryKey() { if (value) RegCloseKey(value); }
    RegistryKey(const RegistryKey&) = delete;
    RegistryKey& operator=(const RegistryKey&) = delete;
    RegistryKey() = default;
};

struct AlgorithmHandle {
    BCRYPT_ALG_HANDLE value{nullptr};
    ~AlgorithmHandle() { if (value) BCryptCloseAlgorithmProvider(value, 0); }
};

struct HashHandle {
    BCRYPT_HASH_HANDLE value{nullptr};
    ~HashHandle() { if (value) BCryptDestroyHash(value); }
};
#endif

[[noreturn]] void throw_last_error(const std::string& operation) {
    throw HResultError(HRESULT_FROM_WIN32(GetLastError()), operation);
}

std::wstring registry_string(const HKEY root, const wchar_t* key_path, const wchar_t* value_name) {
    DWORD bytes = 0;
    const auto flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY;
    auto status = RegGetValueW(root, key_path, value_name, flags, nullptr, nullptr, &bytes);
    if (status == ERROR_FILE_NOT_FOUND) return {};
    if (status != ERROR_SUCCESS) throw HResultError(HRESULT_FROM_WIN32(status), "Read deployment registry value");
    std::wstring result(bytes / sizeof(wchar_t), L'\0');
    status = RegGetValueW(root, key_path, value_name, flags, nullptr, result.data(), &bytes);
    if (status != ERROR_SUCCESS) throw HResultError(HRESULT_FROM_WIN32(status), "Read deployment registry value");
    if (!result.empty() && result.back() == L'\0') result.pop_back();
    return result;
}

#ifdef ASC_PORTABLE_BUILD
void set_registry_string(const RegistryKey& key, const wchar_t* name, const std::wstring_view value) {
    const std::wstring terminated_value(value);
    const auto bytes = static_cast<DWORD>((terminated_value.size() + 1) * sizeof(wchar_t));
    const auto status = RegSetValueExW(key.value, name, 0, REG_SZ,
        reinterpret_cast<const BYTE*>(terminated_value.c_str()), bytes);
    if (status != ERROR_SUCCESS) throw HResultError(HRESULT_FROM_WIN32(status), "Write deployment registry value");
}

void write_portable_marker(const std::filesystem::path& source_path) {
    RegistryKey key;
    const auto status = RegCreateKeyExW(HKEY_LOCAL_MACHINE, deployment_key_path, 0, nullptr, 0,
        KEY_SET_VALUE | KEY_WOW64_64KEY, nullptr, &key.value, nullptr);
    if (status != ERROR_SUCCESS) throw HResultError(HRESULT_FROM_WIN32(status), "Create deployment registry key");
    set_registry_string(key, L"Mode", L"portable");
    set_registry_string(key, L"Version", ASC_RELEASE_VERSION);
    set_registry_string(key, L"SourcePath", source_path.native());
}
#endif

void delete_portable_marker() {
    const auto status = RegDeleteTreeW(HKEY_LOCAL_MACHINE, deployment_key_path);
    if (status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND) {
        throw HResultError(HRESULT_FROM_WIN32(status), "Delete deployment registry key");
    }
}

std::filesystem::path program_files_path() {
    PWSTR raw_path = nullptr;
    check_hresult(SHGetKnownFolderPath(FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, nullptr, &raw_path),
                  "Locate Program Files");
    const std::filesystem::path result(raw_path);
    CoTaskMemFree(raw_path);
    return result;
}

std::filesystem::path portable_directory() {
    return program_files_path() / portable_directory_name;
}

std::filesystem::path portable_source_path() {
    return portable_directory() / source_file_name;
}

std::filesystem::path current_executable_path() {
    std::wstring buffer(512, L'\0');
    for (;;) {
        const DWORD length = GetModuleFileNameW(nullptr, buffer.data(), static_cast<DWORD>(buffer.size()));
        if (length == 0) throw_last_error("Locate current executable");
        if (length < buffer.size() - 1) {
            buffer.resize(length);
            return buffer;
        }
        buffer.resize(buffer.size() * 2);
    }
}

bool is_process_elevated() {
    HANDLE token = nullptr;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) throw_last_error("Open process token");
    TOKEN_ELEVATION elevation{};
    DWORD bytes = 0;
    const BOOL result = GetTokenInformation(token, TokenElevation, &elevation, sizeof(elevation), &bytes);
    const DWORD error = result ? ERROR_SUCCESS : GetLastError();
    CloseHandle(token);
    if (!result) throw HResultError(HRESULT_FROM_WIN32(error), "Read process elevation");
    return elevation.TokenIsElevated != 0;
}

void require_elevation() {
    if (!is_process_elevated()) throw std::runtime_error("This deployment operation requires administrator permission");
}

void run_elevated(const wchar_t* arguments) {
    const auto executable = current_executable_path();
    SHELLEXECUTEINFOW info{sizeof(info)};
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = L"runas";
    info.lpFile = executable.c_str();
    info.lpParameters = arguments;
    info.nShow = SW_HIDE;
    if (!ShellExecuteExW(&info)) throw_last_error("Request administrator permission");
    if (!info.hProcess) throw std::runtime_error("Administrator helper did not start");
    const DWORD wait_result = WaitForSingleObject(info.hProcess, INFINITE);
    if (wait_result != WAIT_OBJECT_0) {
        const DWORD error = wait_result == WAIT_FAILED ? GetLastError() : ERROR_GEN_FAILURE;
        CloseHandle(info.hProcess);
        throw HResultError(HRESULT_FROM_WIN32(error), "Wait for administrator helper");
    }
    DWORD exit_code = 1;
    if (!GetExitCodeProcess(info.hProcess, &exit_code)) {
        const DWORD error = GetLastError();
        CloseHandle(info.hProcess);
        throw HResultError(HRESULT_FROM_WIN32(error), "Read administrator helper result");
    }
    CloseHandle(info.hProcess);
    if (exit_code != 0) throw std::runtime_error("Administrator helper failed with exit code " + std::to_string(exit_code));
}

#ifdef ASC_PORTABLE_BUILD
std::string bytes_to_hex(const std::vector<std::uint8_t>& bytes) {
    constexpr char digits[] = "0123456789abcdef";
    std::string result(bytes.size() * 2, '0');
    for (std::size_t i = 0; i < bytes.size(); ++i) {
        result[i * 2] = digits[bytes[i] >> 4];
        result[i * 2 + 1] = digits[bytes[i] & 0x0f];
    }
    return result;
}

class Sha256 {
public:
    Sha256() {
        check_hresult(BCryptOpenAlgorithmProvider(&algorithm_.value, BCRYPT_SHA256_ALGORITHM, nullptr, 0),
                      "Open SHA-256 provider");
        DWORD result_bytes = 0;
        check_hresult(BCryptGetProperty(algorithm_.value, BCRYPT_OBJECT_LENGTH,
            reinterpret_cast<PUCHAR>(&object_size_), sizeof(object_size_), &result_bytes, 0), "Read SHA-256 object size");
        check_hresult(BCryptGetProperty(algorithm_.value, BCRYPT_HASH_LENGTH,
            reinterpret_cast<PUCHAR>(&hash_size_), sizeof(hash_size_), &result_bytes, 0), "Read SHA-256 digest size");
        object_.resize(object_size_);
        check_hresult(BCryptCreateHash(algorithm_.value, &hash_.value, object_.data(), object_size_, nullptr, 0, 0),
                      "Create SHA-256 hash");
    }

    void update(const void* data, const std::size_t size) {
        if (size > std::numeric_limits<ULONG>::max()) throw std::runtime_error("SHA-256 input is too large");
        check_hresult(BCryptHashData(hash_.value,
            const_cast<PUCHAR>(static_cast<const UCHAR*>(data)), static_cast<ULONG>(size), 0), "Update SHA-256 hash");
    }

    std::string finish() {
        std::vector<std::uint8_t> digest(hash_size_);
        check_hresult(BCryptFinishHash(hash_.value, digest.data(), hash_size_, 0), "Finish SHA-256 hash");
        return bytes_to_hex(digest);
    }

private:
    AlgorithmHandle algorithm_;
    HashHandle hash_;
    DWORD object_size_{0};
    DWORD hash_size_{0};
    std::vector<UCHAR> object_;
};

std::string hash_memory(const void* data, const std::size_t size) {
    Sha256 hash;
    hash.update(data, size);
    return hash.finish();
}

std::string hash_file(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    if (!stream) throw std::runtime_error("Could not open deployment payload for hashing: " + path.string());
    Sha256 hash;
    std::array<char, 64 * 1024> buffer{};
    while (stream) {
        stream.read(buffer.data(), static_cast<std::streamsize>(buffer.size()));
        const auto count = stream.gcount();
        if (count > 0) hash.update(buffer.data(), static_cast<std::size_t>(count));
    }
    if (!stream.eof()) throw std::runtime_error("Could not read deployment payload for hashing: " + path.string());
    return hash.finish();
}

struct PayloadView {
    const void* data{nullptr};
    std::size_t size{0};
};

PayloadView portable_payload(const HINSTANCE instance) {
    const HRSRC resource = FindResourceW(instance, MAKEINTRESOURCEW(portable_source_resource), RT_RCDATA);
    if (!resource) throw_last_error("Locate embedded camera source");
    const HGLOBAL loaded = LoadResource(instance, resource);
    if (!loaded) throw_last_error("Load embedded camera source");
    const DWORD size = SizeofResource(instance, resource);
    const void* data = LockResource(loaded);
    if (!data || size == 0) throw std::runtime_error("Embedded camera source is empty");
    return {data, size};
}

void validate_payload(const HINSTANCE instance) {
    const auto payload = portable_payload(instance);
    if (hash_memory(payload.data, payload.size) != ASC_PORTABLE_SOURCE_SHA256) {
        throw std::runtime_error("Embedded camera source failed SHA-256 verification");
    }
}
#endif

using RegistrationFunction = HRESULT(WINAPI*)();

void invoke_registration(const std::filesystem::path& path, const char* export_name, const std::string& operation) {
    const HMODULE module = LoadLibraryExW(path.c_str(), nullptr, LOAD_WITH_ALTERED_SEARCH_PATH);
    if (!module) throw_last_error("Load camera source for " + operation);
    const auto function = reinterpret_cast<RegistrationFunction>(GetProcAddress(module, export_name));
    if (!function) {
        const DWORD error = GetLastError();
        FreeLibrary(module);
        throw HResultError(HRESULT_FROM_WIN32(error), "Find camera source " + operation + " export");
    }
    const HRESULT result = function();
    FreeLibrary(module);
    check_hresult(result, operation);
}

std::wstring registered_source_path() {
    const std::wstring key = std::wstring(L"Software\\Classes\\CLSID\\") +
        CLSID_AutomaticScreenCameraSourceText + L"\\InprocServer32";
    return registry_string(HKEY_LOCAL_MACHINE, key.c_str(), nullptr);
}

void delete_source_registration() {
    const std::wstring key = std::wstring(L"Software\\Classes\\CLSID\\") +
        CLSID_AutomaticScreenCameraSourceText;
    const auto status = RegDeleteTreeW(HKEY_LOCAL_MACHINE, key.c_str());
    if (status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND) {
        throw HResultError(HRESULT_FROM_WIN32(status), "Delete stale portable camera-source registration");
    }
}

bool equal_paths(const std::filesystem::path& left, const std::filesystem::path& right) {
    const auto normalize = [](const std::filesystem::path& value) {
        std::error_code error;
        auto result = std::filesystem::weakly_canonical(value, error).native();
        if (error) result = std::filesystem::absolute(value, error).native();
        std::transform(result.begin(), result.end(), result.begin(), [](const wchar_t c) {
            return static_cast<wchar_t>(std::towlower(c));
        });
        return result;
    };
    return normalize(left) == normalize(right);
}

#ifdef ASC_PORTABLE_BUILD
void write_payload_file(const HINSTANCE instance, const std::filesystem::path& destination) {
    const auto payload = portable_payload(instance);
    std::ofstream stream(destination, std::ios::binary | std::ios::trunc);
    if (!stream) throw std::runtime_error("Could not create staged camera source: " + destination.string());
    stream.write(static_cast<const char*>(payload.data), static_cast<std::streamsize>(payload.size));
    stream.close();
    if (!stream) throw std::runtime_error("Could not write staged camera source: " + destination.string());
}

void remove_file_if_present(const std::filesystem::path& path) noexcept {
    std::error_code ignored;
    std::filesystem::remove(path, ignored);
}
#endif

} // namespace

Mode current_mode() {
    const auto value = registry_string(HKEY_LOCAL_MACHINE, deployment_key_path, L"Mode");
    if (value.empty()) return Mode::none;
    if (_wcsicmp(value.c_str(), L"portable") == 0) return Mode::portable;
    if (_wcsicmp(value.c_str(), L"installed") == 0) return Mode::installed;
    return Mode::unknown;
}

bool is_portable_build() noexcept {
#ifdef ASC_PORTABLE_BUILD
    return true;
#else
    return false;
#endif
}

void stop_application_for_deployment() {
    const HANDLE mutex = OpenMutexW(SYNCHRONIZE, FALSE, legacy_application_mutex_name);
    if (!mutex) {
        if (GetLastError() == ERROR_FILE_NOT_FOUND) return;
        throw_last_error("Check whether Automatic Screen Camera is running");
    }
    CloseHandle(mutex);
    const HWND window = FindWindowW(application_window_class, nullptr);
    if (!window || !PostMessageW(window, exit_for_deployment_message, 0, 0)) {
        throw std::runtime_error("Automatic Screen Camera is running but could not be asked to exit");
    }
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(15);
    while (std::chrono::steady_clock::now() < deadline) {
        const HANDLE remaining = OpenMutexW(SYNCHRONIZE, FALSE, legacy_application_mutex_name);
        if (!remaining && GetLastError() == ERROR_FILE_NOT_FOUND) return;
        if (remaining) CloseHandle(remaining);
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
    }
    throw std::runtime_error("Automatic Screen Camera did not exit within 15 seconds");
}

void remove_current_user_startup_entry() {
    constexpr wchar_t run_key[] = L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const auto status = RegDeleteKeyValueW(HKEY_CURRENT_USER, run_key, L"AutomaticScreenCamera");
    if (status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND && status != ERROR_PATH_NOT_FOUND) {
        throw HResultError(HRESULT_FROM_WIN32(status), "Remove Automatic Screen Camera startup entry");
    }
}

void verify_portable_payload(const HINSTANCE instance) {
#ifdef ASC_PORTABLE_BUILD
    validate_payload(instance);
#else
    (void)instance;
    throw std::runtime_error("This is not the portable build");
#endif
}

void install_portable_source(const HINSTANCE instance) {
    require_elevation();
#ifdef ASC_PORTABLE_BUILD
    const auto mode = current_mode();
    if (mode == Mode::installed) {
        throw std::runtime_error("The installed edition is present. Uninstall it before using the portable edition");
    }
    if (mode == Mode::unknown) throw std::runtime_error("Deployment registry data is not recognized");
    const auto existing_registration = registered_source_path();
    if (mode == Mode::none && !existing_registration.empty() &&
        !equal_paths(existing_registration, portable_source_path())) {
        throw std::runtime_error("A camera source is already registered outside the portable deployment directory");
    }
    validate_payload(instance);
    const auto directory = portable_directory();
    const auto source = portable_source_path();
    const auto staging = directory / (std::wstring(source_file_name) + L".installing-" + std::to_wstring(GetCurrentProcessId()));
    const auto backup = directory / (std::wstring(source_file_name) + L".backup-" + std::to_wstring(GetCurrentProcessId()));
    std::filesystem::create_directories(directory);
    remove_file_if_present(staging);
    remove_file_if_present(backup);
    write_payload_file(instance, staging);
    if (hash_file(staging) != ASC_PORTABLE_SOURCE_SHA256) {
        remove_file_if_present(staging);
        throw std::runtime_error("Staged camera source failed SHA-256 verification");
    }

    const bool had_source = std::filesystem::is_regular_file(source);
    bool old_unregistered = false;
    bool old_moved = false;
    bool new_moved = false;
    bool new_registered = false;
    try {
        if (had_source && equal_paths(registered_source_path(), source)) {
            invoke_registration(source, "DllUnregisterServer", "Unregister previous portable camera source");
            old_unregistered = true;
        }
        if (had_source) {
            if (!MoveFileExW(source.c_str(), backup.c_str(), MOVEFILE_WRITE_THROUGH)) throw_last_error("Stage previous portable camera source");
            old_moved = true;
        }
        if (!MoveFileExW(staging.c_str(), source.c_str(), MOVEFILE_WRITE_THROUGH)) throw_last_error("Install portable camera source");
        new_moved = true;
        invoke_registration(source, "DllRegisterServer", "Register portable camera source");
        new_registered = true;
        write_portable_marker(source);
        remove_file_if_present(backup);
    } catch (...) {
        if (new_registered) {
            try { invoke_registration(source, "DllUnregisterServer", "Roll back portable camera source registration"); }
            catch (...) {}
        }
        if (new_moved) remove_file_if_present(source);
        if (old_moved) MoveFileExW(backup.c_str(), source.c_str(), MOVEFILE_WRITE_THROUGH);
        if (old_unregistered && std::filesystem::is_regular_file(source)) {
            try { invoke_registration(source, "DllRegisterServer", "Restore previous portable camera source"); }
            catch (...) {}
        }
        remove_file_if_present(staging);
        throw;
    }
#else
    (void)instance;
    throw std::runtime_error("This executable does not contain the portable camera source");
#endif
}

void ensure_portable_source(const HINSTANCE instance, const HANDLE owned_application_mutex) {
    if (!owned_application_mutex) {
        throw std::invalid_argument("Portable deployment requires the caller to own the application mutex");
    }
#ifdef ASC_PORTABLE_BUILD
    const auto mode = current_mode();
    if (mode == Mode::installed) {
        throw std::runtime_error("The installed edition is present. Launch it from the Start menu or uninstall it before using the portable edition");
    }
    if (mode == Mode::unknown) throw std::runtime_error("Deployment registry data is not recognized");
    validate_payload(instance);
    const auto source = portable_source_path();
    const auto registered_source = registered_source_path();
    if (mode == Mode::none && !registered_source.empty() && !equal_paths(registered_source, source)) {
        throw std::runtime_error("A camera source is already registered by a developer or installed build; remove it before using the portable edition");
    }
    const auto marked_source = registry_string(HKEY_LOCAL_MACHINE, deployment_key_path, L"SourcePath");
    const bool ready = mode == Mode::portable && std::filesystem::is_regular_file(source) &&
        !marked_source.empty() && equal_paths(marked_source, source) &&
        equal_paths(registered_source, source) && hash_file(source) == ASC_PORTABLE_SOURCE_SHA256;
    if (!ready) {
        run_elevated(L"--portable-register-under-lock");
    }
#else
    (void)instance;
    (void)owned_application_mutex;
#endif
}

void remove_portable_source() {
    require_elevation();
    const auto mode = current_mode();
    if (mode == Mode::installed) throw std::runtime_error("Refusing to remove the installed edition as portable data");
    if (mode == Mode::unknown) throw std::runtime_error("Deployment registry data is not recognized");
    const auto source = portable_source_path();
    const auto registered_source = registered_source_path();
    if (std::filesystem::is_regular_file(source)) {
        if (equal_paths(registered_source, source)) {
            invoke_registration(source, "DllUnregisterServer", "Unregister portable camera source");
        }
        if (!DeleteFileW(source.c_str()) && GetLastError() != ERROR_FILE_NOT_FOUND) {
            throw_last_error("Delete portable camera source");
        }
    } else if (!registered_source.empty() && equal_paths(registered_source, source)) {
        delete_source_registration();
    }
    delete_portable_marker();
    std::error_code ignored;
    std::filesystem::remove_all(portable_directory(), ignored);
}

void remove_portable_source_elevated() {
    if (is_process_elevated()) remove_portable_source();
    else run_elevated(L"--portable-unregister-under-lock");
}

} // namespace asc::win::deployment
