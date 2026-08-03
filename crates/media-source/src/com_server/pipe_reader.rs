use stageswap_core::{
    CAPTURE_FRAME_POOL_CAPACITY, FrameBufferPool, FrameHeader, HEADER_LEN, MAX_FRAME_BYTES,
    SharedFrameCache,
};
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
    let mut pool = FrameBufferPool::new(MAX_FRAME_BYTES as usize, CAPTURE_FRAME_POOL_CAPACITY);
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
            if header.frame_bytes == 0 {
                let Ok(mut cache) = cache.lock() else {
                    return;
                };
                if cache.ingest(header, Arc::from([]), Instant::now()).is_err() {
                    cache.invalidate();
                    break;
                }
                continue;
            }
            let pixels = match read_payload(&mut pipe, &mut pool, header.frame_bytes as usize) {
                Ok(Some(pixels)) => pixels,
                Ok(None) => continue,
                Err(_) => break,
            };
            let Ok(mut cache) = cache.lock() else {
                return;
            };
            if cache.ingest(header, pixels, Instant::now()).is_err() {
                cache.invalidate();
                break;
            }
        }
        if let Ok(mut cache) = cache.lock() {
            cache.invalidate();
        }
    }
}

fn read_payload(
    reader: &mut impl Read,
    pool: &mut FrameBufferPool,
    frame_bytes: usize,
) -> std::io::Result<Option<Arc<[u8]>>> {
    match pool.try_write_sized(frame_bytes, |destination| reader.read_exact(destination))? {
        Some(pixels) => Ok(Some(pixels)),
        None => {
            discard_exact(reader, frame_bytes)?;
            Ok(None)
        }
    }
}

fn discard_exact(reader: &mut impl Read, mut remaining: usize) -> std::io::Result<()> {
    let mut scratch = [0; 8192];
    while remaining > 0 {
        let chunk = remaining.min(scratch.len());
        reader.read_exact(&mut scratch[..chunk])?;
        remaining -= chunk;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn repeated_pipe_payloads_reuse_slots_after_warmup() {
        let mut pool = FrameBufferPool::new(16, CAPTURE_FRAME_POOL_CAPACITY);
        let mut first_reader = Cursor::new(vec![1; 16]);
        let first = read_payload(&mut first_reader, &mut pool, 16)
            .unwrap()
            .unwrap();
        let first_pointer = first.as_ptr();

        let mut second_reader = Cursor::new(vec![2; 16]);
        let second = read_payload(&mut second_reader, &mut pool, 16)
            .unwrap()
            .unwrap();
        assert!(first.iter().all(|byte| *byte == 1));
        drop(first);

        let mut third_reader = Cursor::new(vec![3; 16]);
        let third = read_payload(&mut third_reader, &mut pool, 16)
            .unwrap()
            .unwrap();
        assert_eq!(third.as_ptr(), first_pointer);
        assert!(second.iter().all(|byte| *byte == 2));
        assert!(third.iter().all(|byte| *byte == 3));
        assert_eq!(pool.allocated_slots(), 2);
    }

    #[test]
    fn exhausted_pipe_pool_drains_dropped_payload_without_aliasing() {
        let mut pool = FrameBufferPool::new(4, 2);
        let mut reader = Cursor::new(vec![1; 4]);
        let first = read_payload(&mut reader, &mut pool, 4).unwrap().unwrap();
        let mut reader = Cursor::new(vec![2; 4]);
        let second = read_payload(&mut reader, &mut pool, 4).unwrap().unwrap();
        let mut reader = Cursor::new(vec![3; 4]);
        assert!(read_payload(&mut reader, &mut pool, 4).unwrap().is_none());
        assert_eq!(reader.position(), 4);
        assert_eq!(first.as_ref(), &[1; 4]);
        assert_eq!(second.as_ref(), &[2; 4]);
        assert_eq!(pool.exhaustion_count(), 1);
    }
}
