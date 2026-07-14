#include "app.hpp"
#include "common.hpp"
#include "deployment.hpp"

#include <mfapi.h>
#include <winrt/base.h>
#include <shellapi.h>
#include <sddl.h>

#include <optional>
#include <stdexcept>
#include <string>

namespace {
constexpr wchar_t portable_register_under_lock[] = L"--portable-register-under-lock";
constexpr wchar_t portable_unregister_under_lock[] = L"--portable-unregister-under-lock";
constexpr wchar_t prepare_uninstall_under_lock[] = L"--prepare-uninstall-under-lock";
constexpr wchar_t remove_virtual_camera_under_lock[] = L"--remove-virtual-camera-under-lock";
constexpr wchar_t stop_for_uninstall[] = L"--stop-for-uninstall";
constexpr wchar_t cleanup_user_after_uninstall[] = L"--cleanup-user-after-uninstall";
constexpr wchar_t cleanup_user_after_uninstall_and_prompt_restart[] =
    L"--cleanup-user-after-uninstall-and-prompt-restart";

struct CreatedMutex {
    HANDLE handle{nullptr};
    DWORD status{ERROR_SUCCESS};
};

CreatedMutex create_shared_mutex(const wchar_t* name, const BOOL initial_owner,
                                 const std::string& operation) {
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
            asc::win::deployment::shared_mutex_security_sddl, SDDL_REVISION_1, &descriptor, nullptr)) {
        asc::win::check_hresult(HRESULT_FROM_WIN32(GetLastError()), "Create shared mutex security descriptor");
    }
    SECURITY_ATTRIBUTES attributes{static_cast<DWORD>(sizeof(SECURITY_ATTRIBUTES)), descriptor, FALSE};
    SetLastError(ERROR_SUCCESS);
    const HANDLE handle = CreateMutexExW(
        &attributes, name, initial_owner ? CREATE_MUTEX_INITIAL_OWNER : 0,
        SYNCHRONIZE | MUTEX_MODIFY_STATE);
    const DWORD status = GetLastError();
    LocalFree(descriptor);
    if (!handle) asc::win::check_hresult(HRESULT_FROM_WIN32(status), operation);
    return {handle, status};
}

class NamedMutexLock {
public:
    explicit NamedMutexLock(const wchar_t* name) {
        mutex_ = create_shared_mutex(name, FALSE, "Create startup/deployment mutex").handle;
        constexpr DWORD wait_milliseconds = 30000;
        const DWORD wait_result = WaitForSingleObject(mutex_, wait_milliseconds);
        if (wait_result == WAIT_OBJECT_0 || wait_result == WAIT_ABANDONED) return;
        const DWORD error = wait_result == WAIT_FAILED ? GetLastError() :
            wait_result == WAIT_TIMEOUT ? ERROR_TIMEOUT : ERROR_GEN_FAILURE;
        CloseHandle(mutex_);
        mutex_ = nullptr;
        throw asc::win::HResultError(HRESULT_FROM_WIN32(error), "Acquire startup/deployment mutex");
    }

    NamedMutexLock(const NamedMutexLock&) = delete;
    NamedMutexLock& operator=(const NamedMutexLock&) = delete;

    ~NamedMutexLock() {
        if (!mutex_) return;
        ReleaseMutex(mutex_);
        CloseHandle(mutex_);
    }

private:
    HANDLE mutex_{nullptr};
};

class ApplicationMutex {
public:
    explicit ApplicationMutex(const wchar_t* name) {
        const auto created = create_shared_mutex(name, TRUE, "Create application mutex");
        mutex_ = created.handle;
        already_exists_ = created.status == ERROR_ALREADY_EXISTS;
    }

    ApplicationMutex(const ApplicationMutex&) = delete;
    ApplicationMutex& operator=(const ApplicationMutex&) = delete;

    ~ApplicationMutex() {
        if (!mutex_) return;
        if (!already_exists_) ReleaseMutex(mutex_);
        CloseHandle(mutex_);
    }

    [[nodiscard]] bool already_exists() const noexcept { return already_exists_; }
    [[nodiscard]] HANDLE get() const noexcept { return mutex_; }

private:
    HANDLE mutex_{nullptr};
    bool already_exists_{false};
};

class ApplicationMutexSet {
public:
    [[nodiscard]] bool claim() {
        machine_.emplace(asc::win::deployment::machine_application_mutex_name);
        if (machine_->already_exists()) {
            reset();
            return false;
        }
        legacy_.emplace(asc::win::deployment::legacy_application_mutex_name);
        if (legacy_->already_exists()) {
            reset();
            return false;
        }
        return true;
    }

    void reset() noexcept {
        legacy_.reset();
        machine_.reset();
    }

    [[nodiscard]] HANDLE machine_handle() const noexcept {
        return machine_ ? machine_->get() : nullptr;
    }

private:
    // Release in reverse order: legacy/session mutex, then machine mutex.
    std::optional<ApplicationMutex> machine_;
    std::optional<ApplicationMutex> legacy_;
};

void claim_application_mutexes_for_deployment(ApplicationMutexSet& mutexes) {
    if (mutexes.claim()) return;
    throw std::runtime_error("Exit Automatic Screen Camera from the system tray before continuing");
}

struct LocalArguments {
    LPWSTR* value{nullptr};
    explicit LocalArguments(LPWSTR* arguments) : value(arguments) {}
    LocalArguments(const LocalArguments&) = delete;
    LocalArguments& operator=(const LocalArguments&) = delete;
    ~LocalArguments() { if (value) LocalFree(value); }
};

std::optional<std::wstring> command_line_argument() {
    int argument_count = 0;
    LocalArguments arguments{CommandLineToArgvW(GetCommandLineW(), &argument_count)};
    if (!arguments.value) {
        asc::win::check_hresult(HRESULT_FROM_WIN32(GetLastError()), "Parse command line");
    }
    if (argument_count < 1 || argument_count > 2) {
        throw std::invalid_argument("unsupported command-line arguments");
    }
    if (argument_count == 2) return std::wstring(arguments.value[1]);
    return std::nullopt;
}

bool suppresses_error_dialog(const std::optional<std::wstring>& command) {
    return command == L"--verify-portable-payload" || command == L"--prepare-install" ||
        command == L"--prepare-uninstall" || command == prepare_uninstall_under_lock ||
        command == remove_virtual_camera_under_lock ||
        command == stop_for_uninstall || command == cleanup_user_after_uninstall ||
        command == cleanup_user_after_uninstall_and_prompt_restart ||
        command == portable_register_under_lock || command == portable_unregister_under_lock;
}

bool runs_under_parent_deployment_lock(const std::optional<std::wstring>& command) {
    return command == portable_register_under_lock || command == portable_unregister_under_lock ||
        command == prepare_uninstall_under_lock || command == remove_virtual_camera_under_lock;
}

bool reserves_application_mutex(const std::wstring& command) {
    return command == L"--portable-register" || command == L"--portable-unregister" ||
        command == L"--remove-virtual-camera" || command == L"--cleanup-portable" ||
        command == L"--prepare-install" || command == L"--prepare-uninstall" ||
        command == stop_for_uninstall || command == cleanup_user_after_uninstall ||
        command == cleanup_user_after_uninstall_and_prompt_restart;
}

bool stops_application_before_reserving(const std::wstring& command) {
    return command == L"--prepare-install" || command == L"--prepare-uninstall" ||
        command == stop_for_uninstall;
}

void remove_virtual_camera_best_effort() noexcept {
    try {
        asc::win::VirtualCamera::remove_registration();
    } catch (const std::exception& error) {
        OutputDebugStringA("Could not remove the virtual camera registration during uninstall: ");
        OutputDebugStringA(error.what());
        OutputDebugStringA("\n");
    } catch (...) {
        OutputDebugStringA("Could not remove the virtual camera registration during uninstall\n");
    }
}

void remove_current_user_startup_best_effort() noexcept {
    try {
        asc::win::deployment::remove_current_user_startup_entry();
    } catch (const std::exception& error) {
        OutputDebugStringA("Could not remove the current-user startup entry during uninstall: ");
        OutputDebugStringA(error.what());
        OutputDebugStringA("\n");
    } catch (...) {
        OutputDebugStringA("Could not remove the current-user startup entry during uninstall\n");
    }
}

void cleanup_current_user_after_uninstall() noexcept {
    bool cleanup_media_foundation_started = false;
    try {
        winrt::init_apartment(winrt::apartment_type::single_threaded);
        asc::win::check_hresult(MFStartup(MF_VERSION, MFSTARTUP_FULL), "MFStartup for uninstall cleanup");
        cleanup_media_foundation_started = true;
        remove_virtual_camera_best_effort();
    } catch (const std::exception& error) {
        OutputDebugStringA("Could not initialize virtual-camera cleanup after uninstall: ");
        OutputDebugStringA(error.what());
        OutputDebugStringA("\n");
    } catch (...) {
        OutputDebugStringA("Could not initialize virtual-camera cleanup after uninstall\n");
    }
    if (cleanup_media_foundation_started) MFShutdown();

    // Startup removal must still run if camera privacy or Media Foundation blocks
    // the independent virtual-camera cleanup above.
    remove_current_user_startup_best_effort();
}

void prompt_for_restart_after_uninstall() noexcept {
    const int response = MessageBoxW(
        nullptr,
        L"Automatic Screen Camera was removed. Restart Windows now to finish deleting any files that were in use?",
        L"Automatic Screen Camera", MB_YESNO | MB_ICONINFORMATION | MB_DEFBUTTON2);
    if (response != IDYES) return;

    wchar_t system_directory[MAX_PATH]{};
    const UINT length = GetSystemDirectoryW(system_directory, ARRAYSIZE(system_directory));
    if (length == 0 || length >= ARRAYSIZE(system_directory)) return;
    const std::wstring shutdown = std::wstring(system_directory, length) + L"\\shutdown.exe";
    ShellExecuteW(nullptr, L"open", shutdown.c_str(), L"/r /t 0", system_directory, SW_HIDE);
}
} // namespace

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int) {
    bool media_foundation_started = false;
    bool suppress_error_dialog = false;
    try {
        const auto command = command_line_argument();
        suppress_error_dialog = suppresses_error_dialog(command);

        // Payload verification is deliberately independent of WinRT and Media Foundation.
        // Besides doing less work, this keeps every verifier failure on the headless path.
        if (command == L"--verify-portable-payload") {
            asc::win::deployment::verify_portable_payload(instance);
            return 0;
        }

        // An owning parent holds the deployment, machine-lifetime, and legacy session
        // mutexes while it invokes each under-lock helper. Re-acquiring any of them in
        // the helper would deadlock or self-report a running app.
        std::optional<NamedMutexLock> deployment_lock;
        if (!runs_under_parent_deployment_lock(command)) {
            deployment_lock.emplace(asc::win::deployment::deployment_mutex_name);
        }

        ApplicationMutexSet application_mutexes;
        if (!command) {
            if (!application_mutexes.claim()) {
                deployment_lock.reset();
                MessageBoxW(nullptr, L"Automatic Screen Camera is already running in the system tray, possibly in another Windows session.",
                            L"Automatic Screen Camera", MB_OK | MB_ICONINFORMATION);
                return 0;
            }
            // An installed executable can already be mapped into memory when uninstall
            // takes the global gate. Recheck the committed marker after that wait and the
            // lifetime reservations so it cannot resume after file removal and recreate state.
            if (!asc::win::deployment::is_portable_build() &&
                asc::win::deployment::current_mode() != asc::win::deployment::Mode::installed) {
                throw std::runtime_error("The installed edition is no longer registered; reinstall it before launching");
            }
        } else if (!runs_under_parent_deployment_lock(command) && reserves_application_mutex(*command)) {
            // Prepare commands must first ask the current tray owner to exit. The global
            // gate excludes new binaries; claiming the machine and legacy session mutexes
            // immediately afterward also reserves the mutation against older same-session
            // binaries that do not know the deployment gate.
            if (stops_application_before_reserving(*command)) {
                asc::win::deployment::stop_application_for_deployment();
            }
            claim_application_mutexes_for_deployment(application_mutexes);
        }

        // The uninstall launcher only uses this command to close and briefly reserve
        // the tray. It must not mutate user state before elevation has succeeded.
        if (command == stop_for_uninstall) return 0;

        // This executable may be a temporary copy retained across machine-file removal.
        // Do not consult the now-removed deployment marker; both operations are per-user
        // and intentionally best effort after the real uninstaller reports success.
        if (command == cleanup_user_after_uninstall ||
            command == cleanup_user_after_uninstall_and_prompt_restart) {
            cleanup_current_user_after_uninstall();
            application_mutexes.reset();
            deployment_lock.reset();
            if (command == cleanup_user_after_uninstall_and_prompt_restart) {
                prompt_for_restart_after_uninstall();
            }
            return 0;
        }

        winrt::init_apartment(winrt::apartment_type::single_threaded);
        asc::win::check_hresult(MFStartup(MF_VERSION, MFSTARTUP_FULL), "MFStartup");
        media_foundation_started = true;

        if (command) {
            if (*command == L"--portable-register" || *command == portable_register_under_lock) {
                asc::win::deployment::install_portable_source(instance);
            } else if (*command == L"--portable-unregister" || *command == portable_unregister_under_lock) {
                asc::win::deployment::remove_portable_source();
            } else if (*command == L"--remove-virtual-camera") {
                asc::win::VirtualCamera::remove_registration();
            } else if (*command == remove_virtual_camera_under_lock) {
                remove_virtual_camera_best_effort();
            } else if (*command == L"--cleanup-portable") {
                if (asc::win::deployment::current_mode() == asc::win::deployment::Mode::installed) {
                    throw std::runtime_error("The installed edition must be removed from Windows Settings");
                }
                asc::win::VirtualCamera::remove_registration();
                asc::win::deployment::remove_portable_source_elevated();
                asc::win::deployment::remove_current_user_startup_entry();
            } else if (*command == L"--prepare-install") {
                const auto mode = asc::win::deployment::current_mode();
                if (mode != asc::win::deployment::Mode::none) asc::win::VirtualCamera::remove_registration();
                if (mode == asc::win::deployment::Mode::portable) {
                    asc::win::deployment::remove_portable_source_elevated();
                    asc::win::deployment::remove_current_user_startup_entry();
                } else if (mode == asc::win::deployment::Mode::unknown) {
                    throw std::runtime_error("Deployment registry data is not recognized");
                }
            } else if (*command == L"--prepare-uninstall" || *command == prepare_uninstall_under_lock) {
                if (asc::win::deployment::current_mode() == asc::win::deployment::Mode::installed) {
                    remove_virtual_camera_best_effort();
                    asc::win::deployment::remove_current_user_startup_entry();
                }
            } else {
                throw std::invalid_argument("unsupported command-line argument");
            }
            MFShutdown();
            media_foundation_started = false;
            return 0;
        }

        // Own both tray-lifetime mutexes throughout portable ensure/deploy. Current
        // binaries are excluded machine-wide; older binaries in this session either
        // observe the legacy reservation or lose their later single-instance claim.
        asc::win::deployment::ensure_portable_source(instance, application_mutexes.machine_handle());
        deployment_lock.reset();
        int result = 0;
        {
            asc::win::App app(instance);
            result = app.run();
        }
        application_mutexes.reset();
        MFShutdown();
        media_foundation_started = false;
        return result;
    } catch (const std::exception& error) {
        if (suppress_error_dialog) {
            OutputDebugStringA(error.what());
            OutputDebugStringA("\n");
        } else {
            MessageBoxA(nullptr, error.what(), "Automatic Screen Camera could not start", MB_OK | MB_ICONERROR);
        }
        if (media_foundation_started) MFShutdown();
        return 1;
    }
}
