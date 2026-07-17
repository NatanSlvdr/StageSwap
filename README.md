# Automatic Screen Camera

Automatic Screen Camera is a local-only Windows 11 virtual camera written in Rust. Automatic mode shows the webcam while a saved visual reference matches a monitor and otherwise shows that monitor. Force Webcam and Force Screen are manual overrides; changes use a reversible 500 ms fade.

Download the executable matching the computer's native architecture:

- `windows-x64-portable.exe`
- `windows-arm64-portable.exe`

Each EXE embeds its same-architecture Rust Media Foundation DLL. First launch requests elevation only to extract and register the DLL under `%ProgramFiles%\Automatic Screen Camera Rust Portable`; later launches run unelevated. Before deleting the EXE, exit the tray application and run:

```powershell
.\windows-x64-portable.exe --cleanup-portable
```

Use the ARM64 artifact on ARM64 Windows. The old product is a separate installation: if it remains installed, run that old executable's `--cleanup-portable`; this version never imports or removes old data.

## Product contract

- Windows 11, native x64 or ARM64.
- Immutable CPU BGRA frames and output fixed at 1280×720, 30 fps.
- Media Foundation webcam input, Windows Graphics Capture screen input, synchronous bounded channels, and no Tokio.
- Reference detection every 250 ms with 5-match/3-mismatch debounce; monitor discovery at startup, every 30 seconds, and on Rescan requires the same winner twice.
- CPU aspect-fit, black letterboxing, placeholder fallback, and reversible 500 ms blend.
- The virtual camera advertises only RGB32 1280×720 at 30 fps. NV12 720p is added only if Windows Camera or Zoom proves it necessary; 1080p is excluded.
- Dashboard, five settings tabs, four previews, tray/close-to-tray, warning notifications, exit confirmation, 14-day JSONL logs, and webcam/screen/virtual/all restarts.
- No hot-plug manager, sleep/resume recovery, docking recovery, dynamic formats, OBS integration, or kernel driver.

Configuration schema 1, references, and logs live under `%LocalAppData%\AutomaticScreenCameraRust`. Frames are not recorded or uploaded.

## Build, test, and package

Install Rust 1.97.1 and Visual Studio 2022 Build Tools with Windows SDK 10.0.22621.0. The pinned toolchain file selects the Rust version. Run packaging from a Developer PowerShell where `WindowsSDKVersion` is `10.0.22621.0\\`; `xtask` rejects any missing or different SDK instead of writing misleading artifact metadata.

```powershell
rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --all-targets --target x86_64-pc-windows-msvc
cargo run -p xtask -- portable x64 dist
cargo run -p xtask -- portable arm64 dist
```

`xtask` builds and validates the DLL first, embeds it into the matching EXE, validates both PE machine types and the embedded payload, and emits each EXE with a SHA-256 sidecar. Windows builds also embed an `asInvoker`, Per-Monitor-V2 manifest and executable version metadata.

See [architecture](docs/ARCHITECTURE.md), [acceptance tests](docs/ACCEPTANCE_TESTS.md), [release gates](docs/RELEASE_GATES.md), and [rewrite scope](docs/RUST_REWRITE_SCOPE.md).
