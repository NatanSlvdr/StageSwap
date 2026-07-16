# Architecture

`windows-x64-portable.exe` and `windows-arm64-portable.exe` are separate native, self-deploying applications. Each embeds a same-architecture Media Foundation source DLL. First launch validates native Windows architecture, elevates only to extract and register that DLL, and then runs the tray application unelevated. `--cleanup-portable` unregisters and removes the payload. There is no Setup, uninstall entry, migration path, or cross-architecture deployment.

The application pipeline is fixed at 1280×720 BGRA and 30 fps:

1. Windows Graphics Capture receives a transient D3D texture and immediately copies it to a staging texture and CPU `Frame`.
2. Media Foundation video processing requests webcam RGB32 at 1280×720, 30 fps into a CPU `Frame`.
3. CPU code generates 160×90 grayscale comparison images, performs aspect-fit scaling with black letterboxing, rejects stale frames, and blends the live screen and webcam using the transition mix.
4. The CPU frame is written directly to the per-user named pipe.
5. The Media Foundation source reads the pipe and preserves its 720p/1080p and RGB32/NV12 consumer negotiation, scaling, and placeholder output.

Automatic detection checks the selected screen every 250 ms. Five matches select webcam; three mismatches select screen. Missing reference or invalid capture selects webcam, unavailable screen falls back to webcam, and unavailable webcam produces the placeholder.

Monitor descriptors exist only at runtime: GDI display name, friendly label, geometry, and `HMONITOR`. Startup selects the primary display. A full scan runs immediately, every 30 seconds, and on Rescan. The highest score above the threshold must win twice; the confirmation scan is requested immediately. No EDID, history, score margin, ambiguity logic, display ordering, or persisted monitor identity exists.

There is no background recovery worker or lifecycle-triggered recovery. Users can manually restart each retained component or all components. D3D device removal requires application relaunch.
