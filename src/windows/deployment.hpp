#pragma once

#include <windows.h>

namespace asc::win::deployment {

inline constexpr wchar_t deployment_mutex_name[] = L"Global\\AutomaticScreenCamera.StartupDeployment.v1";
inline constexpr wchar_t machine_application_mutex_name[] = L"Global\\AutomaticScreenCamera.TrayLifetime.v2";
inline constexpr wchar_t shared_mutex_security_sddl[] =
    L"D:(A;;0x00100001;;;AU)(A;;GA;;;BA)(A;;GA;;;SY)";

enum class Mode {
    none,
    portable,
};

[[nodiscard]] Mode current_mode();
void remove_current_user_startup_entry();

// The caller must own the deployment mutex and both application-lifetime mutexes.
void ensure_portable_source(HINSTANCE instance, HANDLE owned_application_mutex);
void verify_portable_payload(HINSTANCE instance);
void install_portable_source(HINSTANCE instance);
void remove_portable_source();
void remove_portable_source_elevated();

} // namespace asc::win::deployment
