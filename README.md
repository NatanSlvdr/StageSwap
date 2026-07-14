# Automatic Screen Camera

Automatic Screen Camera is a local-only Windows desktop application that exposes a virtual camera and crossfades it between a selected camera source and a tracked monitor. A saved reference image controls the automatic choice: while the reference is visible the camera is shown; when it disappears the tracked monitor is shown.

Prebuilt Windows executables are available from the [latest GitHub release](https://github.com/NatanSlvdr/WebcamSwitcher/releases/latest). Download one of these files and its matching `.sha256` checksum:

- `AutomaticScreenCamera-*-windows-x64-setup.exe` is the normal installer. It adds a Start-menu shortcut and a Windows uninstall entry.
- `AutomaticScreenCamera-*-windows-x64-portable.exe` is a single-file launcher with no shortcuts or uninstall entry. On first launch it requests administrator permission to retain and register only its embedded camera-source DLL under Program Files. Later launches run without elevation while that payload remains valid.

The portable EXE can be moved or deleted independently. Before deleting it permanently, exit the tray application and clean up the registered payload:

```powershell
.\AutomaticScreenCamera-0.1.0-windows-x64-portable.exe --cleanup-portable
```

Portable and installed editions intentionally cannot coexist because Windows registers one media-source CLSID. Setup automatically migrates an existing portable deployment; the portable launcher refuses to overwrite an installed deployment.

The Media Foundation virtual-camera API requires Windows 11 build 22000 or later. Release confidence is intentionally narrower: it applies only to the exact current Windows 11 x64, Zoom, webcam, driver, and artifact versions recorded by the release gate. The control plane is platform-neutral and independently tested; Windows adapters provide Media Foundation camera input, Windows Graphics Capture, Direct3D compositing, and the tray UI.

## Build

On Windows, install Visual Studio 2022 with the Desktop development with C++ workload and Windows SDK `10.0.22621.0`, then run:

```powershell
cmake --preset windows-x64-release
cmake --build --preset windows-x64-release --parallel
ctest --preset windows-x64-release
```

Use the corresponding `windows-x64-debug` presets for a warning-clean Debug build. Both Windows presets use the static MSVC runtime and treat warnings as errors. CI also runs the portable core tests with AddressSanitizer/UndefinedBehaviorSanitizer and runs the concurrency suite with ThreadSanitizer.

The application stores configuration, reference images, and rotating logs beneath `%LocalAppData%\AutomaticScreenCamera`. Nothing is uploaded and frames are never recorded.

For a source-tree developer install, install the Release build from an elevated PowerShell window:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\install.ps1 -StartWithWindows
```

The developer installer defaults to the Release preset output directory. To build the two user-facing EXEs, install Inno Setup 6 and run:

```powershell
.\scripts\package.ps1 -Version 0.1.0
```

This creates the versioned portable and Setup EXEs plus a metadata-bearing SHA-256 sidecar for each. PDB symbols, test results, and release-verification tools remain CI artifacts and are not included in either user-facing executable. Set `ISCC_PATH` when `ISCC.exe` is not discoverable automatically.

Maintainers publish a downloadable GitHub Release by pushing a semantic-version tag. The workflow builds and tests that exact commit before creating the release, and derives the package version from the tag:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

Elevation is needed only to place and register the Media Foundation source DLL where Windows Frame Server can load it. The tray application itself runs unelevated. Remove an installed release through Windows Settings; `scripts\uninstall.ps1` remains available for source-tree developer installs. User configuration and logs are intentionally retained.

See `docs/REQUIREMENTS_TRACEABILITY.md` for specification coverage, `docs/RELEASE_GATES.md` for the evidence required before making a reliability claim, and `docs/ACCEPTANCE_TESTS.md` for the physical-device procedure.

## Safety defaults

- Missing references and failed capture components select the camera or a generated placeholder, never an arbitrary screen.
- The cursor is excluded from screen capture by default.
- Stopping automation transitions away from screen capture.
- Saved device identifiers are retained while devices are unavailable.
- Monitor reassignment requires repeated, unambiguous evidence.
