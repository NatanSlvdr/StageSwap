# Requirements traceability

| Contract | Rust implementation | Verification |
|---|---|---|
| Immutable BGRA frame, fixed 720p30 | `stageswap-core::Frame`, app compositor | invalid-frame, fit, blend tests |
| Saved-then-unique webcam, tiered progressive input negotiation, bounded same-device recovery without rediscovery | `MediaFoundationVideoInput`, `WebcamRecovery`, `choose_video_device` | ranking/optional-metadata/stride/recovery-state tests, x64 build, native RGB32/MJPEG/YUY2/NV12 checks |
| Windows Graphics Capture, cursor option, capability/HDR preflight, session-valid screen health, generation-safe failure handling | `WindowsGraphicsScreenInput`, `DevicePlatform` | lifecycle/error/old-frame tests, x64 build, native SDR/HDR/restart-race checks |
| Reference detection 250 ms, 5/3 debounce | runtime detector, `DebouncedDetector` | debounce test and switching checks |
| Two-scan monitor selection and nonblocking discovery | `MonitorTracker`, bounded monitor/video-device workers | coalescing/shutdown/nonblocking tests, monitor test, and multi-monitor checks |
| Fallbacks and reversible fade | `decide`, `TransitionController`, `Frame::blend` | decision and reversal tests |
| Isolated schema 1, writable-flushed candidate, backup, atomic Windows save | `ConfigStore`, `persist_rgba_atomic`, `save_config_atomic` | round-trip/corruption/rollback tests and native writable-flush check |
| Hidden admin baseline and optional launch restore | `AdminProfileStore`, Settings admin window | profile, rollback, startup, toggle, and gesture tests |
| Strict 40-byte bounded IPC and two-second expiry | `FrameHeader`, `SharedFrameCache`, `FramePublisher` | IPC tests |
| Rust COM source and required interfaces | `stageswap-media-source::com_server` | Windows COM/source-state tests |
| RGB32/NV12 1280×720@30 output with limited BT.601 metadata and sequence cache | `MediaStream` media types and NV12 cache | color/grayscale metadata tests, MF probe, Windows Camera, Zoom |
| Native self-deploying x64 | deployment module, embedded manifest/version, and `xtask publish-release` | PE/payload validation, first-run and cleanup |
| Manual Stable/Beta GitHub updates | update worker, WinHTTP adapter, checksum verification, replacement bootstrap | release-selection/config/checksum tests and native update/rollback checks |
| Deterministic lifecycle, deadlines, bounded mailbox, and constant-time pacing | `RuntimeEngine`, runtime clock, component status, runtime mailbox, `FramePacer` | virtual-clock lifecycle tests, command flood/coalescing/shutdown tests, long-gap pacer tests |
| UI, tray, previews, notifications, logs, recovery submenu, four restarts | `stageswap` | reduced `smoke_` UI coverage, consolidated `contract_`/`flow_` interaction and state tests, tray mapping tests, native ignored tests, and UI-preview/manual screenshot evidence |
| Transactional managed startup and run-once preservation | deployment startup helpers and General Settings | registry status/transition tests and native Windows checks |
