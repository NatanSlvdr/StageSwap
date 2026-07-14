#include "app.hpp"
#include "common.hpp"
#include "deployment.hpp"

#include <mfapi.h>
#include <winrt/base.h>
#include <shellapi.h>

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int) {
    bool media_foundation_started = false;
    bool suppress_error_dialog = false;
    try {
        winrt::init_apartment(winrt::apartment_type::single_threaded);
        asc::win::check_hresult(MFStartup(MF_VERSION, MFSTARTUP_FULL), "MFStartup");
        media_foundation_started = true;
        int argument_count = 0;
        auto arguments = CommandLineToArgvW(GetCommandLineW(), &argument_count);
        if (!arguments) asc::win::check_hresult(HRESULT_FROM_WIN32(GetLastError()), "Parse command line");
        if (argument_count < 1 || argument_count > 2) {
            LocalFree(arguments);
            throw std::invalid_argument("unsupported command-line arguments");
        }
        if (argument_count == 2) {
            const std::wstring_view command(arguments[1]);
            suppress_error_dialog = command == L"--verify-portable-payload" ||
                command == L"--prepare-install" || command == L"--prepare-uninstall";
            if (command == L"--portable-register") {
                asc::win::deployment::install_portable_source(instance);
            } else if (command == L"--portable-unregister") {
                asc::win::deployment::remove_portable_source();
            } else if (command == L"--verify-portable-payload") {
                asc::win::deployment::verify_portable_payload(instance);
            } else if (command == L"--remove-virtual-camera") {
                asc::win::VirtualCamera::remove_registration();
            } else if (command == L"--cleanup-portable") {
                asc::win::deployment::require_application_stopped();
                if (asc::win::deployment::current_mode() == asc::win::deployment::Mode::installed) {
                    throw std::runtime_error("The installed edition must be removed from Windows Settings");
                }
                asc::win::VirtualCamera::remove_registration();
                asc::win::deployment::remove_portable_source_elevated();
                asc::win::deployment::remove_current_user_startup_entry();
            } else if (command == L"--prepare-install") {
                asc::win::deployment::stop_application_for_deployment();
                const auto mode = asc::win::deployment::current_mode();
                if (mode != asc::win::deployment::Mode::none) asc::win::VirtualCamera::remove_registration();
                if (mode == asc::win::deployment::Mode::portable) {
                    asc::win::deployment::remove_portable_source_elevated();
                    asc::win::deployment::remove_current_user_startup_entry();
                } else if (mode == asc::win::deployment::Mode::unknown) {
                    throw std::runtime_error("Deployment registry data is not recognized");
                }
            } else if (command == L"--prepare-uninstall") {
                asc::win::deployment::stop_application_for_deployment();
                if (asc::win::deployment::current_mode() == asc::win::deployment::Mode::installed) {
                    asc::win::VirtualCamera::remove_registration();
                    asc::win::deployment::remove_current_user_startup_entry();
                }
            } else {
                LocalFree(arguments);
                throw std::invalid_argument("unsupported command-line argument");
            }
            LocalFree(arguments);
            MFShutdown();
            media_foundation_started = false;
            return 0;
        }
        if (arguments) LocalFree(arguments);
        asc::win::deployment::ensure_portable_source(instance);
        const HANDLE single_instance = CreateMutexW(nullptr, TRUE, asc::win::deployment::application_mutex_name);
        if (!single_instance) asc::win::check_hresult(HRESULT_FROM_WIN32(GetLastError()), "Create application mutex");
        if (GetLastError() == ERROR_ALREADY_EXISTS) {
            MessageBoxW(nullptr, L"Automatic Screen Camera is already running in the system tray.", L"Automatic Screen Camera", MB_OK | MB_ICONINFORMATION);
            CloseHandle(single_instance);
            MFShutdown();
            media_foundation_started = false;
            return 0;
        }
        int result = 0;
        {
            asc::win::App app(instance);
            result = app.run();
        }
        CloseHandle(single_instance);
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
