# Acceptance tests

Run the full retained workflow in Windows 11 x64 and ARM64 VMs:

1. Launch the matching portable executable, approve first-run deployment, relaunch without elevation, then verify `--cleanup-portable`.
2. Confirm the mismatched package is rejected and cannot register its camera-source DLL.
3. Select a webcam and verify fixed 720p30 RGB32 capture.
4. Select the current monitor, capture and import references, Rescan, and verify a new highest-scoring monitor is selected only after the immediate confirmation scan.
5. Exercise Automatic switching both ways, Force Webcam, Force Screen, and reversal during the 500 ms fade.
6. Verify the main window, tray behavior, four previews, cursor option, placeholder color, startup preferences, 14-day logs, and all four manual restart actions.
7. In Windows Camera and Zoom, verify virtual-camera enumeration and 720p/1080p RGB32/NV12 negotiation.
8. Terminate the tray application and verify placeholder output. Repeat switching without functional failure; no performance threshold applies.

Run one smoke pass on physical x64 and ARM64 Windows hardware: launch, webcam capture, monitor capture, automatic switching, Windows Camera, Zoom, and manual restart.

Explicitly exclude hot-plug, docking, display reordering, resolution changes, vendor-driver compatibility, camera contention, sleep/resume, GPU recovery, and performance measurements.
