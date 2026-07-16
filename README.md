# Automatic Screen Camera

Automatic Screen Camera is a local-only Windows 11 application that publishes a virtual camera. In Automatic mode it shows a selected webcam while a saved visual reference is detected on the selected monitor, and otherwise shows that monitor. Force Webcam and Force Screen provide manual overrides. Changes use a reversible 500 ms fade.

Download the executable matching the computer's native architecture:

- `windows-x64-portable.exe`
- `windows-arm64-portable.exe`

Each is a self-deploying portable executable. First launch requests administrator permission to extract and register its matching Media Foundation source DLL. Later launches do not require elevation. Cross-architecture deployment is rejected; x64 on ARM64 emulation is not supported.

Before deleting the executable, exit the tray application and clean up its registered payload:

```powershell
.\windows-x64-portable.exe --cleanup-portable
```

Use `windows-arm64-portable.exe` in that command on ARM64 Windows.

## Retained contract

- Windows 11 build 22000 or later.
- CPU BGRA pipeline fixed at 1280×720 and 30 fps after Windows Graphics Capture reads each transient D3D texture into memory.
- Webcam requested through Media Foundation as RGB32 1280×720 at 30 fps.
- CPU grayscale reference detection every 250 ms, with five matches and three mismatches.
- All monitors scanned at startup, every 30 seconds, and on Rescan. The highest score above the threshold must win two consecutive scans.
- CPU aspect-fit scaling, black letterboxing, placeholder fallback, and a reversible 500 ms blend.
- Virtual-camera consumers may negotiate RGB32 or NV12 at 720p or 1080p; the media source retains output scaling and placeholder generation.
- Manual webcam, screen-capture, virtual-camera, and all-component restarts. A removed D3D device requires relaunching the application.

Configuration, references, and 14 days of logs are stored beneath `%LocalAppData%\AutomaticScreenCamera`. Frames are not recorded or uploaded. Configuration schema v2 imports retained v1 values and drops removed settings on the next save.

## Build and package

Install Visual Studio 2022 with Desktop development with C++ and Windows SDK 10.0.22621.0.

```powershell
cmake --preset windows-x64-release
cmake --build --preset windows-x64-release --parallel
ctest --preset windows-x64-release

cmake --preset windows-arm64-release
cmake --build --preset windows-arm64-release --parallel
ctest --preset windows-arm64-release

.\scripts\package.ps1 -Architecture x64 -Version 0.1.0
.\scripts\package.ps1 -Architecture arm64 -Version 0.1.0
```

Packaging validates the PE machine type of both the executable and embedded camera-source DLL before producing the two portable artifacts and SHA-256 sidecars.

See [architecture](docs/ARCHITECTURE.md), [acceptance tests](docs/ACCEPTANCE_TESTS.md), and [release gates](docs/RELEASE_GATES.md).
