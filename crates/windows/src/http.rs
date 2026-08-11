use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::path::Path;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts,
};
use windows_core::PCWSTR;

const REQUEST_TIMEOUT_MS: i32 = 30_000;
const BUFFER_SIZE: usize = 64 * 1024;

struct InternetHandle(*mut c_void);

impl InternetHandle {
    fn new(handle: *mut c_void, operation: &str) -> Result<Self, String> {
        if handle.is_null() {
            // SAFETY: GetLastError reads the calling thread's error slot.
            let code = unsafe { GetLastError().0 };
            Err(format!("{operation} failed with Windows error {code}"))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for InternetHandle {
    fn drop(&mut self) {
        // SAFETY: The handle was returned by WinHTTP, is owned here, and is closed once.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

pub fn https_get(url: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    read_https(url, maximum_bytes as u64, |bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    })?;
    Ok(output)
}

pub fn https_download(url: &str, path: &Path, maximum_bytes: u64) -> Result<(), String> {
    let mut file = fs::File::create(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let result = read_https(url, maximum_bytes, |bytes| {
        file.write_all(bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    });
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    result
}

fn read_https(
    url: &str,
    maximum_bytes: u64,
    mut consume: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let (host, path) = parse_url(url)?;
    let agent = wide("StageSwap-Updater/1.0");
    let host = wide(host);
    let path = wide(path);
    let get = wide("GET");
    // SAFETY: All pointers refer to live, NUL-terminated UTF-16 buffers for each call.
    let session = InternetHandle::new(
        unsafe {
            WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        },
        "WinHttpOpen",
    )?;
    // SAFETY: session is a valid WinHTTP session and host is NUL-terminated.
    unsafe {
        WinHttpSetTimeouts(
            session.0,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
        )
    }
    .map_err(|error| format!("could not configure update request timeouts: {error}"))?;
    // SAFETY: session is valid and the host string remains live for this call.
    let connection = InternetHandle::new(
        unsafe { WinHttpConnect(session.0, PCWSTR(host.as_ptr()), 443, 0) },
        "WinHttpConnect",
    )?;
    // SAFETY: connection is valid; method and path are NUL-terminated and live.
    let request = InternetHandle::new(
        unsafe {
            WinHttpOpenRequest(
                connection.0,
                PCWSTR(get.as_ptr()),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            )
        },
        "WinHttpOpenRequest",
    )?;
    let headers = "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n";
    let headers = headers.encode_utf16().collect::<Vec<_>>();
    // SAFETY: request is valid and the optional header slice is live for the call.
    unsafe { WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0) }
        .map_err(|error| format!("could not send update request: {error}"))?;
    // SAFETY: request is valid and no reserved pointer is supplied.
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }
        .map_err(|error| format!("could not receive update response: {error}"))?;
    let mut status = 0u32;
    let mut status_size = std::mem::size_of::<u32>() as u32;
    let mut index = 0u32;
    // SAFETY: The output buffer points to a u32 of the declared size.
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut status_size,
            &mut index,
        )
    }
    .map_err(|error| format!("could not read update response status: {error}"))?;
    if status != 200 {
        return Err(format!("GitHub returned HTTP {status}"));
    }

    let mut total = 0u64;
    let mut buffer = [0u8; BUFFER_SIZE];
    loop {
        let mut read = 0u32;
        // SAFETY: request is valid and buffer is writable for BUFFER_SIZE bytes.
        unsafe {
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
        }
        .map_err(|error| format!("could not read update response: {error}"))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::from(read))
            .ok_or_else(|| "update response size overflow".to_owned())?;
        if total > maximum_bytes {
            return Err("update response exceeded its size limit".into());
        }
        consume(&buffer[..read as usize])?;
    }
    Ok(())
}

fn parse_url(url: &str) -> Result<(&str, &str), String> {
    let remainder = url
        .strip_prefix("https://")
        .ok_or_else(|| "update URL must use HTTPS".to_owned())?;
    let (host, path) = remainder.split_once('/').unwrap_or((remainder, ""));
    if host.is_empty()
        || !(host == "api.github.com"
            || host == "github.com"
            || host.ends_with(".githubusercontent.com"))
    {
        return Err("update URL uses an unexpected host".into());
    }
    Ok((
        host,
        if path.is_empty() {
            "/"
        } else {
            &url[url.len() - path.len() - 1..]
        },
    ))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_expected_https_hosts_are_accepted() {
        assert_eq!(
            parse_url("https://api.github.com/repos/a/b").unwrap(),
            ("api.github.com", "/repos/a/b")
        );
        assert!(parse_url("http://github.com/a").is_err());
        assert!(parse_url("https://example.com/a").is_err());
        assert!(parse_url("https://github.com.example.com/a").is_err());
    }
}
