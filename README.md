# Automatic Screen Camera

Automatic Screen Camera is a local-only Windows 11 virtual camera written in Rust. Automatic mode shows the webcam while a saved visual reference matches a monitor and otherwise shows that monitor. Force Webcam and Force Screen are manual overrides; changes use a reversible 500 ms fade.

Download `windows-x64-portable.exe` for an x64 Windows 11 computer.

Each EXE embeds its same-architecture Rust Media Foundation DLL. First launch requests elevation only to extract and register the DLL under `%ProgramFiles%\Automatic Screen Camera Rust Portable`; later launches run unelevated. Before deleting the EXE, exit the tray application and run:

```powershell
.\windows-x64-portable.exe --cleanup-portable
```

On every launch, the app removes any legacy portable registration and files before verifying the current embedded camera source. User configuration, references, and logs are left intact.

## Product contract

- Windows 11 x64.
- Immutable CPU BGRA frames and output fixed at 1280×720, 30 fps.
- Media Foundation webcam input, Windows Graphics Capture screen input, synchronous bounded channels, and no Tokio.
- Reference detection every 250 ms with 5-match/3-mismatch debounce; monitor discovery at startup, every 30 seconds, and on Rescan requires the same winner twice.
- CPU aspect-fit, black letterboxing, placeholder fallback, and reversible 500 ms blend.
- The virtual camera advertises only RGB32 1280×720 at 30 fps. NV12 720p is added only if Windows Camera or Zoom proves it necessary; 1080p is excluded.
- Dashboard, five settings tabs, four previews, tray/close-to-tray, warning notifications, exit confirmation, 14-day JSONL logs, and webcam/screen/virtual/all restarts.
- No hot-plug manager, sleep/resume recovery, docking recovery, dynamic formats, OBS integration, or kernel driver.

Configuration schema 1, references, and logs live under `%LocalAppData%\AutomaticScreenCameraRust`. Frames are not recorded or uploaded.

## Build, test, and package

### Testing from macOS

You do not need a physical Windows PC for routine development. Before pushing to
`main`, run the platform-independent checks and cross-target Clippy checks locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

When a development build is ready to publish, manually run the
[Windows workflow](https://github.com/NatanSlvdr/WebcamSwitcher/actions/workflows/windows.yml)
from GitHub Actions. It executes the x64 Windows tests, packages the x64 build,
and creates a GitHub release tagged with the commit's short SHA. Running it again
for the same commit replaces the release assets. The release contains the unsigned
portable executable and its checksum; pushes do not build or publish releases.

GitHub-hosted runners cannot validate an interactive desktop, a physical webcam, or
virtual-camera enumeration. Run the final smoke test on a physical x64 Windows 11
computer or an x64 Windows 11 VM. An Apple-silicon Mac cannot virtualize x64 Windows
natively, so use an x64 Windows machine for acceptance testing.

For fast incremental x64 builds on an Apple-silicon Mac, install the native
cross-compilation tools once and run the packaging wrapper:

```bash
brew install llvm
cargo install --locked cargo-xwin
./scripts/package-x64-macos.sh
```

The wrapper cross-compiles with the x64 Windows MSVC target, pins the Windows SDK,
embeds the matching DLL and Windows resources, validates the generated PE files and
payload, and writes the executable and checksum to `dist`. Cached Rust and SDK files
make subsequent builds faster. GitHub Actions remains the authoritative release
builder, and the resulting executable cannot be run or hardware-tested on macOS.

In the VM, use the short smoke pass below:

1. Launch the x64 portable executable and approve first-run registration.
2. Confirm that Windows Camera lists **Automatic Screen Camera**.
3. Verify screen capture, Force Screen, and the placeholder output.
4. Verify webcam capture and Force Webcam if the VM exposes a camera.
5. Verify Automatic mode, tray/close behavior, restart actions, and cleanup.

This gives strong day-to-day coverage, but it does not replace the physical x64
release smoke test listed in [the acceptance tests](docs/ACCEPTANCE_TESTS.md).

Install Rust 1.97.1 and Visual Studio 2022 Build Tools with Windows SDK 10.0.22621.0. The pinned toolchain file selects the Rust version. Run packaging from a Developer PowerShell where `WindowsSDKVersion` is `10.0.22621.0\\`; `xtask` rejects any missing or different SDK instead of writing misleading artifact metadata.

```powershell
rustup target add x86_64-pc-windows-msvc
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --all-targets --target x86_64-pc-windows-msvc
cargo run -p xtask -- portable x64 dist
```

`xtask` builds and validates the x64 DLL first, embeds it into the EXE, validates the PE machine type and embedded payload, and emits the EXE with a SHA-256 sidecar. Windows builds also embed an `asInvoker`, Per-Monitor-V2 manifest and executable version metadata.

See [architecture](docs/ARCHITECTURE.md), [acceptance tests](docs/ACCEPTANCE_TESTS.md), [release gates](docs/RELEASE_GATES.md), and [rewrite scope](docs/RUST_REWRITE_SCOPE.md).
