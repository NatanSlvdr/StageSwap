#include "app.hpp"
#include "common.hpp"
#include "deployment.hpp"

#include <mfapi.h>
#include <shellapi.h>
#include <sddl.h>
#include <winrt/base.h>

#include <optional>
#include <stdexcept>
#include <string>

namespace {
constexpr wchar_t register_under_lock[] = L"--portable-register-under-lock";
constexpr wchar_t unregister_under_lock[] = L"--portable-unregister-under-lock";

class NamedMutexLock {
public:
    NamedMutexLock(const wchar_t* name, const bool initially_owned) {
        PSECURITY_DESCRIPTOR descriptor = nullptr;
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
                asc::win::deployment::shared_mutex_security_sddl, SDDL_REVISION_1, &descriptor, nullptr))
            asc::win::check_hresult(HRESULT_FROM_WIN32(GetLastError()), "Create mutex security descriptor");
        SECURITY_ATTRIBUTES security{sizeof(security), descriptor, FALSE};
        SetLastError(ERROR_SUCCESS);
        handle_ = CreateMutexExW(&security, name, initially_owned ? CREATE_MUTEX_INITIAL_OWNER : 0,
                                 SYNCHRONIZE | MUTEX_MODIFY_STATE);
        const auto status = GetLastError();
        LocalFree(descriptor);
        if (!handle_) asc::win::check_hresult(HRESULT_FROM_WIN32(status), "Create application mutex");
        already_exists_ = status == ERROR_ALREADY_EXISTS;
        owned_ = initially_owned && !already_exists_;
        if (!initially_owned) {
            const auto wait = WaitForSingleObject(handle_, 30000);
            if (wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED)
                throw std::runtime_error("Timed out waiting for portable deployment");
            owned_ = true;
        }
    }
    ~NamedMutexLock() { if (owned_) ReleaseMutex(handle_); if (handle_) CloseHandle(handle_); }
    NamedMutexLock(const NamedMutexLock&) = delete;
    NamedMutexLock& operator=(const NamedMutexLock&) = delete;
    [[nodiscard]] bool already_exists() const noexcept { return already_exists_; }
    [[nodiscard]] HANDLE get() const noexcept { return handle_; }
private:
    HANDLE handle_{nullptr};
    bool owned_{false};
    bool already_exists_{false};
};

std::optional<std::wstring> command_line_argument() {
    int count = 0;
    LPWSTR* raw = CommandLineToArgvW(GetCommandLineW(), &count);
    if (!raw) asc::win::check_hresult(HRESULT_FROM_WIN32(GetLastError()), "Parse command line");
    std::optional<std::wstring> result;
    if (count == 2) result = raw[1];
    else if (count != 1) { LocalFree(raw); throw std::invalid_argument("unsupported command-line arguments"); }
    LocalFree(raw);
    return result;
}
}

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int) {
    bool media_foundation_started = false;
    bool headless = false;
    try {
        const auto command = command_line_argument();
        headless = command.has_value();
        if (command == L"--verify-portable-payload") {
            asc::win::deployment::verify_portable_payload(instance);
            return 0;
        }

        // Elevated registration helpers are invoked while the parent owns the
        // deployment lock and therefore must not acquire it again.
        std::optional<NamedMutexLock> deployment_lock;
        if (command != register_under_lock && command != unregister_under_lock)
            deployment_lock.emplace(asc::win::deployment::deployment_mutex_name, false);

        if (command == register_under_lock || command == L"--portable-register") {
            asc::win::deployment::install_portable_source(instance);
            return 0;
        }
        if (command == unregister_under_lock || command == L"--portable-unregister") {
            asc::win::deployment::remove_portable_source();
            return 0;
        }

        NamedMutexLock application_lock(asc::win::deployment::machine_application_mutex_name, true);
        if (application_lock.already_exists())
            throw std::runtime_error("Automatic Screen Camera is already running");

        winrt::init_apartment(winrt::apartment_type::single_threaded);
        asc::win::check_hresult(MFStartup(MF_VERSION, MFSTARTUP_FULL), "MFStartup");
        media_foundation_started = true;

        if (command == L"--cleanup-portable") {
            asc::win::VirtualCamera::remove_registration();
            asc::win::deployment::remove_portable_source_elevated();
            asc::win::deployment::remove_current_user_startup_entry();
            MFShutdown();
            return 0;
        }
        if (command) throw std::invalid_argument("unsupported command-line argument");

        asc::win::deployment::ensure_portable_source(instance, application_lock.get());
        deployment_lock.reset();
        int result = 0;
        { asc::win::App app(instance); result = app.run(); }
        MFShutdown();
        return result;
    } catch (const std::exception& error) {
        if (headless) { OutputDebugStringA(error.what()); OutputDebugStringA("\n"); }
        else MessageBoxA(nullptr, error.what(), "Automatic Screen Camera could not start", MB_OK | MB_ICONERROR);
        if (media_foundation_started) MFShutdown();
        return 1;
    }
}
