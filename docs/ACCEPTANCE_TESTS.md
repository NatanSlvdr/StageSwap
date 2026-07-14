# Windows acceptance tests

Run these on the exact Windows 11 x64, Zoom, webcam, driver, SDK, and artifact versions named in the release report, with camera privacy access enabled. Build 22000 is the API minimum, not the breadth of the reliability claim. Test the versioned Setup and portable EXEs separately as the target user.

## Distribution lifecycle

1. Verify each EXE against its matching `.sha256` sidecar and confirm the sidecar identifies the expected version, revision, Release configuration, x64 architecture, and SDK.
2. Launch the portable EXE on a clean machine. Accept UAC once, verify only `AutomaticScreenCameraSource.dll` is retained under `%ProgramFiles%\Automatic Screen Camera Portable`, and confirm the tray process is not elevated.
3. Exit and relaunch the same portable EXE. Verify there is no UAC prompt. Move the EXE and repeat.
4. Launch a newer portable build while the old tray app is running and verify the update refuses until the tray exits. After exit, verify atomic payload replacement and successful camera enumeration.
5. Run `portable.exe --cleanup-portable`; verify it refuses while the tray app is active, then removes the virtual camera, COM registration, deployment marker, and protected DLL after the app exits while retaining LocalAppData.
6. From a standard-user account, run Setup and supply credentials for a different administrator. Verify the Start-menu and Programs entries, unchecked Start-with-Windows default, optional unelevated post-install launch, and that enabling Start with Windows creates the entry for the original standard user rather than the administrator.
7. Start uninstall from Windows Settings while signed in as that standard user, then cancel UAC and cancel the uninstall confirmation in separate runs. Verify the virtual camera and startup entry remain unchanged. Complete uninstall and verify the original user's virtual camera and startup entry are removed, machine files are removed, LocalAppData is retained, and restart is requested only after current-user cleanup. Repeat with camera privacy disabled and verify a failed virtual-camera cleanup does not prevent machine-file removal.
8. With portable mode active, start Setup and cancel on every page before installation. Then force an installation failure before the post-install step. In each case verify the portable DLL, COM registration, deployment marker, virtual camera, and startup entry remain usable and unchanged.
9. Complete the portable-to-Setup migration. Verify Setup requires a restart, does not offer the post-install launch, and does not remove portable files until the installed files and COM registration have committed; it must then remove or schedule removal of the obsolete portable DLL. With installed mode active, verify the portable launcher refuses to replace it.
10. With the patched tray running in another Fast User Switching or RDP session, verify Setup and uninstall refuse to proceed. Exit every pre-v2 tray in all sessions before an upgrade. During Setup and uninstall, repeatedly try to launch both installed and portable EXEs; verify current launchers cannot cross the deployment transaction and that no process blocked behind uninstall starts after the installed marker is removed.

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
4. Press Stop while screen output is active. Verify the output fades to camera or placeholder, the tracked-screen preview becomes unavailable, the Windows Graphics Capture session closes, and background recovery does not reopen it until Start is pressed.
5. Terminate the tray process while a consumer is open. Verify the camera source changes to its generated placeholder rather than replaying stale frames.

## Recovery and persistence

1. Disconnect/reconnect the camera. Verify warning, live virtual camera, placeholder/camera fallback, and successful manual restart.
2. Sleep and resume, then lock and unlock. Verify recovery waits briefly for devices to return, coalesces the resulting event storm, and records `RECOVERY_STARTED` followed by `RECOVERY_SUCCEEDED` or actionable `RECOVERY_FAILED` component states before the full reference scan.
3. Corrupt `config.json`; preserve a valid `config.backup.json`. Verify the invalid file is copied to `config.invalid.json`, the backup loads, and a warning is shown/logged.
4. Run for 24 hours while sampling private bytes, GPU memory, handle count, and working set. Confirm no sustained growth and continuing 30-second scans.

## Dashboard and settings usability

1. Open the dashboard at 100%, 150%, and 200% Windows scaling. Resize it to its minimum size and verify the output preview, current output, mode, reference, tracked display, health, actions, and three activity rows remain readable without overlap.
2. Hover the output, reference, display, and health summaries and verify their technical tooltips match the expanded Technical details text. Reach each information button by keyboard and verify the same tooltip appears on focus.
3. Exercise Automatic, Force webcam/video, and Force screen. Verify manual override is always visible, Return to Automatic remains available, and warnings replace neither the current-output label nor the safe-placeholder state.
4. Expand Technical details and verify all former dashboard metrics and all 20 recent events remain selectable. Use View all and verify focus moves to the full activity list.
5. Hide and minimize the dashboard while monitoring preview calls; verify the dashboard output preview stops refreshing. Restore it and verify refresh resumes at no more than once per second.
6. Open Settings and navigate General, Sources, Detection, Output, and Advanced & diagnostics using only the keyboard. Verify Save and Cancel stay visible on every page.
7. Expand Sources device details and verify the stable identifier and supported formats appear without covering later controls. Collapse it and verify the compact connection status remains visible.
8. Change one setting on each page, save, reopen Settings, and verify every value persisted. Repeat, choose Cancel, and verify none of the pending values changed.
9. Enable Windows high contrast and repeat the dashboard status checks. Verify every colored state remains paired with readable text and focus remains visible.

## Timing measurements

- Output cadence: 30 fps with monotonic 100 ns sample timestamps.
- Fast check: configured 250 ms default.
- Loss confirmation: three checks; restore confirmation: five checks.
- Fade: 500 ms default, approximately 15 frames at 30 fps.
- End-to-end reference loss to completed switch: normally under 1.5 seconds.
