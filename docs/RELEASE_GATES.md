# Windows and Zoom release gates

Passing hosted CI is necessary but does not establish the 95% reliability claim. That claim applies only to the release candidate and the environment recorded below, after every hosted and interactive gate passes without an unexplained retry.

## Release identity

Record these values in the release report before testing:

- Git commit, application version, artifact filename, and SHA-256 digest.
- Windows edition, version, OS build, and installed updates.
- Zoom version and installer channel.
- Webcam make/model, USB identifiers, driver provider, and driver version.
- Visual Studio version and Windows SDK version.
- VM image identifier, four-vCPU configuration, display driver, and USB-passthrough configuration.

Store hosted JUnit files, interactive probe JSON, screenshots, and failure classifications with the release report. Production failures reset the affected consecutive-run gate; infrastructure failures must be supported by logs or screenshots before rerunning.

## Hosted gate

Run five clean `Build, test, and package` workflows from fresh checkouts of the same commit. Every Debug, Release, and portable sanitizer job must pass. Retain the unsigned release ZIP, its external SHA-256 file, and all JUnit artifacts from each run.

## Persistent interactive Windows 11 gate

Use a persistent Windows 11 x64 VM with an interactive desktop, administrator access, a virtual display, the current stable Zoom client, and a USB-passthrough UVC webcam. Test fixtures and control hooks must remain in separate executables and must not add production interfaces.

The release candidate qualifies only after all of these pass:

- 20 clean install/uninstall or upgrade cycles, with correct COM registration and no stale virtual-camera entries.
- 100 application cold-start/exit cycles and 100 virtual-camera open/close cycles.
- 300 automatic camera/screen switches, with no black or stale output longer than two seconds.
- 50 webcam disconnect/reconnect cycles, each recovering within ten seconds.
- 20 lock/unlock, display-reset, and VM pause/resume cycles.
- 30 Zoom cold launches in which Automatic Screen Camera enumerates, can be selected, and renders the expected preview.
- Two consecutive 24-hour Zoom soaks. Toggle the deterministic reference every 60 seconds and require 29–31 fps, no crash/hang/deadlock/unrecovered device loss, no incorrect privacy fallback or stale private frame beyond two seconds, average CPU at most 25%, p95 CPU at most 50%, private-memory growth below 50 MiB, and net handle growth below 20 after warm-up.

The Media Foundation output probe must capture negotiated format, QPC-correlated timestamps, frame hashes, cadence, stale-frame duration, CPU, private memory, and handle count as machine-readable JSON. Windows UI Automation must install the application, launch Zoom, select Automatic Screen Camera, and verify the deterministic expected frames.

## Verification tools and evidence

Build the Windows Release preset and use the two executables under `out/build/windows-x64-release/tests/windows/Release` only on the test VM. They are not part of the production package.

```powershell
# Display the deterministic reference on monitor 1 and toggle it every 60 seconds.
asc_test_screen_fixture.exe --mode toggle --monitor 1 --toggle-seconds 60 --duration-seconds 86400

# Open the virtual camera independently of Zoom and record one soak as JSON.
asc_mf_output_probe.exe --duration-seconds 86400 --warmup-seconds 60 `
  --minimum-fps 29 --maximum-fps 31 --maximum-stale-ms 2000 `
  --output soak-1-mf-probe.json

# Capture the exact release environment and artifact identity.
.\scripts\collect-release-environment.ps1 `
  -ArtifactPath .\AutomaticScreenCamera-0.1.0-windows-x64-unsigned.zip `
  -OutputPath .\release-environment.json -ZoomChannel stable `
  -VmImageId 'win11-release-vm-2026-07' -UsbPassthroughId 'usb-port-3-uvc-fixture'
```

Copy `docs/release-evidence.example.json` into the evidence directory and update only counters backed by retained logs/screenshots. Give each soak a distinct run ID, round-trip UTC start/completion timestamps (for example `2026-07-13T00:00:00.0000000+00:00`), probe path, and the tested archive's SHA-256. Reference the environment report, then evaluate the full claim:

```powershell
.\scripts\evaluate-release-gates.ps1 -EvidenceManifest .\release-evidence.json `
  -OutputPath .\release-gate-report.json
```

The evaluator exits nonzero and sets `confidence_claim_allowed` to false if any threshold, media format, cadence, resource, duration, environment, privacy review, or Zoom/UI evidence is missing. The probe's `stale_frame_duration_ms` measures delivery lateness beyond one nominal frame; `maximum_unchanged_hash_ms` is diagnostic because static content is legitimate. The required privacy review must correlate the sampled QPC/hash timeline with fixture state changes and Zoom screenshots to classify visually stale output.

Run a shorter Zoom enumeration, selection, preview, and switching suite weekly. A Zoom update invalidates compatibility confidence until that suite passes and the new version is recorded.
