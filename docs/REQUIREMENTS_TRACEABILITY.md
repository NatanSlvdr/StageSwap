# Requirements traceability

| Contract | Rust implementation | Verification |
|---|---|---|
| Immutable BGRA frame, fixed 720p30 | `stageswap-core::Frame`, app compositor | invalid-frame, fit, blend tests |
| Saved-then-unique webcam, no rediscovery loop | `MediaFoundationVideoInput`, `choose_video_device` | selection test and native Windows checks |
| Windows Graphics Capture and cursor option | `WindowsGraphicsScreenInput` | x64 build and native Windows capture checks |
| Reference detection 250 ms, 5/3 debounce | runtime detector, `DebouncedDetector` | debounce test and switching checks |
| Two-scan monitor selection | `MonitorTracker`, runtime 30-second scanner | monitor test and multi-monitor checks |
| Fallbacks and reversible fade | `decide`, `TransitionController`, `Frame::blend` | decision and reversal tests |
| Isolated schema 1, backup, atomic Windows save | `ConfigStore`, `save_config_atomic` | round-trip/corruption tests |
| Hidden admin baseline and optional launch restore | `AdminProfileStore`, Settings admin window | profile, rollback, startup, toggle, and gesture tests |
| Strict 40-byte bounded IPC and two-second expiry | `FrameHeader`, `SharedFrameCache`, `FramePublisher` | IPC tests |
| Rust COM source and required interfaces | `stageswap-media-source::com_server` | Windows COM/source-state tests |
| RGB32 1280×720@30 only | `MediaStream` media type | MF probe, Windows Camera, Zoom |
| Native self-deploying x64 | deployment module, embedded manifest/version, and `xtask package` | PE/payload validation, first-run and cleanup |
| UI, tray, previews, notifications, logs, four restarts | `stageswap` | headless 100%/150% render tests, native ignored tests, and screenshot comparison |
