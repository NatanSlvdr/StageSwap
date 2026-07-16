# Release gates

A release contains exactly `windows-x64-portable.exe` and `windows-arm64-portable.exe`, plus their SHA-256 sidecars. Both must be built and tested from the release commit with Windows SDK 10.0.22621.0.

Before publication:

- Package validation confirms x64 PE machine type `0x8664` and ARM64 type `0xaa64` for both executable and embedded Media Foundation DLL.
- Debug and Release presets build warning-clean for both architectures, and portable unit tests pass.
- VM acceptance passes on current Windows 11 x64 and ARM64 for deployment, cleanup, retained workflow, Windows Camera, Zoom, and all consumer formats.
- Physical x64 and ARM64 smoke tests pass for the retained stable-hardware workflow.
- Artifact versions, source revisions, architecture metadata, and checksums match the release record.

No reliability claim covers excluded lifecycle, hot-plug, driver, contention, or performance scenarios.
