# Requirements traceability

This matrix tracks the original specification against authoritative evidence in the current worktree. “Implemented” means source evidence exists; it does not claim Windows runtime verification. Hardware-dependent rows remain unproven until the Windows acceptance suite is executed.

| Requirement area | Implementation evidence | Verification status |
|---|---|---|
| Standalone local-only architecture | `src/windows/app.cpp`, `shared_frame.cpp`, `media_source/`; no OBS/plugin or network runtime dependency | Implemented; Windows runtime pending |
| Webcam/video device enumeration, identifiers and native formats | `device_enumerator.cpp`; Settings shows each selected source's connection state, stable identifier, and reported native formats while preserving unavailable saved sources | Implemented; physical devices pending |
| Saved source, unavailable-source preservation and automatic retry | `AppConfig`, `ConfigStore`, `build_persistent_device_choices`, unavailable Settings entry, `App::recovery_loop` | Portable persistence/unavailable-choice regression and Windows build pass; physical disconnect pending |
| Manual video-source restart | `App::restart_video_input`, tray and Settings commands | Implemented; device test pending |
| Full-monitor Windows Graphics Capture, cursor default off | `ScreenCapture`, `AppConfig::cursor_visible` | Implemented; capture test pending |
| GPU scaling/blending and 1080p30/720p30 output | `Compositor`, D3D11 shaders, camera media types | Implemented; cadence/GPU test pending |
| Reference capture with monitor choice, hidden window, three-second delay and no cursor | tray monitor menu, `set_reference_monitor`, temporary cursorless `ScreenCapture` | Implemented; UI test pending |
| PNG/JPEG/BMP import and internal copy | `ReferenceStore::import_image`, internal `reference.png` | Implemented; Windows WIC test pending |
| Persisted optimized comparison thumbnail | `reference-thumbnail.gray`, `save_reference_thumbnail` | Implemented |
| Robust low-resolution similarity | `image_similarity`, GPU 160×90 downscale | Portable similarity tests pass; GPU path pending |
| 250 ms checks, threshold and match/mismatch debounce | `DebouncedDetector`, Settings | Portable tests pass |
| Periodic and event-driven all-monitor rediscovery | `rescan_loop`, display/session/power handlers | Implemented; display-event tests pending |
| Stable monitor identity, observation history and renumbering resistance | DisplayConfig device path/adapter/EDID metadata; `MonitorTracker` retains bounded per-display similarity, scan/reference timestamps, capture validity, and prior-tracked state; a missing persisted monitor cannot fall back to an arbitrary display | Portable identity/observation/history tests pass; hardware identity test pending |
| Three-scan reassignment, margin and ambiguity handling | `MonitorTracker` | Portable tests pass |
| Missing-reference and duplicate-reference policies | `DecisionEngine`, `scan_safety_state_`, Settings behavior selector | Portable tests pass |
| Webcam/screen crossfade and mid-fade reversal | `TransitionController`, GPU compositor | Portable timing/reversal tests pass; visual test pending |
| Screen-to-screen reassignment fade | prior-screen texture and `screen_switch_mix` | Implemented; visual test pending |
| Reference is detection-only during automatic restoration | last-known-nonmatching safe screen texture and reassignment hold | Implemented; visual test pending |
| Automatic / Force webcam / Force screen | `OutputMode`, controller and UI controls | Portable policy tests pass; runtime test pending |
| Manual override banner, persistent override and return-to-auto fade | `TrayWindow`, `AppController::set_mode`, configuration | Implemented; UI test pending |
| Start/Stop with privacy-safe stopped output | controller stopped-state invariant; compositor/virtual camera remain alive | Portable invariant tests pass |
| Tray startup, status icon/tooltip and required menu actions | `TrayWindow`, installer startup entry | Implemented; shell test pending |
| Compact status dashboard and recent events | `TrayWindow::refresh`, bounded 20-event list with timestamp, event code/type, component, message, and structured context | Portable event-summary regression passes; Windows UI pending |
| Persistent structured logs, levels, rotation, export/copy/clear | `EventLog`; Settings and tray both expose diagnostic log actions | Portable write/export/clear tests pass |
| Recovery controls and automatic graphics/device recovery | `restart_*`, `restart_all`, `recovery_loop` | Implemented; fault-injection pending |
| Atomic configuration, backup and invalid-file preservation | `ConfigStore`; confirmed automatic monitor reassignments are persisted; unavailable saved cameras remain selected in Settings | Portable corruption/recovery and unavailable-device tests pass |
| General, input, capture, detection, output and logging settings | `SettingsWindow`, `AppConfig` | Implemented |
| Fit/fill/stretch without implicit distortion | compositor transforms | Implemented; visual test pending |
| Camera, monitor, final-output and reference previews | `PreviewWindow`, 1 fps only while visible | Implemented; UI test pending |
| Diagnostic counters and reset action | `DiagnosticCounters`, Settings reset button | Implemented |
| Windows login, sleep/unlock/display recovery | startup registry and Win32 lifecycle handlers | Implemented; lifecycle test pending |
| Windows 11 virtual camera visible to Teams/Zoom/Discord/WebRTC | `MFCreateVirtualCamera`, COM media source, RGB32/NV12 1080p/720p | Unproven until Windows consumer matrix passes |
| Long-run resource stability | bounded newest-frame storage, reusable textures/buffers, rotating logs | Design evidence only; 24-hour test pending |
| Reproducible Windows build | `CMakePresets.json`; Visual Studio 2022 x64 and Windows SDK 10.0.22621.0 presets; static MSVC runtime | Current-head Debug/Release hosted build and tests pass; repeated/interactive gates pending |
| Warning-clean and sanitizer CI | `ASC_WARNINGS_AS_ERRORS`, `ASC_ENABLE_SANITIZERS`, `.github/workflows/windows.yml` | Current-head Windows `/W4 /WX`, ASan/UBSan, and TSan jobs pass |
| Release packaging and integrity | `scripts/package.ps1`; EXE, DLL, PDBs, installer scripts, JUnit results, metadata, and SHA-256 manifests | Current-head package generated; external and every internal SHA-256 verified; install cycle pending |
| Release qualification | `docs/RELEASE_GATES.md`; hosted and persistent interactive Windows/Zoom gates | Five-run hosted gate satisfied for `2289419`; interactive gates remain unsatisfied |
| Webcam fallback and reconnect safety | `video_format.hpp`, `VideoInput` ranked native formats, MF conversion, callback proxy/drain, preserved symbolic link | Portable ranking passes; UVC unplug/replug pending |
| Serialized capture lifecycle | `App::lifecycle_mutex_`, `compositor_mutex_`; `ScreenCapture` callback generation/drain | Implemented; concurrent Windows fault injection pending |
| Frame Server media-source compliance | synchronized `MediaSource`/`MediaStream`; validated `SetD3DManager`; 2D RGB32/NV12 buffers; QPC pacing | Implemented; Zoom/open-close/cadence gates pending |
| IPC and frame-path performance | triple-buffered GPU readback, cached scale maps/SRVs, same-size copies, preallocated vectors | Implemented; VM CPU/resource gates pending |
| Independent release probes | `asc_test_screen_fixture`, `asc_mf_output_probe`, environment collector and gate evaluator | Built separately from production; interactive execution pending |
| RGB/NV12 conversion safety | shared `pixel_conversion.hpp` production helper; exact-color, padded-stride, truncation, and no-write-on-error tests | Portable ASan/UBSan tests pass; Windows 2D-buffer path pending |
| IPC packet validation | shared `shared_frame_validation.hpp`; overflow/size/stride/invalidation tests | Portable ASan/UBSan tests pass; named-pipe fault injection pending |
| State/configuration concurrency | `AppController` synchronized snapshot API and four-thread mutation/snapshot stress test | Portable ASan/UBSan/TSan tests pass |

## Current completion gate

At commit `22894191f55cfb241101cd2902c05118a5cd6d6a`, five clean workflows from fresh checkouts passed Windows x64 Debug/Release builds and tests, Release packaging, ASan/UBSan, and TSan: [29280472320](https://github.com/NatanSlvdr/WebcamSwitcher/actions/runs/29280472320), [29280751676](https://github.com/NatanSlvdr/WebcamSwitcher/actions/runs/29280751676), [29280762935](https://github.com/NatanSlvdr/WebcamSwitcher/actions/runs/29280762935), [29280773832](https://github.com/NatanSlvdr/WebcamSwitcher/actions/runs/29280773832), and [29280785528](https://github.com/NatanSlvdr/WebcamSwitcher/actions/runs/29280785528). The downloaded Release artifact from the first run identified that exact revision, reported zero JUnit failures, and passed its external ZIP digest plus every internal SHA-256 entry without line-ending normalization. GitHub retains the JUnit, unsigned package, and verification-tool artifacts for each run.

This satisfies the five-run hosted gate in `docs/RELEASE_GATES.md`. Hosted runners do not provide the physical camera, persistent interactive desktop, downstream consumer matrix, lifecycle fault injection, or duration needed for product acceptance. Completion and any reliability claim still require the persistent Windows/Zoom procedure, including the two consecutive 24-hour soaks, for the recorded release environment.
