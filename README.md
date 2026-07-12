# Automatic Screen Camera

Automatic Screen Camera is a local-only Windows desktop application that exposes a virtual camera and crossfades it between a selected camera source and a tracked monitor. A saved reference image controls the automatic choice: while the reference is visible the camera is shown; when it disappears the tracked monitor is shown.

The implementation targets Windows 11 build 22000 or later because it uses the Windows Media Foundation virtual-camera API. The control plane is platform-neutral and independently tested; Windows adapters provide Media Foundation camera input, Windows Graphics Capture, Direct3D compositing, and the tray UI.

## Build

On Windows, install Visual Studio 2022 with the Desktop development with C++ workload and a recent Windows 11 SDK, then run:

```powershell
cmake -S . -B build -G "Visual Studio 17 2022" -A x64
cmake --build build --config Release
ctest --test-dir build -C Release --output-on-failure
```

The application stores configuration, reference images, and rotating logs beneath `%LocalAppData%\AutomaticScreenCamera`. Nothing is uploaded and frames are never recorded.

Install the Release build from an elevated PowerShell window:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\install.ps1 -BuildDirectory .\build -StartWithWindows
```

Elevation is needed only to place and register the Media Foundation source DLL where Windows Frame Server can load it. The tray application itself runs unelevated. Uninstall with `scripts\uninstall.ps1`; user configuration and logs are intentionally retained.

See `docs/REQUIREMENTS_TRACEABILITY.md` for specification coverage and the exact remaining Windows verification gates. The physical-device procedure is in `docs/ACCEPTANCE_TESTS.md`.

## Safety defaults

- Missing references and failed capture components select the camera or a generated placeholder, never an arbitrary screen.
- The cursor is excluded from screen capture by default.
- Stopping automation transitions away from screen capture.
- Saved device identifiers are retained while devices are unavailable.
- Monitor reassignment requires repeated, unambiguous evidence.
