#![cfg_attr(not(windows), forbid(unsafe_code))]

pub const SOURCE_CLSID: &str = "{4ABA794D-7B23-449C-8467-CE74A41C2820}";
pub const PIPE_ATTRIBUTE: &str = "{75C753A0-587B-4064-BB77-F0171FCD4AD7}";

#[cfg(windows)]
mod com_server;

// The COM implementation is built only on Windows. Keeping the crate valid on
// other hosts lets the pure workspace checks run without pretending to test MF.
#[cfg(not(windows))]
pub fn platform_gate() -> &'static str {
    "Media Foundation source requires Windows"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_public_media_source_identity_is_stageswap() {
        assert_eq!(SOURCE_CLSID, "{4ABA794D-7B23-449C-8467-CE74A41C2820}");
        assert_eq!(PIPE_ATTRIBUTE, "{75C753A0-587B-4064-BB77-F0171FCD4AD7}");
    }
}
