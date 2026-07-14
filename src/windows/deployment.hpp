#pragma once

#include <windows.h>

namespace asc::win::deployment {

inline constexpr wchar_t application_mutex_name[] = L"Local\\AutomaticScreenCamera.TrayInstance.v1";
inline constexpr wchar_t application_window_class[] = L"AutomaticScreenCameraWindow";
inline constexpr UINT exit_for_deployment_message = WM_APP + 42;

enum class Mode {
    none,
    portable,
    installed,
    unknown,
};

[[nodiscard]] Mode current_mode();
[[nodiscard]] bool is_portable_build() noexcept;
void require_application_stopped();
void stop_application_for_deployment();
void remove_current_user_startup_entry();

void ensure_portable_source(HINSTANCE instance);
void verify_portable_payload(HINSTANCE instance);
void install_portable_source(HINSTANCE instance);
void remove_portable_source();
void remove_portable_source_elevated();

} // namespace asc::win::deployment
