use stageswap_core::Frame;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

struct PreviewJob {
    frame: Arc<Frame>,
    size: [usize; 2],
}

pub(crate) struct PreparedPreview {
    pub(crate) frame: Arc<Frame>,
    pub(crate) size: [usize; 2],
    pub(crate) image: eframe::egui::ColorImage,
}

#[derive(Default)]
pub(crate) struct PreviewConverterState {
    latest_request: Option<(Arc<Frame>, [usize; 2])>,
    pending: Option<PreviewJob>,
    pub(crate) ready: Option<PreparedPreview>,
    stopping: bool,
}

pub(crate) struct PreviewConverter {
    shared: Arc<(Mutex<PreviewConverterState>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl PreviewConverter {
    pub(crate) fn new(key: &'static str) -> Self {
        let shared = Arc::new((Mutex::new(PreviewConverterState::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name(format!("stageswap-preview-{key}"))
            .spawn(move || preview_converter_loop(&worker_shared))
            .expect("preview conversion worker can be created");
        Self {
            shared,
            worker: Some(worker),
        }
    }

    pub(crate) fn submit(&self, frame: Arc<Frame>, size: [usize; 2]) {
        let (state, wake) = &*self.shared;
        let mut state = state
            .lock()
            .expect("preview converter state is not poisoned");
        if state
            .latest_request
            .as_ref()
            .is_some_and(|(requested, requested_size)| {
                Arc::ptr_eq(requested, &frame) && *requested_size == size
            })
        {
            return;
        }
        state.latest_request = Some((Arc::clone(&frame), size));
        state.pending = Some(PreviewJob { frame, size });
        wake.notify_one();
    }

    pub(crate) fn take_ready(&self) -> Option<PreparedPreview> {
        self.shared
            .0
            .lock()
            .expect("preview converter state is not poisoned")
            .ready
            .take()
    }
}

impl Drop for PreviewConverter {
    fn drop(&mut self) {
        let (state, wake) = &*self.shared;
        if let Ok(mut state) = state.lock() {
            state.stopping = true;
            state.pending = None;
            wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn preview_converter_loop(shared: &Arc<(Mutex<PreviewConverterState>, Condvar)>) {
    loop {
        let job = {
            let (state, wake) = &**shared;
            let mut state = state
                .lock()
                .expect("preview converter state is not poisoned");
            while state.pending.is_none() && !state.stopping {
                state = wake
                    .wait(state)
                    .expect("preview converter state is not poisoned");
            }
            if state.stopping {
                return;
            }
            state
                .pending
                .take()
                .expect("pending preview job is present")
        };
        let prepared = PreparedPreview {
            image: super::frame_image(&job.frame, job.size),
            frame: Arc::clone(&job.frame),
            size: job.size,
        };
        let mut state = shared
            .0
            .lock()
            .expect("preview converter state is not poisoned");
        store_completed_preview(&mut state, prepared);
    }
}

pub(crate) fn store_completed_preview(
    state: &mut PreviewConverterState,
    prepared: PreparedPreview,
) {
    state.ready = Some(prepared);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn completed_frame_is_kept_when_a_newer_request_is_pending() {
        let now = Instant::now();
        let completed = Arc::new(Frame::placeholder(
            stageswap_core::Size::new(2, 2),
            0xff00_0001,
            1,
            0,
            now,
        ));
        let pending = Arc::new(Frame::placeholder(
            stageswap_core::Size::new(2, 2),
            0xff00_0002,
            2,
            0,
            now,
        ));
        let size = [2, 2];
        let mut state = PreviewConverterState {
            latest_request: Some((Arc::clone(&pending), size)),
            pending: Some(PreviewJob {
                frame: Arc::clone(&pending),
                size,
            }),
            ..PreviewConverterState::default()
        };

        store_completed_preview(
            &mut state,
            PreparedPreview {
                image: super::super::frame_image(&completed, size),
                frame: completed,
                size,
            },
        );

        assert_eq!(state.ready.as_ref().unwrap().frame.sequence, 1);
        assert_eq!(state.pending.as_ref().unwrap().frame.sequence, 2);
    }
}
