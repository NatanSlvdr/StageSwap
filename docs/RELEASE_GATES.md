# Release gates

A release contains exactly `StageSwap_win64_vX.Y.Z.exe` plus its SHA-256 sidecar. A checksum change increments `Z`; identical bytes with an already synchronized workspace retain their existing release version. Packaging persists the selected version to `Cargo.toml` and `Cargo.lock`, rebuilds the production DLL and EXE, and requires that the filename, UI, Windows version resources, `applicationVersion`, and `releaseVersion` all agree. It must be built and tested from the release commit with Windows SDK 10.0.22621.0. CI restores the newest release sidecar before packaging, selects that SDK explicitly, and `xtask` refuses packaging when the selected SDK is missing or different.

Before publication:

- Package validation confirms x64 PE machine type `0x8664` for both executable and embedded Media Foundation DLL.
- Cargo Debug and Release builds are warning-clean for x64; format, Clippy, packaging tests, dependency audit, and x64 Windows tests pass.
- VM acceptance passes on current Windows 11 x64 for deployment, cleanup, retained workflow, Windows Camera, Zoom, and all consumer formats.
- The physical x64 smoke test passes for the retained stable-hardware workflow.
- Reference screenshots exist and the Rust UI comparison passes at both 100% and 150% DPI; the native interactive test suite passes on x64.
- Artifact versions, source revisions, architecture metadata, and checksums match the release record.

No reliability claim covers excluded lifecycle, hot-plug, driver, contention, or performance scenarios beyond the documented 1280×720-at-30-fps acceptance checks.
