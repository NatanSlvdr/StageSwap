# StageSwap developer guide

This document contains the engineering, build, test, packaging, and release information for StageSwap. The main [README](README.md) is the product presentation and end-user guide.

## Product contract

StageSwap is a local-only, native Windows 11 x64 virtual camera for automatic Zoom retransmission during congregation meetings using JW Library. It watches the second display visually and does not integrate with or control JW Library.

- Frames are immutable CPU BGRA and the internal/output contract is fixed at 1280×720, 30 fps.
- Webcam input uses Media Foundation with native-aspect-aware centered 16:9 cropping. Native 16:9 and unknown-aspect signals pass through unchanged. Screen input uses Windows Graphics Capture. Cropping feeds both preview and output.
- The runtime uses synchronous bounded channels and no Tokio.
- Reference detection runs every 250 ms with five-match/three-mismatch debounce.
- Monitor selection is restored by friendly label with secondary-display fallback. Reference discovery requires the same winning display twice, does not pause output, runs at startup, when Settings opens, after reference changes, and every 30 seconds by default, and can be limited to explicit rescans.
- Independent black-screen recovery samples only the selected capture every 30 seconds. Two consecutive near-black checks restart screen capture; the setting can be disabled without changing reference discovery.
- Composition uses CPU aspect-fit, black letterboxing, configurable missing-source fallback, and a reversible 500 ms blend.
- The virtual camera prefers RGB32 1280×720 at 30 fps. Selectable NV12 720p is retained for Windows Camera and Zoom compatibility; 1080p is intentionally excluded.
- Output uses deadline-based 30 fps pacing. Visible dashboard previews use latest-only conversion workers and display-sized textures. FPS is runtime-owned and remains meaningful while the dashboard is hidden.
- While automation is stopped, a fixed black screen with the centered StageSwap icon is published at 30 fps. The virtual camera generates the same frame whenever the app publisher is absent.
- The app includes the dashboard, five settings categories, contextual previews, shared executable/window/tray icon, synchronized tray controls, warning notifications, exit confirmation, 14-day JSONL logs, and individual/all component restarts.
- There is no general hot-plug manager, sleep/resume recovery, docking recovery, dynamic format management, OBS integration, or kernel driver. Screen recovery is limited to the selected-capture black-frame check.

Configuration schema 1, references, and logs live under `%LocalAppData%\StageSwap`. Frames are never recorded or uploaded.

## Workspace

```text
StageSwap/
├── crates/core/          state machine, frames, detector, and transitions
├── crates/app/           UI, configuration, orchestration, and composition
├── crates/windows/       webcam, screen, deployment, and IPC adapters
├── crates/media-source/  Media Foundation virtual-camera source DLL
├── xtask/                packaging, PE validation, and checksums
├── scripts/              local packaging and release-evidence helpers
└── docs/                 architecture, acceptance, and release specifications
```

The workspace uses Rust edition 2024 and is pinned to Rust 1.97.1 in `rust-toolchain.toml`. `stageswap-core` and `xtask` are the default members so platform-independent work can be built and tested on non-Windows hosts. Direct Windows APIs, COM, and unavoidable unsafe code belong in `stageswap-windows` and `stageswap-media-source`.

## Routine development checks

Before pushing to `main`, run the platform-independent format, lint, and test suite:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

On macOS, also cross-check the Windows target:

```bash
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

These checks provide strong day-to-day coverage, but GitHub-hosted runners and macOS cannot validate an interactive Windows desktop, a physical webcam, or virtual-camera enumeration. Native Windows acceptance testing remains required.

## Interactive UI preview on macOS

Launch the real StageSwap Settings interface with deterministic mock cameras, displays, reference imagery, and healthy runtime states:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview
```

General opens by default. To open a particular page:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview matching
```

Available preview names are `dashboard`, `general`, `webcam`, `screen`, `matching`, and `diagnostics`. You can still navigate between every page after launch.

Setup-guide previews are `setup-1` through `setup-5`. Each name opens one deterministic full-window step for the JW Library-to-Zoom workflow.

For the idle-reference step, append `--ui-setup-reference-state captured`, `empty`, `review`, or `missing-screen` to inspect its deterministic saved, initial, candidate-review, and unavailable-display states:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview setup-4 --ui-setup-reference-state review
```

The `setup-4` capture, retake, confirmation, and capture-again controls are interactive in preview mode. They update only the in-memory mock frames and never write `reference.png`.

Dialog previews are `dialog-exit`, `dialog-clear-logs`, `dialog-reference-capture`, `dialog-admin`, `dialog-replace-baseline`, `dialog-load-admin-config`, and `dialog-remove-baseline`.

The English user-guide screenshot set can be regenerated from the repository root with:

```bash
./scripts/capture-user-guide-screenshots.sh
```

The helper writes verified 1280×720 PNGs to `docs/images/user-guide/`.

Append `--ui-language en-US`, `--ui-language fr-FR`, or `--ui-language es` to render any preview in a supported language:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview matching --ui-language fr-FR
```

Preview mode uses a temporary configuration directory and does not save changes to the normal StageSwap configuration. It is intended for checking layout, wrapping, conditional explanations, and interactions. Windows tray behavior, native file dialogs, hardware capture, and exact Windows font rendering still require native Windows acceptance testing.

## Fast x64 packaging from macOS

An Apple-silicon Mac can cross-compile the x64 Windows build but cannot run or hardware-test it. Install the native tools once:

```bash
brew install llvm
cargo install --locked cargo-xwin
```

Then run:

```bash
./scripts/package-x64-macos.sh
```

The wrapper cross-compiles with the x64 Windows MSVC target, pins the Windows SDK, embeds the matching Media Foundation DLL and Windows resources, validates the generated PE files and payload, and writes the versioned executable and checksum to `dist/`.

If the optimized release build differs from the latest versioned artifact in `dist/`, the patch number is incremented and the selected version is persisted to `Cargo.toml` and `Cargo.lock` before the final DLL and EXE are rebuilt. Rebuilding identical bytes with an already synchronized workspace keeps the existing version. The filename, application UI, Windows version resources, and checksum metadata therefore use one version. Rust and SDK caches make later builds faster. GitHub Actions remains the authoritative release builder.

## Native Windows build and package

Install:

- Rust 1.97.1
- Visual Studio 2022 Build Tools
- Windows SDK 10.0.22621.0

Use a Developer PowerShell with `WindowsSDKVersion` set to `10.0.22621.0\\`. `xtask` deliberately rejects a missing or different SDK so it cannot write misleading artifact metadata.

```powershell
rustup target add x86_64-pc-windows-msvc
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --all-targets --target x86_64-pc-windows-msvc
cargo run --release -p xtask -- package x64 dist
```

`xtask` builds and validates the x64 DLL first, embeds it into the EXE, validates the PE machine type and embedded payload, and emits the EXE with a SHA-256 sidecar. Packaging and both Windows payloads use Cargo's optimized release profile. Windows builds also embed an `asInvoker`, Per-Monitor-V2 manifest and executable version metadata.

## Packaging and deployment model

`StageSwap_win64_vX.Y.Z.exe` is an installerless, self-deploying application. A downloaded copy offers to:

- install atomically at `%LocalAppData%\Programs\StageSwap\StageSwap.exe`, creating per-user Start Menu and Desktop shortcuts; or
- run once without registering its download path for Windows startup.

The executable embeds its x64 Media Foundation source DLL. First launch verifies the native architecture and elevates only to extract and register that DLL under `%ProgramFiles%\StageSwap`; ordinary launches remain unelevated.

DLL payloads use content-versioned names, allowing a new build to register while a camera application still has the prior DLL loaded. Unlocked stale copies are removed immediately; locked copies are scheduled for deletion at reboot.

Opening a different downloaded build asks to replace the installed copy, gracefully closes the running managed instance, activates the new executable, and opens its dashboard. Failed replacement startup rolls back to the previous executable. A legacy instance that cannot participate in the handoff must be exited manually and is never force-terminated.

Cleanup entry points are intentionally different:

```powershell
# Remove startup and the virtual-camera deployment; keep app and user data
.\StageSwap_win64_vX.Y.Z.exe --cleanup

# Also remove the managed app and shortcuts; keep user data
.\StageSwap_win64_vX.Y.Z.exe --uninstall
```

StageSwap owns independent storage, startup, IPC, COM, virtual-camera, and deployment identities. It does not migrate, unregister, overwrite, or delete Automatic Screen Camera data or deployments.

## Release workflow

When a development build is ready to publish, manually run the [Windows workflow](https://github.com/NatanSlvdr/StageSwap/actions/workflows/windows.yml) in GitHub Actions. It runs the x64 Windows tests, packages the x64 build, and creates a GitHub release tagged with the commit's short SHA. Re-running it for the same commit replaces the release assets. Ordinary pushes do not build or publish releases.

A release contains the unsigned versioned executable and its SHA-256 sidecar. The full required gates are documented in [Release gates](docs/RELEASE_GATES.md).

## Native acceptance testing

Run the final smoke pass on a physical x64 Windows 11 computer or an x64 Windows 11 VM. An Apple-silicon Mac cannot virtualize x64 Windows natively.

Short VM smoke pass:

1. Launch the x64 executable, test both **Run once** and **Install**, and approve first-run virtual-camera registration.
2. Confirm Windows Camera lists **StageSwap**.
3. Verify screen capture, **Screen** mode, the configured missing-source fallback, and the branded off screen after stopping automation.
4. Verify webcam capture and **Webcam** mode if the VM exposes a camera.
5. Verify **Automatic** mode, tray/close behavior, restart actions, update handoff, cleanup, and uninstall.

Before the manual pass, run the ignored interactive tests on the native target:

```powershell
cargo test -p stageswap-media-source --target x86_64-pc-windows-msvc -- --test-threads=1
cargo test -p stageswap-windows --target x86_64-pc-windows-msvc -- --ignored --test-threads=1
cargo test -p stageswap --target x86_64-pc-windows-msvc -- --ignored --test-threads=1
```

The complete retained workflow, timing requirements, compatibility checks, and deliberate exclusions live in [Acceptance tests](docs/ACCEPTANCE_TESTS.md).

## Technical references

- [Architecture](docs/ARCHITECTURE.md) — runtime pipeline, deployment, source selection, monitoring, and recovery boundaries
- [Acceptance tests](docs/ACCEPTANCE_TESTS.md) — native Windows workflow and compatibility checks
- [Release gates](docs/RELEASE_GATES.md) — mandatory publication evidence
- [Requirements traceability](docs/REQUIREMENTS_TRACEABILITY.md) — requirements mapped to implementation and evidence
- [Rust rewrite scope](docs/RUST_REWRITE_SCOPE.md) — locked product boundaries and dependency decisions
