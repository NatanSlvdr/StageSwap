use anyhow::{Context, Result, bail};
use std::cmp::max;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const REQUIRED_WINDOWS_SDK: &str = "10.0.22621.0";
const APP_PACKAGE: &str = "stageswap";
const MEDIA_SOURCE_PACKAGE: &str = "stageswap-media-source";
const APP_EXECUTABLE: &str = "StageSwap.exe";
const MEDIA_SOURCE_DLL: &str = "stageswap_media_source.dll";
const RELEASE_PREFIX: &str = "StageSwap_win64_v";
const RELEASE_SUFFIX: &str = ".exe.sha256";

fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("validate-pe") => validate_pe(
            arguments.next().context("missing PE path")?,
            arguments.next().context("missing architecture")?,
        ),
        Some("package") => package(
            arguments.next().context("missing architecture")?,
            arguments.next().map(PathBuf::from),
        ),
        Some(command) => bail!("unknown xtask command: {command}"),
        None => bail!("usage: cargo xtask <validate-pe PATH x64 | package x64 [OUTPUT_DIR]>"),
    }
}

fn package(architecture: String, output: Option<PathBuf>) -> Result<()> {
    let windows_sdk = selected_windows_sdk()?;
    let target = match architecture.as_str() {
        "x64" => "x86_64-pc-windows-msvc",
        _ => bail!("architecture must be x64"),
    };
    run(&mut cargo_build(MEDIA_SOURCE_PACKAGE, target, None))?;
    let dll = PathBuf::from("target")
        .join(target)
        .join("release")
        .join(MEDIA_SOURCE_DLL);
    if !dll.is_file() {
        bail!("media-source build did not produce {}", dll.display());
    }
    validate_pe_path(&dll, &architecture)?;
    let embedded_dll = dll
        .canonicalize()
        .context("canonicalize media-source DLL")?;
    run(&mut cargo_build(APP_PACKAGE, target, Some(&embedded_dll)))?;
    let executable = PathBuf::from("target")
        .join(target)
        .join("release")
        .join(APP_EXECUTABLE);
    validate_pe_path(&executable, &architecture)?;
    validate_embedded_payload(&executable, &dll)?;
    let output = output.unwrap_or_else(|| PathBuf::from("dist"));
    fs::create_dir_all(&output).context("create dist directory")?;
    let digest = sha256(&fs::read(&executable)?);
    let history = env::var_os("STAGESWAP_RELEASE_HISTORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| output.clone());
    let release_version = select_release_version(&history, &digest)?;
    let artifact = format!("{RELEASE_PREFIX}{release_version}.exe");
    let destination = output.join(&artifact);
    fs::copy(&executable, &destination).with_context(|| {
        format!(
            "copy executable from {} to {}",
            executable.display(),
            destination.display()
        )
    })?;
    let revision = env::var("GITHUB_SHA").unwrap_or_else(|_| "unknown".into());
    let checksum = format!(
        "# applicationVersion={}\n# releaseVersion={release_version}\n# sourceRevision={revision}\n# architecture={architecture}\n# configuration=Release\n# windowsSdk={windows_sdk}\n{} *{artifact}\n",
        env!("CARGO_PKG_VERSION"),
        hex(&digest)
    );
    fs::write(output.join(format!("{artifact}.sha256")), checksum)?;
    println!("packaged {}", destination.display());
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl ReleaseVersion {
    fn parse(value: &str) -> Result<Self> {
        let mut parts = value.split('.');
        let version = Self {
            major: parts.next().context("missing major version")?.parse()?,
            minor: parts.next().context("missing minor version")?.parse()?,
            patch: parts.next().context("missing patch version")?.parse()?,
        };
        if parts.next().is_some() {
            bail!("release version must contain exactly three numbers");
        }
        Ok(version)
    }

    fn increment_patch(self) -> Result<Self> {
        Ok(Self {
            patch: self
                .patch
                .checked_add(1)
                .context("patch version overflow")?,
            ..self
        })
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn select_release_version(output: &Path, digest: &[u8; 32]) -> Result<ReleaseVersion> {
    let application_version = ReleaseVersion::parse(env!("CARGO_PKG_VERSION"))?;
    let Some((latest_version, latest_digest)) = latest_release(output)? else {
        return Ok(application_version);
    };
    if latest_digest == hex(digest) {
        return Ok(latest_version);
    }
    Ok(max(latest_version.increment_patch()?, application_version))
}

fn latest_release(output: &Path) -> Result<Option<(ReleaseVersion, String)>> {
    if !output.exists() {
        return Ok(None);
    }
    let mut latest = None;
    for entry in fs::read_dir(output).context("read release output directory")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(version) = name
            .strip_prefix(RELEASE_PREFIX)
            .and_then(|name| name.strip_suffix(RELEASE_SUFFIX))
        else {
            continue;
        };
        let Ok(version) = ReleaseVersion::parse(version) else {
            continue;
        };
        if latest
            .as_ref()
            .is_some_and(|(latest_version, _)| *latest_version >= version)
        {
            continue;
        }
        let contents = fs::read_to_string(entry.path())
            .with_context(|| format!("read release checksum {}", entry.path().display()))?;
        let digest = contents
            .lines()
            .find(|line| !line.starts_with('#') && !line.trim().is_empty())
            .and_then(|line| line.split_whitespace().next())
            .context("release sidecar has no checksum")?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("release sidecar has an invalid SHA-256 checksum");
        }
        latest = Some((version, digest.to_ascii_lowercase()));
    }
    Ok(latest)
}

fn cargo_build(package: &str, target: &str, embedded_dll: Option<&Path>) -> Command {
    let mut command = Command::new("cargo");
    if env::var_os("STAGESWAP_USE_CARGO_XWIN").is_some() {
        command.args(["xwin", "build"]);
    } else {
        command.arg("build");
    }
    command.args(["-p", package, "--release", "--target", target]);
    if let Some(dll) = embedded_dll {
        command.env("STAGESWAP_MEDIA_SOURCE_DLL", dll);
    }
    command
}

fn selected_windows_sdk() -> Result<String> {
    let value = env::var("STAGESWAP_WINDOWS_SDK_VERSION")
        .or_else(|_| env::var("WindowsSDKVersion"))
        .context(
            "Windows SDK is not selected; run from a VS Developer shell or set \
             STAGESWAP_WINDOWS_SDK_VERSION",
        )?;
    let normalized = value.trim().trim_end_matches(['\\', '/']);
    if normalized != REQUIRED_WINDOWS_SDK {
        bail!("Windows SDK {normalized} is selected; packaging requires {REQUIRED_WINDOWS_SDK}");
    }
    Ok(normalized.to_owned())
}

fn validate_embedded_payload(executable: &Path, payload: &Path) -> Result<()> {
    let executable_bytes =
        fs::read(executable).with_context(|| format!("could not read {}", executable.display()))?;
    let payload_bytes =
        fs::read(payload).with_context(|| format!("could not read {}", payload.display()))?;
    let marker_len = payload_bytes.len().min(64);
    if marker_len == 0 {
        bail!("embedded media-source payload is empty");
    }
    let marker = &payload_bytes[..marker_len];
    let embedded = executable_bytes
        .windows(marker_len)
        .enumerate()
        .any(|(offset, candidate)| {
            candidate == marker
                && executable_bytes.get(offset..offset + payload_bytes.len())
                    == Some(payload_bytes.as_slice())
        });
    if !embedded {
        bail!("executable does not contain the media-source payload");
    }
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let description = format!("{command:?}");
    let status = command
        .status()
        .with_context(|| format!("could not run {description}"))?;
    if !status.success() {
        bail!("command failed ({status}): {description}");
    }
    Ok(())
}

fn validate_pe(path: String, architecture: String) -> Result<()> {
    validate_pe_path(Path::new(&path), &architecture)?;
    println!("validated {architecture} PE: {path}");
    Ok(())
}

fn validate_pe_path(path: &Path, architecture: &str) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    if bytes.get(..2) != Some(b"MZ") {
        bail!("{} is not a PE file", path.display());
    }
    let pe_offset = u32::from_le_bytes(
        bytes
            .get(0x3c..0x40)
            .context("truncated DOS header")?
            .try_into()?,
    ) as usize;
    if bytes.get(pe_offset..pe_offset + 4) != Some(b"PE\0\0") {
        bail!("{} has no PE signature", path.display());
    }
    let machine = u16::from_le_bytes(
        bytes
            .get(pe_offset + 4..pe_offset + 6)
            .context("truncated COFF header")?
            .try_into()?,
    );
    let expected = match architecture {
        "x64" => 0x8664,
        _ => bail!("architecture must be x64"),
    };
    if machine != expected {
        bail!("PE machine 0x{machine:04x} does not match {architecture}");
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    output
}

// Compact FIPS 180-4 SHA-256 used by packaging to avoid relying on a platform
// command or adding a runtime dependency to the artifact.
fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hash = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (word, bytes) in words.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().expect("four-byte word"));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (value, addition) in hash.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(addition);
        }
    }
    let mut output = [0; 32];
    for (bytes, value) in output.chunks_exact_mut(4).zip(hash) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn embedded_payload_validation_skips_partial_marker_matches() {
        let directory = tempfile::tempdir().unwrap();
        let payload = (0..96_u8).collect::<Vec<_>>();
        let payload_path = directory.path().join("source.dll");
        let executable_path = directory.path().join("app.exe");
        fs::write(&payload_path, &payload).unwrap();
        let mut executable = payload[..64].to_vec();
        executable.extend_from_slice(b"not the payload");
        executable.extend_from_slice(&payload);
        fs::write(&executable_path, executable).unwrap();
        validate_embedded_payload(&executable_path, &payload_path).unwrap();
    }

    #[test]
    fn windows_sdk_version_normalization_is_exact() {
        assert_eq!(
            "10.0.22621.0\\".trim_end_matches(['\\', '/']),
            REQUIRED_WINDOWS_SDK
        );
        assert_ne!(
            "10.0.26100.0".trim_end_matches(['\\', '/']),
            REQUIRED_WINDOWS_SDK
        );
    }

    #[test]
    fn release_version_reuses_matching_checksum_and_increments_changed_checksum() {
        let directory = tempfile::tempdir().unwrap();
        let digest = sha256(b"current build");
        let sidecar = directory.path().join("StageSwap_win64_v1.2.22.exe.sha256");
        fs::write(&sidecar, format!("{} *artifact.exe\n", hex(&digest))).unwrap();

        assert_eq!(
            select_release_version(directory.path(), &digest).unwrap(),
            ReleaseVersion::parse("1.2.22").unwrap()
        );
        assert_eq!(
            select_release_version(directory.path(), &sha256(b"changed build")).unwrap(),
            ReleaseVersion::parse("1.2.23").unwrap()
        );
    }

    #[test]
    fn packaging_identity_is_stageswap() {
        assert_eq!(APP_PACKAGE, "stageswap");
        assert_eq!(MEDIA_SOURCE_PACKAGE, "stageswap-media-source");
        assert_eq!(APP_EXECUTABLE, "StageSwap.exe");
        assert_eq!(MEDIA_SOURCE_DLL, "stageswap_media_source.dll");
        assert_eq!(RELEASE_PREFIX, "StageSwap_win64_v");
        assert_eq!(RELEASE_SUFFIX, ".exe.sha256");
    }

    #[test]
    fn release_version_starts_at_application_version() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            select_release_version(directory.path(), &sha256(b"first build")).unwrap(),
            ReleaseVersion::parse(env!("CARGO_PKG_VERSION")).unwrap()
        );
    }

    #[test]
    fn webcam_switcher_history_does_not_affect_stageswap_versions() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory
                .path()
                .join("WebcamSwitcher_win64_v9.9.9.exe.sha256"),
            format!("{} *legacy.exe\n", hex(&sha256(b"legacy"))),
        )
        .unwrap();
        assert_eq!(
            select_release_version(directory.path(), &sha256(b"first StageSwap build")).unwrap(),
            ReleaseVersion::parse("0.2.0").unwrap()
        );
    }
}
