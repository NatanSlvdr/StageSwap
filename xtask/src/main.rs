use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const REQUIRED_WINDOWS_SDK: &str = "10.0.22621.0";

fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("validate-pe") => validate_pe(
            arguments.next().context("missing PE path")?,
            arguments.next().context("missing architecture")?,
        ),
        Some("portable") => portable(
            arguments.next().context("missing architecture")?,
            arguments.next().map(PathBuf::from),
        ),
        Some(command) => bail!("unknown xtask command: {command}"),
        None => bail!(
            "usage: cargo xtask <validate-pe PATH x64|arm64 | portable x64|arm64 [OUTPUT_DIR]>"
        ),
    }
}

fn portable(architecture: String, output: Option<PathBuf>) -> Result<()> {
    let windows_sdk = selected_windows_sdk()?;
    let (target, artifact) = match architecture.as_str() {
        "x64" => ("x86_64-pc-windows-msvc", "windows-x64-portable.exe"),
        "arm64" => ("aarch64-pc-windows-msvc", "windows-arm64-portable.exe"),
        _ => bail!("architecture must be x64 or arm64"),
    };
    run(Command::new("cargo").args([
        "build",
        "-p",
        "asc-media-source",
        "--release",
        "--target",
        target,
    ]))?;
    let dll = PathBuf::from("target")
        .join(target)
        .join("release")
        .join("asc_media_source.dll");
    if !dll.is_file() {
        bail!("media-source build did not produce {}", dll.display());
    }
    validate_pe_path(&dll, &architecture)?;
    let mut build = Command::new("cargo");
    build
        .env(
            "ASC_MEDIA_SOURCE_DLL",
            dll.canonicalize()
                .context("canonicalize media-source DLL")?,
        )
        .args([
            "build",
            "-p",
            "automatic-screen-camera",
            "--release",
            "--target",
            target,
        ]);
    run(&mut build)?;
    let executable = PathBuf::from("target")
        .join(target)
        .join("release")
        .join("AutomaticScreenCamera.exe");
    validate_pe_path(&executable, &architecture)?;
    validate_embedded_payload(&executable, &dll)?;
    let output = output.unwrap_or_else(|| PathBuf::from("dist"));
    fs::create_dir_all(&output).context("create dist directory")?;
    let destination = output.join(artifact);
    fs::copy(&executable, &destination).with_context(|| {
        format!(
            "copy portable executable from {} to {}",
            executable.display(),
            destination.display()
        )
    })?;
    let digest = sha256(&fs::read(&destination)?);
    let revision = env::var("GITHUB_SHA").unwrap_or_else(|_| "unknown".into());
    let checksum = format!(
        "# applicationVersion={}\n# sourceRevision={revision}\n# architecture={architecture}\n# configuration=Release\n# windowsSdk={windows_sdk}\n{} *{artifact}\n",
        env!("CARGO_PKG_VERSION"),
        hex(&digest)
    );
    fs::write(output.join(format!("{artifact}.sha256")), checksum)?;
    println!("packaged {}", destination.display());
    Ok(())
}

fn selected_windows_sdk() -> Result<String> {
    let value = env::var("ASC_WINDOWS_SDK_VERSION")
        .or_else(|_| env::var("WindowsSDKVersion"))
        .context(
            "Windows SDK is not selected; run from a VS Developer shell or set \
             ASC_WINDOWS_SDK_VERSION",
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
        bail!("portable executable does not contain the media-source payload");
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
        "arm64" => 0xaa64,
        _ => bail!("architecture must be x64 or arm64"),
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
// command or adding a runtime dependency to the portable artifact.
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
}
