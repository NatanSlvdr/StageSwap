# Automatic Screen Camera

Automatic Screen Camera is a local-only Windows desktop application that exposes a virtual camera and crossfades it between a selected camera source and a tracked monitor. A saved reference image controls the automatic choice: while the reference is visible the camera is shown; when it disappears the tracked monitor is shown.

Prebuilt Windows packages are available from the [latest GitHub release](https://github.com/NatanSlvdr/WebcamSwitcher/releases/latest). Download the `AutomaticScreenCamera-*-windows-x64-unsigned.zip` file and its matching `.sha256` checksum.

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

Install the Release build from an elevated PowerShell window:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\install.ps1 -StartWithWindows
```

The installer defaults to the Release preset output directory and prefers packaged binaries beside the script. A release ZIP containing the EXE, media-source DLL, PDB symbols, installer scripts, test results, and SHA-256 manifests can be produced with:

```powershell
.\scripts\package.ps1 -TestResultsDirectory .\out\test-results
```

Maintainers publish a downloadable GitHub Release by pushing a semantic-version tag. The workflow builds and tests that exact commit before creating the release, and derives the package version from the tag:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

Elevation is needed only to place and register the Media Foundation source DLL where Windows Frame Server can load it. The tray application itself runs unelevated. Uninstall with `scripts\uninstall.ps1`; user configuration and logs are intentionally retained.

See `docs/REQUIREMENTS_TRACEABILITY.md` for specification coverage, `docs/RELEASE_GATES.md` for the evidence required before making a reliability claim, and `docs/ACCEPTANCE_TESTS.md` for the physical-device procedure.

## Safety defaults

- Missing references and failed capture components select the camera or a generated placeholder, never an arbitrary screen.
- The cursor is excluded from screen capture by default.
- Stopping automation transitions away from screen capture.
- Saved device identifiers are retained while devices are unavailable.
- Monitor reassignment requires repeated, unambiguous evidence.
