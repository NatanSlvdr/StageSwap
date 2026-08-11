<p align="center">
  <img src="crates/app/assets/app-icon.png" width="120" alt="StageSwap app icon">
</p>

<h1 align="center">StageSwap</h1>

StageSwap is a free Windows 11 tool that automatically switches Zoom between a webcam and the secondary screen used by JW Library during congregation meetings.

The virtual-camera output is fixed at 1280×720 and 30 fps. StageSwap watches the selected display visually; it does not integrate with or control JW Library. It does not start Zoom's native screen-sharing mode and does not transmit audio.

> [!IMPORTANT]
> StageSwap is an independent, unofficial project and is not affiliated with or endorsed by the publisher of JW Library. The name JW Library is used only to describe compatibility.

## How Automatic mode works

Auto mode uses a saved **reference image** of JW Library with no media playing—the view with centered text and a gray square in the corner—to decide what Zoom shows.

<p align="center"><strong>No media in JW Library → 📷 Zoom shows the webcam &nbsp;&nbsp;|&nbsp;&nbsp; Media detected → 🖥️ Zoom shows the secondary screen</strong></p>

StageSwap compares the secondary screen with the reference image four times per second. It only measures visual similarity; it does not read titles or text and has no direct connection to JW Library.

With the default settings:

- A changed display is recognized after about **0.75 seconds**.
- A returned reference is confirmed after about **1.25 seconds**.
- Each change uses a reversible **0.5-second fade**.

Waiting for several checks prevents a cursor, animation, or single unusual frame from causing a switch. **Required similarity** controls how closely the screen must resemble the reference image.

If no usable reference image is available, Auto mode stays on the webcam. Stopping automatic switching keeps the virtual camera available but replaces its output with the black StageSwap off screen.

## Output modes

| Mode | Behavior while automation is running |
|:---|:---|
| **Auto** | Shows the webcam when no media is playing and the secondary screen when media is detected |
| **Camera** | Keeps the selected webcam visible and ignores the reference image |
| **Screen** | Keeps the selected secondary screen visible and ignores the reference image |

Camera and Display are manual overrides. They remain selected until another mode is chosen and use the same fade as Automatic mode.

## Set up StageSwap

StageSwap requires a 64-bit Windows 11 computer, a webcam, a secondary screen configured for JW Library presentations, and Zoom.

1. Download `StageSwap_win64_vX.Y.Z.exe` from the [official releases page](https://github.com/NatanSlvdr/StageSwap/releases/latest).
2. Open it and choose **Install StageSwap**, or choose **Run once** to try it without copying the app to your computer.

<p align="center"><em>First launch lets you install StageSwap or run the downloaded copy once.</em></p>

3. Approve the administrator prompt. This is required only to add or update the virtual camera; normal launches run without administrator access.
4. On a fresh installation, follow the five-step full-page guided setup. It explains the JW Library-to-Zoom signal path, lets you choose the webcam and secondary screen, captures the reference image, and prepares Zoom. You can choose **Set up later** or reopen it under **Settings → General → Open guided setup**.
5. Confirm the webcam and secondary screen used by JW Library.

<p align="center"><em>The dashboard shows the four previews, component status, output mode, and automation controls.</em></p>

6. Open the JW Library presentation on the secondary screen. Leave it on the normal view with no media playing, centered text, and the gray square in the corner; then select **Capture reference image** and confirm the captured frame. An existing reference image can also be imported.

<p align="center"><em>Reference image settings capture or import the saved image and adjust required similarity.</em></p>

7. Select **StageSwap** as the camera in Zoom.
8. Select **Start automatic switching**.

> [!NOTE]
> Current releases are unsigned, so Windows may show a security warning. Continue only when the file came from the official StageSwap releases page.

## Features

### Switching and output

- 🔄 **Automatic switching** shows the webcam when no media is playing and the secondary screen when media is detected.
- 🎛️ **Manual modes** force the camera or display when the operator needs direct control.
- 👁️ **Four previews** show the webcam, secondary screen, reference image, and Zoom output.

### Displays and operation

- 🖥️ **Display discovery** looks for the reference image at startup, when Settings opens, after reference changes, and every 30 seconds by default without interrupting the 30 fps output loop. Camera-list refreshes are also performed in the background.
- 🩺 **Black-or-unavailable recovery** checks only the selected display every 30 seconds and restarts capture after two consecutive nearly-black or missing-frame checks. An unchanged but visible JW Library screen remains ready while its capture session is alive; processing errors and closed sessions clear readiness immediately.
- 🛠️ **Recovery controls** restart the camera, display capture, virtual camera, or the complete pipeline.
- 🔒 **Local processing** keeps frames on the computer and does not record or upload them.

StageSwap can include or hide the mouse cursor, intelligently crop non-16:9 camera signals while leaving native 16:9 video unchanged, start minimized, begin automation on launch, and continue running when the dashboard is closed to the system tray.

Webcam capture prefers RGB32 1280×720 at 30 fps and can normalize compatible RGB32, NV12, YUY2, or MJPEG camera modes—including missing default stride/sample-size metadata, padded rows, and common 4:3 or non-30-fps inputs—into the fixed local 720p30 output. HDR/10-bit secondary displays are detected but are not tone-mapped; disable HDR on the selected display before automatic matching or reference capture.

## Everyday controls

- **Start automatic switching** makes the selected output mode live.
- **Stop automatic switching** shows the StageSwap off screen without removing the virtual camera.
- **Camera**, **Display**, and **Automatic** can be selected from the dashboard or tray menu.
- The tray **Recovery** submenu can rescan displays, restart screen capture, restart the virtual camera, restart all components, or open the confirmation-gated reference capture flow.
- **Capture reference image** captures the current JW Library idle view for review and saves it only after confirmation.
- **Rescan screens** searches the connected displays for the saved reference. It does not restart screen capture.
- **Open guided setup** under General Settings repeats the interactive webcam, secondary-screen, reference-image, and Zoom setup at any time.
- Closing the dashboard to the tray keeps capture and output running. Fully exiting stops capture; camera apps receive the StageSwap off screen.

## Privacy

Camera and display frames are processed locally and are not recorded or uploaded. Settings, the reference image, and 14-day diagnostic logs are stored under `%LocalAppData%\StageSwap`.

Anything visible on the selected secondary screen can appear in Zoom while Auto or Screen mode is active.

## Updates and removal

To update, open a newer downloaded build and confirm the replacement. The installed copy is updated and the existing settings and reference are retained.

To remove StageSwap, exit it from the system tray, open PowerShell in the folder containing the downloaded executable, and run:

```powershell
.\StageSwap_win64_vX.Y.Z.exe --uninstall
```

This removes the installed app, shortcuts, startup entry, and virtual camera. Settings, references, and logs are retained.

## Troubleshooting

- **The camera or display stopped after being unplugged, reconnected, or waking from sleep:** for a previously selected display whose capture closed, enable **Automatically fix screen capture problems**; otherwise select the source again or restart it under **Settings → Diagnostics**.
- **Screen capture stays black or unavailable after an HDMI splitter or screen change:** enable **Automatically fix screen capture problems**, or use **Restart screen capture** under **Settings → Diagnostics**.
- **Auto mode switches at the wrong time:** return JW Library to the normal screen with no media playing, capture the reference image again, and adjust **Required similarity**.
- **The saved display is missing:** recovery retries the stored selected monitor, so reconnecting that same display identity can restore capture automatically. If Windows assigns a different identity, use **Rescan screens** or choose another display. StageSwap does not provide a general hot-plug, docking-recovery, or monitor-reselection system.
- **A camera is missing:** refresh the camera list and select it again. StageSwap does not silently substitute another camera.

<p align="center"><em>Diagnostics shows component health and provides individual and complete restart controls.</em></p>

Build, test, and architecture information is in the [engineering guide](AGENTS.md).
