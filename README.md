<p align="center">
  <img src="crates/app/assets/app-icon.png" width="120" alt="StageSwap app icon">
</p>

<h1 align="center">StageSwap</h1>

StageSwap is a free Windows 11 tool that combines one camera and one display into a virtual camera for meetings and hybrid events. Select **StageSwap** as the camera in your meeting app, then control whether its output shows the camera or the display.

The virtual-camera output is fixed at 1280×720 and 30 fps. StageSwap does not start the meeting app's native screen-sharing mode and does not transmit audio.

## How Automatic mode works

Automatic mode uses a saved image of the display to decide which source to show. This image is called the **reference**. It can be a holding slide, event graphic, desktop background, or any other view that means the camera should be active.

<p align="center"><strong>🖼️ Reference matches → 📷 Camera &nbsp;&nbsp;|&nbsp;&nbsp; Reference changes → 🖥️ Display</strong></p>

StageSwap compares the selected display with the reference four times per second. It only looks at the visual similarity; it does not read slide titles, app names, or text.

With the default settings:

- A changed display is recognized after about **0.75 seconds**.
- A returned reference is confirmed after about **1.25 seconds**.
- Each change uses a reversible **0.5-second fade**.

Waiting for several checks prevents a cursor, animation, or single unusual frame from causing a switch. **Match strictness** controls how closely the display must resemble the reference.

If no usable reference is available, Automatic mode stays on the camera. Stopping automation keeps the virtual camera available but replaces its output with the black StageSwap off screen.

## Output modes

| Mode | Behavior while automation is running |
|:---|:---|
| **Automatic** | Uses the reference to switch between the camera and display |
| **Camera** | Keeps the selected camera visible and ignores reference changes |
| **Display** | Keeps the selected display visible and ignores reference changes |

Camera and Display are manual overrides. They remain selected until another mode is chosen and use the same fade as Automatic mode.

## Set up StageSwap

StageSwap requires a 64-bit Windows 11 computer, a camera, and the display you want to present.

1. Download `StageSwap_win64_vX.Y.Z.exe` from the [official releases page](https://github.com/NatanSlvdr/StageSwap/releases/latest).
2. Open it and choose **Install StageSwap**, or choose **Run once** to try it without copying the app to your computer.

<p align="center">
  <img src="docs/images/readme/first-launch.svg" width="760" alt="Placeholder for the StageSwap first-launch installation choice">
</p>
<p align="center"><em>First launch lets you install StageSwap or run the downloaded copy once.</em></p>

3. Approve the administrator prompt. This is required only to add or update the virtual camera; normal launches run without administrator access.
4. Choose the camera and display StageSwap should use.

<p align="center">
  <img src="docs/images/readme/dashboard.svg" width="760" alt="Placeholder for the StageSwap dashboard">
</p>
<p align="center"><em>The dashboard shows the four previews, component status, output mode, and automation controls.</em></p>

5. Show the idle view on the selected display and select **Capture reference**. An existing image can also be imported.

<p align="center">
  <img src="docs/images/readme/matching-settings.svg" width="760" alt="Placeholder for the StageSwap reference-matching settings">
</p>
<p align="center"><em>Matching settings are used to capture or import the reference and adjust match strictness.</em></p>

6. Select **StageSwap** as the camera in the meeting app.
7. Select **Start automation**.

> [!NOTE]
> Current releases are unsigned, so Windows may show a security warning. Continue only when the file came from the official StageSwap releases page.

## Features

### Switching and output

- 🔄 **Automatic switching** uses the saved reference to choose the camera or display.
- 🎛️ **Manual modes** force the camera or display when the operator needs direct control.
- 👁️ **Four previews** show the camera, display, reference, and final audience output.

### Displays and operation

- 🖥️ **Display rescanning** looks for the reference at startup, after reference changes, and every 30 seconds by default.
- 🛠️ **Recovery controls** restart the camera, display capture, virtual camera, or the complete pipeline.
- 🔒 **Local processing** keeps frames on the computer and does not record or upload them.

StageSwap can include or hide the mouse cursor, crop and center the camera to 16:9, start minimized, begin automation on launch, and continue running when the dashboard is closed to the system tray.

## Everyday controls

- **Start automation** makes the selected output mode live.
- **Stop automation** shows the StageSwap off screen without removing the virtual camera.
- **Camera**, **Display**, and **Automatic** can be selected from the dashboard or tray menu.
- **Capture reference** saves the current display view.
- **Rescan screens** searches the connected displays for the saved reference.
- Closing the dashboard to the tray keeps capture and output running. Fully exiting stops capture; camera apps receive the StageSwap off screen.

## Privacy

Camera and display frames are processed locally and are not recorded or uploaded. Settings, the reference image, and 14-day diagnostic logs are stored under `%LocalAppData%\StageSwap`.

Anything visible on the selected display can appear in the virtual-camera output while Automatic or Display mode is active.

## Updates and removal

To update, open a newer downloaded build and confirm the replacement. The installed copy is updated and the existing settings and reference are retained.

To remove StageSwap, exit it from the system tray, open PowerShell in the folder containing the downloaded executable, and run:

```powershell
.\StageSwap_win64_vX.Y.Z.exe --uninstall
```

This removes the installed app, shortcuts, startup entry, and virtual camera. Settings, references, and logs are retained.

## Troubleshooting

- **The camera or display stopped after being unplugged, reconnected, or waking from sleep:** select the source again or restart it under **Settings → Diagnostics**.
- **Automatic mode switches at the wrong time:** capture the idle reference again and adjust **Match strictness**.
- **The saved display is missing:** use **Rescan screens** or choose another display. StageSwap does not provide automatic hot-plug or docking recovery.
- **A camera is missing:** refresh the camera list and select it again. StageSwap does not silently substitute another camera.

<p align="center">
  <img src="docs/images/readme/diagnostics.svg" width="760" alt="Placeholder for the StageSwap diagnostics settings">
</p>
<p align="center"><em>Diagnostics shows component health and provides individual and complete restart controls.</em></p>

Build, test, and architecture information is in the [developer guide](DEV.md).
