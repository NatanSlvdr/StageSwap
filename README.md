<p align="center">
  <img src="crates/app/assets/app-icon.png" width="120" alt="StageSwap app icon">
</p>

<h1 align="center">StageSwap</h1>

StageSwap is a free Windows 11 virtual camera that automatically switches Zoom between a webcam and the secondary screen used for JW Library presentations.

> [!IMPORTANT]
> StageSwap is an independent, unofficial project. It is not affiliated with or endorsed by the publisher of JW Library. The name JW Library is used only to describe compatibility.

## User guide

The [StageSwap wiki](https://github.com/NatanSlvdr/StageSwap/wiki) covers installation, setup, meetings, dashboard controls, settings, troubleshooting, updates, and privacy.

## How it works

In **Auto** mode, StageSwap compares the selected secondary screen with a saved picture of its normal idle view:

- idle view matches → Zoom receives the webcam;
- the screen changes because media is playing → Zoom receives the secondary screen; and
- automatic switching is stopped → the StageSwap off screen is published.

StageSwap watches the screen visually. It does not control or read JW Library, transmit audio, record frames, upload video, or start Zoom's native screen-sharing mode. The virtual-camera output is fixed at 1280×720 and 30 fps.

## Requirements

- 64-bit Windows 11;
- a webcam;
- a secondary screen for JW Library presentations; and
- Zoom with camera permission.

Download the latest build from the [official releases page](https://github.com/NatanSlvdr/StageSwap/releases/latest). Current releases are unsigned.

## Development

StageSwap is a Rust workspace. Build, test, packaging, and architecture information is in [AGENTS.md](AGENTS.md), [Architecture](docs/ARCHITECTURE.md), and [Requirements traceability](docs/REQUIREMENTS_TRACEABILITY.md).
