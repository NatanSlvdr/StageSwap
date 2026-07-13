# Windows acceptance tests

Run these on the exact Windows 11 x64, Zoom, webcam, driver, SDK, and artifact versions named in the release report, with camera privacy access enabled. Build 22000 is the API minimum, not the breadth of the reliability claim. Install the Release build using an elevated PowerShell session, then launch the app normally as the target user.

## Functional switching

1. Select a physical camera in Settings and verify its status becomes Ready.
2. Put the desired reference on a monitor and choose **Set current screen as reference**. Verify the app hides for three seconds, logs `REFERENCE_CREATED`, and shows a live score near 100%.
3. Select the virtual camera in Windows Camera, Teams, Zoom, Discord, Chrome, and Edge.
4. Replace the reference. After three mismatches, verify a 500 ms fade to that monitor with no black frame.
5. Restore it. After five matches, verify a 500 ms fade to the camera.
6. Change state around 300 ms into a fade and verify the blend reverses from its current visual position.

## Monitor identity and ambiguity

1. Change display ordering and reboot. Verify the reference scan selects the same physical panel without relying on its display number.
2. Move the reference to another monitor. Verify three rapid confirmation scans occur and tracking changes once, with `TRACKED_MONITOR_CHANGED` logged.
3. Put the reference on two monitors, including a visually degraded copy on the tracked panel so the other panel scores higher. Verify the previous tracked panel remains selected and the UI/log reports ambiguity without oscillation.
4. Disconnect and reconnect the tracked panel. Verify a red/missing state, safe camera output, automatic rescan, and recovery.

## Modes and privacy

1. Force webcam/video, remove the reference, and verify output remains camera while Automatic target says Screen capture.
2. Force screen, restore the reference, and verify output remains screen while Automatic target says Webcam/video.
3. Return to Automatic and verify a fade to the current automatic target.
4. Press Stop while screen output is active. Verify the output fades to camera or placeholder and does not retain private screen content.
5. Terminate the tray process while a consumer is open. Verify the camera source changes to its generated placeholder rather than replaying stale frames.

## Recovery and persistence

1. Disconnect/reconnect the camera. Verify warning, live virtual camera, placeholder/camera fallback, and successful manual restart.
2. Sleep and resume, then lock and unlock. Verify recovery waits briefly for devices to return, coalesces the resulting event storm, and records `RECOVERY_STARTED` followed by `RECOVERY_SUCCEEDED` or actionable `RECOVERY_FAILED` component states before the full reference scan.
3. Corrupt `config.json`; preserve a valid `config.backup.json`. Verify the invalid file is copied to `config.invalid.json`, the backup loads, and a warning is shown/logged.
4. Run for 24 hours while sampling private bytes, GPU memory, handle count, and working set. Confirm no sustained growth and continuing 30-second scans.

## Timing measurements

- Output cadence: 30 fps with monotonic 100 ns sample timestamps.
- Fast check: configured 250 ms default.
- Loss confirmation: three checks; restore confirmation: five checks.
- Fade: 500 ms default, approximately 15 frames at 30 fps.
- End-to-end reference loss to completed switch: normally under 1.5 seconds.
