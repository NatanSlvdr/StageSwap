use asc_core::{Frame, FrameHeader, HEADER_LEN, MAX_FRAME_BYTES};
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL,
    INVALID_HANDLE_VALUE, LocalFree,
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

struct Latest {
    sequence: u64,
    header: [u8; HEADER_LEN],
    pixels: Arc<[u8]>,
}

impl Default for Latest {
    fn default() -> Self {
        Self {
            sequence: 0,
            header: [0; HEADER_LEN],
            pixels: Arc::from([]),
        }
    }
}

struct Shared {
    latest: Mutex<Latest>,
    changed: Condvar,
    stop: AtomicBool,
    failure: Mutex<Option<String>>,
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
            failure: Mutex::new(None),
        });
        let worker_shared = Arc::clone(&shared);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("asc-frame-publisher".into())
            .spawn(move || {
                if let Err(error) = server_loop(&path, &worker_shared, &startup_sender) {
                    if let Ok(mut failure) = worker_shared.failure.lock() {
                        *failure = Some(error.clone());
                    }
                    let _ = startup_sender.try_send(Err(error));
                }
            })
            .map_err(|error| format!("could not start frame publisher: {error}"))?;
        match startup_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                shared,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err("frame publisher stopped before creating its named pipe".into())
            }
        }
    }

    pub fn publish(&self, frame: &Frame) -> Result<(), String> {
        self.check_worker()?;
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
        latest.header = FrameHeader {
            sequence: latest.sequence,
            size: frame.size,
            stride: frame.stride,
            timestamp_100ns: frame.timestamp_100ns,
            frame_bytes,
        }
        .encode()
        .map_err(|error| format!("invalid IPC frame: {error:?}"))?;
        latest.pixels = frame.pixels_arc();
        drop(latest);
        self.shared.changed.notify_all();
        Ok(())
    }

    pub fn invalidate(&self) -> Result<(), String> {
        self.check_worker()?;
        let mut latest = self
            .shared
            .latest
            .lock()
            .map_err(|_| "frame publisher state is poisoned")?;
        latest.sequence = latest.sequence.wrapping_add(1).max(1);
        latest.header = FrameHeader::invalidation(latest.sequence)
            .and_then(FrameHeader::encode)
            .map_err(|error| format!("invalid IPC invalidation: {error:?}"))?;
        latest.pixels = Arc::from([]);
        drop(latest);
        self.shared.changed.notify_all();
        Ok(())
    }

    fn check_worker(&self) -> Result<(), String> {
        self.shared
            .failure
            .lock()
            .map_err(|_| "frame publisher failure state is poisoned")?
            .as_ref()
            .map_or_else(|| Ok(()), |error| Err(error.clone()))
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

fn server_loop(
    path: &str,
    shared: &Shared,
    startup: &mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // Owner, SYSTEM, and LocalService can connect; remote clients are rejected
    // by the named-pipe mode as an additional boundary.
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;LS)"
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: SDDL is terminated and descriptor is a writable output pointer.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|error| format!("could not create frame-pipe security descriptor: {error}"))?;
    let _descriptor = SecurityDescriptor(descriptor);
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: BOOL(0),
    };
    let path: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let mut startup = Some(startup);
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
            let error = unsafe { GetLastError() };
            if error == ERROR_PIPE_BUSY {
                return Err(
                    "another Automatic Screen Camera instance already owns the frame pipe; exit it from the system tray before relaunching"
                        .into(),
                );
            }
            return Err(format!("could not create frame pipe: {error:?}"));
        }
        let pipe = OwnedHandle(raw);
        if let Some(startup) = startup.take() {
            startup
                .send(Ok(()))
                .map_err(|_| "frame publisher startup receiver was dropped".to_string())?;
        }
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
    Ok(())
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
        let header = latest.header;
        let pixels = Arc::clone(&latest.pixels);
        drop(latest);
        if !write_all(pipe, &header) || (!pixels.is_empty() && !write_all(pipe, &pixels)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use asc_core::Size;
    use std::time::Instant;

    #[test]
    fn duplicate_pipe_publisher_is_rejected_during_startup() {
        let name = format!(
            r"\\.\pipe\AutomaticScreenCameraRust.PublisherTest.{}",
            std::process::id()
        );
        let first = FramePublisher::start(&name).expect("first publisher should own the pipe");
        let second = FramePublisher::start(&name);
        assert!(second.is_err(), "duplicate publisher unexpectedly started");
        drop(first);
    }

    #[test]
    fn publishing_replaces_the_latest_frame_without_copying_its_pixels() {
        let name = format!(
            r"\\.\pipe\AutomaticScreenCameraRust.LatestTest.{}",
            std::process::id()
        );
        let publisher = FramePublisher::start(&name).unwrap();
        let first = Frame::placeholder(Size::new(2, 2), 0xff00_0000, 1, 0, Instant::now());
        let second = Frame::placeholder(Size::new(2, 2), 0xffff_ffff, 2, 1, Instant::now());
        let second_pixels = second.pixels_arc();
        publisher.publish(&first).unwrap();
        publisher.publish(&second).unwrap();
        let latest = publisher.shared.latest.lock().unwrap();
        assert_eq!(latest.sequence, 2);
        assert!(Arc::ptr_eq(&latest.pixels, &second_pixels));
    }
}
