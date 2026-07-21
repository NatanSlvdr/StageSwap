#![cfg_attr(not(windows), forbid(unsafe_code))]

pub const SOURCE_CLSID: &str = "{402EB87C-123B-4765-9FF7-6E11CC7DA5B3}";
pub const PIPE_ATTRIBUTE: &str = "{905306DD-B9A3-4385-A273-606E05B3208B}";

#[cfg(windows)]
mod com_server;

// The COM implementation is built only on Windows. Keeping the crate valid on
// other hosts lets the pure workspace checks run without pretending to test MF.
#[cfg(not(windows))]
pub fn platform_gate() -> &'static str {
    "Media Foundation source requires Windows"
}
