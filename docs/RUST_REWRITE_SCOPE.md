# Locked Rust rewrite scope

This scope replaced the former production application directly. The repository now contains only the Rust implementation and retained evidence-collection scripts. All unavoidable COM and unsafe code stays inside `asc-windows` and `asc-media-source`.

## Product contract to preserve

- One self-contained Windows 11 x64 application. The portable executable may embed and deploy its matching camera-source DLL; users install neither OBS nor another runtime application.
- One webcam input. Store its identifier, open it at startup, and use a fixed internal BGRA/RGB32 1280×720 at 30 fps contract.
- One screen capture selected by the saved visual reference. Compare a 160×90 grayscale image every 250 ms, with five matches and three mismatches. Scan displays at startup, every 30 seconds, and on explicit Rescan only to locate that reference.
- Automatic, Force Webcam, and Force Screen modes, with the current 500 ms reversible fade, configurable missing-source fallback, and fixed crossed-camera off screen.
- One per-user frame transport from the application to a Media Foundation custom source DLL.
- Virtual-camera output prefers RGB32 1280×720 at 30 fps and retains selectable NV12 720p for Windows Camera and Zoom compatibility; do not restore 1080p without evidence.
- Local configuration, reference image, previews, tray/main UI, startup preference, and bounded diagnostic logs.

This is a new installation with source CLSID `{402EB87C-123B-4765-9FF7-6E11CC7DA5B3}`, pipe attribute `{905306DD-B9A3-4385-A273-606E05B3208B}`, and storage under `%LocalAppData%\AutomaticScreenCameraRust`. It does not import the old schema-v2 data, but startup removes the legacy portable deployment artifacts while preserving user configuration, references, and logs.

## Deliberate non-goals

- No background supervisor, health polling, retry backoff, or automatic component graph reconstruction.
- No webcam or display hot-plug support. If a source is disconnected, the user may relaunch the application or select it again.
- No continuous webcam enumeration. At startup, use the saved identifier if present; as a small usability improvement, automatically choose the sole physical webcam when no saved device can be opened. If several candidates exist, require an explicit choice.
- No sleep/resume, docking, display-reordering, D3D-device-loss, camera-contention, or vendor-driver recovery guarantees.
- No input format matrix. Request one internal format and let the capture backend perform the conversion.
- No GPU compositor, zero-copy pipeline, encoder, audio, recording, network service, OBS integration, kernel driver, or plugin system.
- No persisted EDID or physical-monitor identity. Display scanning exists only to find the saved visual reference.
- Do not preserve `WM_DEVICECHANGE`/`WM_DPICHANGED` restart behavior merely because the current Win32 window happens to contain it.

## Dependency boundary

| Need | Recommended boundary | Decision |
|---|---|---|
| Windows APIs and COM | [`windows`](https://github.com/microsoft/windows-rs), with only required namespace features | Adopt. It is the official Rust projection and exposes `MFCreateVirtualCamera` plus COM implementation support. |
| Screen capture | [`windows-capture`](https://github.com/NiiightmareXD/windows-capture) 2.0.0 | Adopted after a warning-clean x64 target build. Native 300-frame execution remains an explicit release-machine test. |
| Webcam capture | Direct Media Foundation source reader through `windows` | Adopted instead of `nokhwa`, keeping symbolic-link enumeration, fixed RGB32 negotiation, callbacks, and COM ownership inside the Windows adapter. Native webcam execution remains a release-machine test. |
| Configuration | [`serde`](https://serde.rs/) and [`serde_json`](https://docs.rs/serde_json/latest/serde_json/) | Adopt a new typed `schema_version: 1`, atomic replacement, backup recovery, and defaults-with-warning when both files are invalid. No old-schema migration. |
| Reference import and CPU image operations | [`image`](https://docs.rs/image/latest/image/) with only PNG, JPEG, and BMP features | Adopted for decoding. Direct grayscale thumbnails, cached BGRA composition, letterboxing, and the NV12 compatibility conversion remain in the repository with unit tests. |
| Main window and previews | `eframe` 0.35 / `egui` with `wgpu` | Adopt with the retained dashboard, five settings tabs, 2×2 previews, and restrained visual parity at 100% and 150% DPI. |
| Tray | `tray-icon` 0.24 | Adopt with close-to-tray, notifications, and the retained lifecycle actions. |
| Virtual-camera media source | No production-ready drop-in crate found | Implement in the Rust DLL with `windows`. Reuse the current protocol and behavior; do not invent a new driver architecture. |

Do not make the screen/webcam wrappers mandatory until the x64 prototype passes. Falling back to direct `windows` bindings is acceptable and still produces a fully Rust codebase. It would confine unsafe/COM code to platform modules rather than spreading it through the application.

## Why the virtual-camera DLL remains special

Windows 11's [`MFCreateVirtualCamera`](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-mfcreatevirtualcamera) registers a CLSID for a custom media source. The Frame Server then loads that source outside the main application. A Rust rewrite therefore still needs:

1. a `cdylib` COM server implementing the required Media Foundation source and stream interfaces;
2. registration/deployment of the architecture-matched DLL;
3. a small per-user IPC protocol carrying the latest BGRA frame;
4. consumer media-type negotiation and sample timing.

Microsoft's [custom media source guidance](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source) defines this boundary. The [`vcam-source` Rust code in SectionSummarizer](https://github.com/jkudo/SectionSummarizer/tree/main/vcam-source) is useful proof that these COM interfaces can be implemented with `windows` 0.62, but it is reference code rather than a reusable camera crate. [`vcam-windows-rs`](https://github.com/nope-e/vcam-windows-rs) is another recent prototype and explicitly says it is not production-ready; it also has no declared repository license, so its code must not be copied.

## Workspace

```text
automatic-screen-camera/
├── crates/core/          pure state machine, frames, detector, transitions
├── crates/app/           UI, configuration, capture orchestration, composition
├── crates/windows/       webcam/screen/deployment/IPC adapters
├── crates/media-source/  cdylib implementing the virtual-camera source
└── xtask/                builds/embeds DLLs, validates PE, packages and checksums
```

`core` must compile and test on non-Windows hosts. `windows` and `media-source` should contain every `unsafe` block and every direct COM call. The application should exchange owned frames and commands over bounded channels; it does not need a generic actor framework or asynchronous runtime.

## Release sequence and gates

1. Run format, Clippy, unit tests, dependency audit, and Debug/Release builds.
2. Build the Media Foundation DLL first, validate its PE architecture, embed it in the matching application, then validate the EXE and generate its SHA-256 sidecar.
3. Execute COM/source-state, capture start/stop, stale-frame, restart, deployment, cleanup, UI, and workflow tests on Windows 11 x64.
4. Verify preferred RGB32 1280×720 at 30 fps in Windows Camera and Zoom, then explicitly exercise the retained NV12 720p fallback.

Consumer compatibility is the irreducible release risk; cross-compilation alone does not satisfy the native Windows acceptance gates.
