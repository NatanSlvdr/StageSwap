# Requirements traceability

| Contract | Rust implementation | Verification |
|---|---|---|
| Immutable BGRA frame, fixed 720p30 | `stageswap-core::Frame`, app compositor | invalid-frame, fit, blend tests |
| Saved-then-unique webcam, tiered progressive input negotiation, no rediscovery loop | `MediaFoundationVideoInput`, `choose_video_device` | ranking/validation/stride tests, x64 build, native RGB32/MJPEG/YUY2/NV12 checks |
| Windows Graphics Capture, cursor option, capability/HDR preflight, generation-safe closure | `WindowsGraphicsScreenInput` | x64 build and native SDR/HDR/restart-race checks |
| Reference detection 250 ms, 5/3 debounce | runtime detector, `DebouncedDetector` | debounce test and switching checks |
| Two-scan monitor selection | `MonitorTracker`, runtime 30-second scanner | monitor test and multi-monitor checks |
| Fallbacks and reversible fade | `decide`, `TransitionController`, `Frame::blend` | decision and reversal tests |
| Isolated schema 1, backup, atomic Windows save | `ConfigStore`, `save_config_atomic` | round-trip/corruption tests |
| Hidden admin baseline and optional launch restore | `AdminProfileStore`, Settings admin window | profile, rollback, startup, toggle, and gesture tests |
| Strict 40-byte bounded IPC and two-second expiry | `FrameHeader`, `SharedFrameCache`, `FramePublisher` | IPC tests |
| Rust COM source and required interfaces | `stageswap-media-source::com_server` | Windows COM/source-state tests |
| RGB32/NV12 1280×720@30 output with limited BT.601 metadata and sequence cache | `MediaStream` media types and NV12 cache | color/grayscale metadata tests, MF probe, Windows Camera, Zoom |
| Native self-deploying x64 | deployment module, embedded manifest/version, and `xtask package` | PE/payload validation, first-run and cleanup |
| Deterministic lifecycle, deadlines, bounded mailbox, and constant-time pacing | `RuntimeEngine`, runtime clock, component status, runtime mailbox, `FramePacer` | virtual-clock lifecycle tests, command flood/coalescing/shutdown tests, long-gap pacer tests |
| UI, tray, previews, notifications, logs, recovery submenu, four restarts | `stageswap` | headless locale/DPI render tests, tray mapping tests, native ignored tests, screenshot comparison |
| Transactional managed startup and run-once preservation | deployment startup helpers and General Settings | registry status/transition tests and native Windows checks |
