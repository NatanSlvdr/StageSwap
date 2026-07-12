#include "app.hpp"
#include "common.hpp"

#include <mfapi.h>
#include <winrt/base.h>
#include <shellapi.h>

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int) {
    bool media_foundation_started = false;
    try {
        winrt::init_apartment(winrt::apartment_type::single_threaded);
        asc::win::check_hresult(MFStartup(MF_VERSION, MFSTARTUP_FULL), "MFStartup");
        media_foundation_started = true;
        int argument_count = 0;
        auto arguments = CommandLineToArgvW(GetCommandLineW(), &argument_count);
        if (argument_count > 1 && std::wstring_view(arguments[1]) == L"--remove-virtual-camera") {
            asc::win::VirtualCamera::remove_registration();
            LocalFree(arguments);
            MFShutdown();
            media_foundation_started = false;
            return 0;
        }
        if (arguments) LocalFree(arguments);
        const HANDLE single_instance = CreateMutexW(nullptr, TRUE, L"Local\\AutomaticScreenCamera.TrayInstance.v1");
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
        MessageBoxA(nullptr, error.what(), "Automatic Screen Camera could not start", MB_OK | MB_ICONERROR);
        if (media_foundation_started) MFShutdown();
        return 1;
    }
}
