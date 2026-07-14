# Architecture

Automatic Screen Camera deliberately separates policy, the trusted renderer, and the Windows camera boundary.

```text
Camera via Media Foundation ───────┐
                                  ├─ D3D11 compositor ─ ACL-restricted named pipe ─ MF camera-source DLL ─ Windows Frame Server
Monitor via Graphics Capture ─────┘
                 │
                 └─ 160×90 CPU thumbnail ─ similarity/debounce ─ decision and transition controllers
```

Only the final composited BGRA frame crosses the process boundary. Raw camera and monitor textures remain in the tray process. The cross-session pipe is named per Windows user SID, grants access only to that user plus the Frame Server service identities, and leaves no frame file on disk. If the app disappears or stops publishing for two seconds, the Media Foundation source generates a neutral placeholder instead of replaying an old private screen frame.

## Components

- `asc_core`: platform-neutral configuration, similarity, debounce, monitor selection, decision, transition, orchestration, and logging.
- `AutomaticScreenCamera.exe`: Win32 tray UI, Media Foundation input, Windows Graphics Capture, D3D11 compositor, monitoring workers, recovery, and final-frame publisher.
- `AutomaticScreenCameraSource.dll`: COM `IMFActivate` plus live `IMFMediaSourceEx`/`IMFMediaStream2`, loaded by Windows Frame Server. It advertises RGB32 at 1920×1080p30 and 1280×720p30.

The installed release places both binaries under Program Files. The portable release embeds the same source DLL into the tray EXE and extracts only that DLL to an administrator-protected Program Files directory before registration. The portable EXE itself runs from its original location and remains unelevated. A machine-wide deployment marker prevents portable and installed copies from replacing each other's COM registration.

## Threading

- The UI thread owns the window, tray icon, menus, settings, and Windows lifecycle messages.
- The compositor worker runs at output cadence and is independent of detection.
- Fast detection reads only a downscaled copy of the tracked monitor.
- The rediscovery worker captures every monitor only at the configured low-frequency interval or after a display/lifecycle event.
- Media Foundation camera callbacks retain only the newest frame, preventing unbounded queues.

## Failure invariants

1. A screen is selected only after the detector confirms absence of the reference.
2. Missing, ambiguous, or failed reference capture defaults to the camera.
3. A requested unavailable screen falls back to the camera, then the placeholder.
4. A fade never targets an invalid input; invalid textures are replaced before rendering.
5. Stopping automation forces the camera target while the compositor and virtual camera stay alive.
6. Monitor changes require a threshold, a score margin, and repeated scans.
7. Configuration replacement is atomic and the previous valid file is retained.
