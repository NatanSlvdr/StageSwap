# Acceptance tests

Run the full retained workflow in a Windows 11 x64 VM:

1. Launch the matching portable executable, approve first-run deployment, relaunch without elevation, then verify `--cleanup-portable`.
2. Keep the installed DLL loaded in Windows Camera, launch a newer portable build, and verify deployment switches to its content-versioned DLL without an access-denied failure. Confirm the locked old copy is removed after reboot.
3. Confirm the mismatched package is rejected and cannot register its camera-source DLL.
4. Select the single expected webcam, relaunch, and verify that the saved device opens as fixed 720p30 RGB32 capture.
5. Select the current monitor, capture and import references, Rescan, and verify a new highest-scoring monitor is selected only after the immediate confirmation scan.
6. Exercise Automatic switching both ways, Force Webcam, Force Screen, and reversal during the 500 ms fade.
7. Verify the main window, tray behavior, four previews, cursor option, placeholder color, startup preferences, 14-day logs, and all four manual restart actions.
8. In Windows Camera and Zoom, verify virtual-camera enumeration and RGB32 1280×720 at 30 fps. Add and retest only NV12 720p if a rejection is proven.
9. Terminate the tray application and verify placeholder output. Repeat switching without functional failure; no performance threshold applies.

Run one smoke pass on physical x64 Windows hardware: launch, webcam capture, monitor capture, automatic switching, Windows Camera, Zoom, and manual restart.

Before the manual pass, run the ignored interactive test set on the native target:

```powershell
cargo test -p asc-media-source --target <native-target> -- --test-threads=1
cargo test -p asc-windows --target <native-target> -- --ignored --test-threads=1
cargo test -p automatic-screen-camera --target <native-target> -- --ignored --test-threads=1
```

These commands exercise COM activation and source/stream state, 300 screen frames with cursor on/off, stop invalidation, repeated physical-webcam start/stop, virtual-camera restart, all four runtime restart actions, and interactive tray creation. Run the UI visual comparison against the retained C++ screenshots at both 100% and 150% DPI.

Explicitly exclude webcam unplug/replug, continuous device rediscovery, docking, display reordering, resolution changes, vendor-driver compatibility, camera contention, sleep/resume, GPU recovery, and performance measurements. Those cases may fail until the user relaunches the application or selects the source again.
