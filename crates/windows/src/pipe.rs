use asc_core::{Frame, FrameHeader, MAX_FRAME_BYTES};
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
    LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{PIPE_ACCESS_OUTBOUND, WriteFile};
use windows::Win32::System::IO::CancelSynchronousIo;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::core::{BOOL, PCWSTR};

#[derive(Default)]
struct Latest {
    sequence: u64,
    packet: Vec<u8>,
}

struct Shared {
    latest: Mutex<Latest>,
    changed: Condvar,
    stop: AtomicBool,
}

/// Single-client, bounded-memory publisher used by the out-of-process Media
/// Foundation source. Publishing replaces the previous frame rather than
/// queueing, so a slow consumer cannot make the application accumulate frames.
pub struct FramePublisher {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl FramePublisher {
    pub fn start(pipe_name: &str) -> Result<Self, String> {
        if pipe_name.is_empty() {
            return Err("frame pipe name must not be empty".into());
        }
        let path = if pipe_name.starts_with(r"\\.\pipe\") {
            pipe_name.to_owned()
        } else {
            format!(r"\\.\pipe\{pipe_name}")
        };
        let shared = Arc::new(Shared {
            latest: Mutex::new(Latest::default()),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("asc-frame-publisher".into())
            .spawn(move || server_loop(&path, &worker_shared))
            .map_err(|error| format!("could not start frame publisher: {error}"))?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub fn publish(&self, frame: &Frame) -> Result<(), String> {
        let frame_bytes =
            u32::try_from(frame.pixels().len()).map_err(|_| "frame exceeds the IPC capacity")?;
        if frame_bytes > MAX_FRAME_BYTES {
            return Err("frame exceeds the IPC capacity".into());
        }
        let mut latest = self
            .shared
            .latest
            .lock()
            .map_err(|_| "frame publisher state is poisoned")?;
        latest.sequence = latest.sequence.wrapping_add(1).max(1);
        let header = FrameHeader {
            sequence: latest.sequence,
            size: frame.size,
            stride: frame.stride,
            timestamp_100ns: frame.timestamp_100ns,
            frame_bytes,
        }
        .encode()
        .map_err(|error| format!("invalid IPC frame: {error:?}"))?;
        latest.packet.clear();
        latest.packet.reserve(header.len() + frame.pixels().len());
        latest.packet.extend_from_slice(&header);
        latest.packet.extend_from_slice(frame.pixels());
        drop(latest);
        self.shared.changed.notify_all();
        Ok(())
    }

    pub fn invalidate(&self) -> Result<(), String> {
        let mut latest = self
            .shared
            .latest
            .lock()
            .map_err(|_| "frame publisher state is poisoned")?;
        latest.sequence = latest.sequence.wrapping_add(1).max(1);
        latest.packet = FrameHeader::invalidation(latest.sequence)
            .and_then(FrameHeader::encode)
            .map_err(|error| format!("invalid IPC invalidation: {error:?}"))?
            .to_vec();
        drop(latest);
        self.shared.changed.notify_all();
        Ok(())
    }
}

impl Drop for FramePublisher {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        if let Some(worker) = self.worker.take() {
            // SAFETY: this is the live worker thread handle and cancellation is
            // limited to its blocking ConnectNamedPipe/WriteFile calls.
            let _ = unsafe { CancelSynchronousIo(HANDLE(worker.as_raw_handle())) };
            let _ = worker.join();
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns a valid kernel handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the SDDL conversion allocates this buffer with LocalAlloc.
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}

fn server_loop(path: &str, shared: &Shared) {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // Owner, SYSTEM, and LocalService can connect; remote clients are rejected
    // by the named-pipe mode as an additional boundary.
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;LS)"
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: SDDL is terminated and descriptor is a writable output pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .is_err()
    {
        return;
    }
    let _descriptor = SecurityDescriptor(descriptor);
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: BOOL(0),
    };
    let path: Vec<u16> = path.encode_utf16().chain([0]).collect();
    while !shared.stop.load(Ordering::Acquire) {
        // SAFETY: name and security attributes remain live until the pipe handle
        // is closed, and all sizes fit in u32 by the core IPC limit.
        let raw = unsafe {
            CreateNamedPipeW(
                PCWSTR(path.as_ptr()),
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_FRAME_BYTES + 40,
                64 * 1024,
                0,
                Some(&security),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return;
        }
        let pipe = OwnedHandle(raw);
        // SAFETY: pipe is a live server-side named-pipe handle.
        let connected = unsafe { ConnectNamedPipe(pipe.0, None) }.is_ok()
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            continue;
        }
        send_frames(pipe.0, shared);
        // SAFETY: pipe remains live through this call.
        let _ = unsafe { DisconnectNamedPipe(pipe.0) };
    }
}

fn send_frames(pipe: HANDLE, shared: &Shared) {
    let mut sent = 0;
    while !shared.stop.load(Ordering::Acquire) {
        let Ok(latest) = shared.latest.lock() else {
            return;
        };
        let Ok(latest) = shared.changed.wait_while(latest, |latest| {
            latest.sequence == sent && !shared.stop.load(Ordering::Acquire)
        }) else {
            return;
        };
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        let sequence = latest.sequence;
        let packet = latest.packet.clone();
        drop(latest);
        if packet.is_empty() || !write_all(pipe, &packet) {
            return;
        }
        sent = sequence;
    }
}

fn write_all(pipe: HANDLE, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let mut written = 0;
        // SAFETY: pipe is live, the slice is readable, and written is writable.
        if unsafe { WriteFile(pipe, Some(bytes), Some(&mut written), None) }.is_err()
            || written == 0
        {
            return false;
        }
        bytes = &bytes[written as usize..];
    }
    true
}
