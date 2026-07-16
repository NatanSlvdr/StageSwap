# Requirements traceability

| Contract | Implementation | Verification |
|---|---|---|
| CPU BGRA frame, fixed 720p30 | `asc/core/frame.*`, `video_input`, `screen_capture` | frame, stale-frame, and Windows capture tests |
| CPU fit, letterbox, placeholder, blend | `asc/core/frame.cpp`, `compositor.cpp` | portable unit tests |
| Fixed detector debounce | `detector`, `App::detector_loop` | five-match/three-mismatch unit test |
| Two-scan runtime monitor selection | `RuntimeMonitorDescriptor`, `MonitorTracker`, `full_monitor_scan` | monitor unit and VM acceptance |
| Missing/unavailable fallbacks | `DecisionEngine`, CPU compositor | decision and placeholder unit tests |
| Schema v2 migration | `ConfigStore` | v1 import and removed-field serialization tests |
| Manual lifecycle only | tray/settings commands, no recovery worker | code review and acceptance |
| Portable native x64/ARM64 | deployment validation, CMake presets, `package.ps1` | PE validation and first-run/cleanup acceptance |
| Consumer 720p/1080p RGB32/NV12 | media-source stream | Windows Camera and Zoom probes |
