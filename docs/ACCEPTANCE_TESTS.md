# Acceptance tests

Run the full retained workflow in a Windows 11 x64 VM:

1. Launch the matching executable, approve first-run deployment, relaunch without elevation, then verify `--cleanup`.
2. Keep the installed DLL loaded in Windows Camera, launch a newer build, and verify deployment switches to its content-versioned DLL without an access-denied failure. Confirm the locked old copy is removed after reboot.
3. Confirm the mismatched package is rejected and cannot register its camera-source DLL.
4. Select the single expected webcam, relaunch, and verify that the saved device opens as fixed 720p30 RGB32 capture.
5. Select the current monitor, relaunch, and verify its friendly label restores the same display. Remove or rename that display and verify the first secondary display is selected, with the sole primary used on a single-monitor system.
6. Capture and import references, Rescan, and verify a new highest-scoring monitor is selected only after the immediate confirmation scan and becomes the persisted selection.
7. Disable Automatic display rescans and verify startup, 30-second, reference-capture, and reference-import scans do not run; verify explicit Rescan and its confirmation pass still work. Re-enable the setting and verify an immediate scan runs.
8. Exercise Automatic switching both ways, Force Webcam, Force Screen, and reversal during the 500 ms fade.
9. Verify the versioned main-window title, shared window/tray icon, sectioned dashboard controls, branded Settings sidebar, contextual Settings previews, cursor option, startup preferences, 14-day logs, and all four manual restart actions. Confirm both left- and right-clicking the tray icon open its menu without raising the window; then confirm Open StageSwap opens the dashboard, Settings opens the Settings view, and the synchronized tray controls remain functional. Confirm the persisted missing-source fallback still behaves correctly; it no longer has a Settings editor.
10. In Windows Camera and Zoom, verify virtual-camera enumeration and preferred RGB32 1280×720 output at 30 fps. Confirm NV12 720p remains selectable and renders correctly when explicitly negotiated.
11. Stop automation and verify that the dashboard output and virtual-camera consumer both show the same black frame with the centered StageSwap app icon. Terminate the tray application and verify that the consumer continues showing that off frame. Repeat switching without functional failure.
12. In a release build, measure at least 300 post-warm-up frames for Force Webcam, Force Screen, Automatic, and a reversed transition. Require monotonic sample timestamps, at least 29 fps average wall-clock delivery, and no capture gap over 100 ms. Leave each of Windows Camera and Zoom running for several minutes and confirm the displayed rate remains 30 fps.
13. Enable and disable the optional webcam crop and confirm the same centered crop feeds both the preview and virtual-camera output.
14. Record Task Manager CPU for 60 seconds with the dashboard open and with the app hidden in the tray. Confirm runtime-owned FPS remains populated while hidden. Compare CPU with the previous build; it must improve materially, but there is no fixed percentage gate.
15. Install StageSwap beside Automatic Screen Camera. Confirm both cameras enumerate independently, neither can be selected as StageSwap's physical input, and StageSwap cleanup leaves the other product's files, data, registry state, startup value, and camera registration unchanged.

Run one smoke pass on physical x64 Windows hardware: launch, webcam capture, monitor capture, automatic switching, Windows Camera, Zoom, and manual restart.

Before the manual pass, run the ignored interactive test set on the native target:

```powershell
cargo test -p stageswap-media-source --target <native-target> -- --test-threads=1
cargo test -p stageswap-windows --target <native-target> -- --ignored --test-threads=1
cargo test -p stageswap --target <native-target> -- --ignored --test-threads=1
```

These commands exercise COM activation and source/stream state, 300 screen frames with cursor on/off, stop invalidation, repeated physical-webcam start/stop, virtual-camera restart, all four runtime restart actions, and interactive tray creation. Run the UI visual comparison against the retained C++ screenshots at both 100% and 150% DPI.

Explicitly exclude webcam unplug/replug, continuous device rediscovery, docking, display reordering, resolution changes, vendor-driver compatibility, camera contention, sleep/resume, GPU recovery, and performance claims beyond the 1280×720-at-30-fps checks above. Those cases may fail until the user relaunches the application or selects the source again.
