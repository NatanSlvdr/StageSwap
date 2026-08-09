# StageSwap repository guidance

Use this guide for engineering, build, test, packaging, deployment, and release work on StageSwap. The main [README](README.md) is the product presentation and end-user guide.

## Product contract

StageSwap is a local-only, native Windows 11 x64 virtual camera for automatic Zoom retransmission during congregation meetings using JW Library. It watches the second display visually and does not integrate with or control JW Library.

- Frames are immutable CPU BGRA and the internal/output contract is fixed at 1280×720, 30 fps.
- Webcam input uses Media Foundation tiered negotiation: it prefers progressive RGB32 1280×720 at 30 fps, safely falls back through compatible RGB32, NV12, YUY2, and MJPEG modes, and normalizes row-aware input into the fixed BGRA contract. Native-aspect-aware centered 16:9 cropping leaves native 16:9 and unknown-aspect signals unchanged. Screen input uses Windows Graphics Capture. Cropping feeds both preview and output.
- The runtime uses synchronous bounded channels and no Tokio.
- Reference detection runs every 250 ms with five-match/three-mismatch debounce.
- Monitor selection is restored by friendly label with secondary-display fallback. Reference discovery requires the same winning display twice, does not pause output, runs at startup, when Settings opens, after reference changes, and every 30 seconds by default, and can be limited to explicit rescans.
- Independent black-or-unavailable recovery samples only the selected capture every 30 seconds. Two consecutive near-black or missing-frame checks restart screen capture; the setting can be disabled without changing reference discovery.
- Composition uses CPU aspect-fit, black letterboxing, configurable missing-source fallback, and a reversible 500 ms blend.
- The virtual camera prefers RGB32 1280×720 at 30 fps. Selectable NV12 720p is retained for Windows Camera and Zoom compatibility; 1080p is intentionally excluded.
- Output uses deadline-based 30 fps pacing. Visible dashboard previews use latest-only conversion workers and display-sized textures. FPS is runtime-owned and remains meaningful while the dashboard is hidden.
- While automation is stopped, a fixed black screen with the centered StageSwap icon is published at 30 fps. The virtual camera generates the same frame whenever the app publisher is absent.
- The app includes the dashboard, five settings categories, contextual previews, shared executable/window/tray icon, synchronized tray controls, warning notifications, exit confirmation, 14-day JSONL logs, and individual/all component restarts.
- There is no general hot-plug manager, monitor-reselection system, sleep/resume recovery, docking recovery, dynamic output-format management, OBS integration, or kernel driver. Compatible webcam media-type changes are revalidated, but failures require an explicit webcam restart. Screen recovery only retries a selected capture that is black or unavailable, using its stored display identity.

Configuration schema 1, references, and logs live under `%LocalAppData%\StageSwap`. Frames are never recorded or uploaded.

## Repository layout

```text
StageSwap/
├── crates/core/          state machine, frames, detector, and transitions
├── crates/app/           UI, configuration, orchestration, and composition
├── crates/windows/       webcam, screen, deployment, and IPC adapters
├── crates/media-source/  Media Foundation virtual-camera source DLL
├── xtask/                packaging, PE validation, and checksums
├── scripts/              local packaging and release-evidence helpers
└── docs/                 architecture, localization, requirements, and release-evidence references
```

The workspace uses Rust edition 2024 and is pinned to Rust 1.97.1 in `rust-toolchain.toml`. `stageswap-core` and `xtask` are the default members so platform-independent work can be built and tested on non-Windows hosts. Direct Windows APIs, COM, and unavoidable unsafe code belong in `stageswap-windows` and `stageswap-media-source`.

## Development workflow

Before pushing to `main`, run the format, lint, and host test suite:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

For cross-target linting, run:

```bash
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

### UI preview

Launch the real StageSwap Settings interface with deterministic mock cameras, displays, reference imagery, and healthy runtime states:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview
```

General opens by default. To open a particular page:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview matching
```

Available page names are `general`, `webcam`, `screen`, `matching`, and `diagnostics`. You can still navigate between every page after launch.

Setup-guide previews are `setup-1` through `setup-5`. Each name opens one deterministic full-window step for the JW Library-to-Zoom workflow.

For the idle-reference step, append `--ui-setup-reference-state captured`, `empty`, `review`, or `missing-screen` to inspect its deterministic saved, initial, candidate-review, and unavailable-display states:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview setup-4 --ui-setup-reference-state review
```

The `setup-4` capture, retake, confirmation, and capture-again controls are interactive in preview mode. They update only the in-memory mock frames and never write `reference.png`.

Dialog previews are `dialog-exit`, `dialog-clear-logs`, `dialog-admin`, `dialog-replace-baseline`, `dialog-load-admin-config`, and `dialog-remove-baseline`.

Append `--ui-language en-US`, `--ui-language fr-FR`, or `--ui-language es` to render any preview in a supported language:

```bash
cargo run -p stageswap --bin StageSwap -- --ui-preview matching --ui-language fr-FR
```

Preview mode uses a temporary configuration directory and does not save changes to the normal StageSwap configuration.

## Platform limitations

Development is performed on macOS. Host checks, deterministic UI previews, and cross-compilation are available locally, but macOS cannot validate an interactive Windows desktop, physical webcam capture, virtual-camera enumeration, native Windows dialogs/tray behavior, exact Windows font rendering, or hardware-specific capture behavior. Use a native Windows machine or the GitHub Actions workflow for those checks.

## Packaging and release

### Cross-compiled x64 package

```bash
./scripts/package-x64-macos.sh
```

The wrapper cross-compiles the x64 Windows build, pins the Windows SDK, embeds the matching Media Foundation DLL and Windows resources, validates the generated PE files and payload, and writes the versioned executable and checksum to `dist/`.

If the optimized release build differs from the latest versioned artifact in `dist/`, the patch number is incremented and the selected version is persisted to `Cargo.toml` and `Cargo.lock` before the final DLL and EXE are rebuilt. Rebuilding identical bytes with an already synchronized workspace keeps the existing version. The filename, application UI, Windows version resources, and checksum metadata therefore use one version. Rust and SDK caches make later builds faster. GitHub Actions remains the authoritative release builder.

### Release and deployment

Releases are unsigned versioned executables with SHA-256 sidecars. The local package command is `cargo run --release -p xtask -- package x64 dist`.

`StageSwap_win64_vX.Y.Z.exe` self-deploys per-user or can run once without installation. `--cleanup` removes startup and the virtual-camera deployment; `--uninstall` also removes the managed app and shortcuts. Both preserve user data, and deployment never modifies Automatic Screen Camera data.

## Technical references

- [Architecture](docs/ARCHITECTURE.md) — runtime pipeline, deployment, source selection, monitoring, and recovery boundaries
- [Requirements traceability](docs/REQUIREMENTS_TRACEABILITY.md) — requirements mapped to implementation and evidence
