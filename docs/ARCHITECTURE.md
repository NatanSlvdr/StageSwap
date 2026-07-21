# Architecture

`windows-x64-portable.exe` is a native, self-deploying application. It embeds an x64 Media Foundation source DLL. First launch validates native Windows architecture, elevates only to extract and register that DLL, and then runs the tray application unelevated. Each distinct payload is extracted to a content-versioned DLL filename before COM registration switches to it, so an older DLL locked by a camera process never blocks an upgrade. Unlocked stale copies are removed immediately and locked copies are scheduled for deletion at reboot. `--cleanup-portable` unregisters and removes the payload. There is no Setup, uninstall entry, migration path, or cross-architecture deployment.

The application pipeline is fixed at 1280×720 BGRA and 30 fps:

1. Windows Graphics Capture receives a transient D3D texture and immediately copies it to a staging texture and CPU `Frame`.
2. Media Foundation video processing requests webcam RGB32 at 1280×720, 30 fps into a CPU `Frame`.
3. CPU code generates 160×90 grayscale comparison images, performs aspect-fit scaling with black letterboxing, rejects stale frames, and blends the live screen and webcam using the transition mix.
4. The CPU frame is written directly to the per-user named pipe.
5. The Media Foundation source reads the pipe and exposes RGB32 1280×720 at 30 fps with placeholder output. NV12 720p is a compatibility fallback only if Windows Camera or Zoom rejects RGB32; 1080p is excluded without evidence.

The executable also embeds an `asInvoker`, Per-Monitor-V2 Windows manifest and version resource. First launch and `--startup` verify the native architecture and installed payload. Every launch first checks for and removes the legacy portable virtual camera, COM registration, deployment marker, payload directory, and startup entry; elevation is requested only when machine-wide legacy state or current payload registration needs changing. Cleanup leaves configuration, references, and logs intact.

The webcam selection model is deliberately small. Configuration stores one Media Foundation symbolic link. The current implementation opens that exact identifier at startup and when settings change; it does not poll for webcams or replace a disconnected device automatically. A relaunch or an explicit selection/restart is the supported recovery path.

Automatic detection checks the selected screen every 250 ms. Five matches select webcam; three mismatches select screen. Missing reference or invalid capture selects webcam, unavailable screen falls back to webcam, and unavailable webcam produces the placeholder.

Monitor descriptors exist only at runtime: GDI display name, friendly label, geometry, and `HMONITOR`. Startup selects the primary display. A full scan runs immediately, every 30 seconds, and on Rescan to find the display containing the saved visual reference. The highest score above the threshold must win twice; the confirmation scan is requested immediately. This scan is application behavior, not a general hot-plug subsystem. No EDID, history, score margin, ambiguity logic, display ordering, or persisted monitor identity exists.

There is no background recovery worker or webcam hot-plug recovery. Users can manually restart each retained component or all components. Device-change and DPI messages do not trigger video recovery; D3D device removal requires application relaunch.

The deliberately excluded behavior is: continuous webcam enumeration, unplug/replug recovery, sleep/resume recovery, GPU-device recreation, camera-contention handling, persisted physical monitor identity, and dynamic input-format management. See [the Rust rewrite scope](RUST_REWRITE_SCOPE.md) for the boundary between reusable crates and the Windows-specific virtual-camera code.
