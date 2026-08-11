use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows_core::PCWSTR;

static LAST_LOGGED: OnceLock<Mutex<HashMap<&'static str, Instant>>> = OnceLock::new();

pub(super) fn always(message: impl Display) {
    let message = format!("StageSwap.MediaSource: {message}");
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    // SAFETY: the string is terminated and remains live for the duration of the call.
    unsafe { OutputDebugStringW(PCWSTR(wide.as_ptr())) };
}

pub(super) fn rate_limited(key: &'static str, interval: Duration, message: impl Display) {
    let now = Instant::now();
    let state = LAST_LOGGED.get_or_init(|| Mutex::new(HashMap::new()));
    let should_log = state
        .lock()
        .map(|mut state| match state.get(key).copied() {
            Some(previous) if now.saturating_duration_since(previous) < interval => false,
            _ => {
                state.insert(key, now);
                true
            }
        })
        .unwrap_or(true);
    if should_log {
        always(message);
    }
}

pub(super) fn path_tag(path: &str) -> u64 {
    // A stable, non-reversible tag is sufficient to correlate retries without
    // exposing the per-user SID embedded in the named-pipe path.
    path.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
