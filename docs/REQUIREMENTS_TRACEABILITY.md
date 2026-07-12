# Requirements traceability

This matrix tracks the original specification against authoritative evidence in the current worktree. “Implemented” means source evidence exists; it does not claim Windows runtime verification. Hardware-dependent rows remain unproven until the Windows acceptance suite is executed.

| Requirement area | Implementation evidence | Verification status |
|---|---|---|
| Standalone local-only architecture | `src/windows/app.cpp`, `shared_frame.cpp`, `media_source/`; no OBS/plugin or network runtime dependency | Implemented; Windows runtime pending |
| Webcam/video device enumeration, identifiers and native formats | `device_enumerator.cpp`, Settings device list | Implemented; physical devices pending |
| Saved source, unavailable-source preservation and automatic retry | `AppConfig`, `ConfigStore`, `App::recovery_loop`, configurable reconnect policy | Portable persistence verified; disconnect test pending |
| Manual video-source restart | `App::restart_video_input`, tray and Settings commands | Implemented; device test pending |
| Full-monitor Windows Graphics Capture, cursor default off | `ScreenCapture`, `AppConfig::cursor_visible` | Implemented; capture test pending |
| GPU scaling/blending and 1080p30/720p30 output | `Compositor`, D3D11 shaders, camera media types | Implemented; cadence/GPU test pending |
| Reference capture with monitor choice, hidden window, three-second delay and no cursor | tray monitor menu, `set_reference_monitor`, temporary cursorless `ScreenCapture` | Implemented; UI test pending |
| PNG/JPEG/BMP import and internal copy | `ReferenceStore::import_image`, internal `reference.png` | Implemented; Windows WIC test pending |
| Persisted optimized comparison thumbnail | `reference-thumbnail.gray`, `save_reference_thumbnail` | Implemented |
| Robust low-resolution similarity | `image_similarity`, GPU 160×90 downscale | Portable similarity tests pass; GPU path pending |
| 250 ms checks, threshold and match/mismatch debounce | `DebouncedDetector`, Settings | Portable tests pass |
| Periodic and event-driven all-monitor rediscovery | `rescan_loop`, display/session/power handlers | Implemented; display-event tests pending |
| Stable monitor identity and renumbering resistance | DisplayConfig device path/adapter/EDID manufacturer metadata, `MonitorTracker` | Selection tests pass; hardware identity test pending |
| Three-scan reassignment, margin and ambiguity handling | `MonitorTracker` | Portable tests pass |
| Missing-reference and duplicate-reference policies | `DecisionEngine`, `scan_safety_state_`, Settings behavior selector | Portable tests pass |
| Webcam/screen crossfade and mid-fade reversal | `TransitionController`, GPU compositor | Portable timing/reversal tests pass; visual test pending |
| Screen-to-screen reassignment fade | prior-screen texture and `screen_switch_mix` | Implemented; visual test pending |
| Reference is detection-only during automatic restoration | last-known-nonmatching safe screen texture and reassignment hold | Implemented; visual test pending |
| Automatic / Force webcam / Force screen | `OutputMode`, controller and UI controls | Portable policy tests pass; runtime test pending |
| Manual override banner, persistent override and return-to-auto fade | `TrayWindow`, `AppController::set_mode`, configuration | Implemented; UI test pending |
| Start/Stop with privacy-safe stopped output | controller stopped-state invariant; compositor/virtual camera remain alive | Portable invariant tests pass |
| Tray startup, status icon/tooltip and required menu actions | `TrayWindow`, installer startup entry | Implemented; shell test pending |
| Compact status dashboard and recent events | `TrayWindow::refresh`, 20-event list | Implemented |
| Persistent structured logs, levels, rotation, export/copy/clear | `EventLog`, Settings/tray commands | Portable write/export/clear tests pass |
| Recovery controls and automatic graphics/device recovery | `restart_*`, `restart_all`, `recovery_loop` | Implemented; fault-injection pending |
| Atomic configuration, backup and invalid-file preservation | `ConfigStore` | Portable corruption/recovery tests pass |
| General, input, capture, detection, output and logging settings | `SettingsWindow`, `AppConfig` | Implemented |
| Fit/fill/stretch without implicit distortion | compositor transforms | Implemented; visual test pending |
| Camera, monitor, final-output and reference previews | `PreviewWindow`, 1 fps only while visible | Implemented; UI test pending |
| Diagnostic counters and reset action | `DiagnosticCounters`, Settings reset button | Implemented |
| Windows login, sleep/unlock/display recovery | startup registry and Win32 lifecycle handlers | Implemented; lifecycle test pending |
| Windows 11 virtual camera visible to Teams/Zoom/Discord/WebRTC | `MFCreateVirtualCamera`, COM media source, RGB32/NV12 1080p/720p | Unproven until Windows consumer matrix passes |
| Long-run resource stability | bounded newest-frame storage, reusable textures/buffers, rotating logs | Design evidence only; 24-hour test pending |

## Current completion gate

The portable control plane and persistence tests pass locally. Completion still requires a Windows 11 SDK build followed by the physical acceptance procedure in `docs/ACCEPTANCE_TESTS.md`, including consumer compatibility and the 24-hour stability run.

