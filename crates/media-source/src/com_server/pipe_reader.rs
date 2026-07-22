use stageswap_core::{FrameHeader, HEADER_LEN, SharedFrameCache};
use std::fs::OpenOptions;
use std::io::Read;
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::CancelSynchronousIo;

pub(super) struct PipeReader {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PipeReader {
    pub(super) fn start(pipe_name: String, cache: Arc<Mutex<SharedFrameCache>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        if pipe_name.is_empty() {
            return Self { stop, worker: None };
        }
        let path = if pipe_name.starts_with(r"\\.\pipe\") {
            pipe_name
        } else {
            format!(r"\\.\pipe\{pipe_name}")
        };
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("stageswap-mf-pipe-reader".into())
            .spawn(move || reader_loop(&path, &worker_stop, &cache))
            .ok();
        Self { stop, worker }
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // SAFETY: the raw handle belongs to the still-live worker thread and
            // is used only to cancel its synchronous named-pipe read.
            let _ = unsafe { CancelSynchronousIo(HANDLE(worker.as_raw_handle())) };
            let _ = worker.join();
        }
    }
}

fn reader_loop(path: &str, stop: &AtomicBool, cache: &Mutex<SharedFrameCache>) {
    while !stop.load(Ordering::Acquire) {
        let Ok(mut pipe) = OpenOptions::new().read(true).open(path) else {
            thread::sleep(Duration::from_millis(250));
            continue;
        };
        let mut previous_sequence = None;
        while !stop.load(Ordering::Acquire) {
            let mut encoded = [0; HEADER_LEN];
            if pipe.read_exact(&mut encoded).is_err() {
                break;
            }
            let header = match FrameHeader::decode(&encoded, previous_sequence) {
                Ok(header) => header,
                Err(_) => {
                    if let Ok(mut cache) = cache.lock() {
                        cache.invalidate();
                    }
                    break;
                }
            };
            previous_sequence = Some(header.sequence);
            let mut pixels = vec![0; header.frame_bytes as usize];
            if pipe.read_exact(&mut pixels).is_err() {
                break;
            }
            let Ok(mut cache) = cache.lock() else {
                return;
            };
            if cache.ingest(header, pixels.into(), Instant::now()).is_err() {
                cache.invalidate();
                break;
            }
        }
        if let Ok(mut cache) = cache.lock() {
            cache.invalidate();
        }
    }
}
