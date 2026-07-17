# Release gates

A release contains exactly `windows-x64-portable.exe` and `windows-arm64-portable.exe`, plus their SHA-256 sidecars. Both must be built and tested from the release commit with Windows SDK 10.0.22621.0. CI selects that SDK explicitly, and `xtask` refuses packaging when the selected SDK is missing or different; the sidecar records the normalized selected value.

Before publication:

- Package validation confirms x64 PE machine type `0x8664` and ARM64 type `0xaa64` for both executable and embedded Media Foundation DLL.
- Cargo Debug and Release builds are warning-clean for both architectures; format, Clippy, portable tests, dependency audit, and x64 Windows tests pass.
- VM acceptance passes on current Windows 11 x64 and ARM64 for deployment, cleanup, retained workflow, Windows Camera, Zoom, and all consumer formats.
- Physical x64 and ARM64 smoke tests pass for the retained stable-hardware workflow.
- Reference screenshots exist and the Rust UI comparison passes at both 100% and 150% DPI; the native interactive test suites pass on both architectures.
- Artifact versions, source revisions, architecture metadata, and checksums match the release record.

No reliability claim covers excluded lifecycle, hot-plug, driver, contention, or performance scenarios.
