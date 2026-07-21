use crate::{InputDevice, VideoInput};
use asc_core::{Frame, PIPELINE_SIZE, Size};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaEvent, IMFMediaType, IMFSample, IMFSourceReader, IMFSourceReaderCallback,
    IMFSourceReaderCallback_Impl, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_DEFAULT_STRIDE,
    MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_ASYNC_CALLBACK, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ERROR, MF_VERSION, MFCreateAttributes,
    MFCreateDeviceSource, MFCreateMediaType, MFCreateSourceReaderFromMediaSource,
    MFEnumDeviceSources, MFMediaType_Video, MFSTARTUP_FULL, MFShutdown, MFStartup,
    MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
};
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_core::{HRESULT, PCWSTR, PWSTR, Ref, implement};

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

pub fn enumerate_video_devices() -> Result<Vec<InputDevice>, String> {
    let mut attributes = None;
    // SAFETY: output storage is writable and Media Foundation is initialized by
    // the owning runtime thread.
    unsafe { MFCreateAttributes(&mut attributes, 1) }
        .map_err(|error| format!("could not create device attributes: {error}"))?;
    let attributes = attributes.expect("MFCreateAttributes returned no attributes");
    // SAFETY: attributes is a live IMFAttributes instance.
    unsafe {
        attributes.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )
    }
    .map_err(|error| format!("could not select video capture devices: {error}"))?;
    let mut raw = core::ptr::null_mut();
    let mut count = 0;
    // SAFETY: both output pointers are writable and released below.
    unsafe { MFEnumDeviceSources(&attributes, &mut raw, &mut count) }
        .map_err(|error| format!("could not enumerate video devices: {error}"))?;
    let mut result = Vec::with_capacity(count as usize);
    if count == 0 {
        // Media Foundation may represent an empty array with a null pointer.
        unsafe { CoTaskMemFree((!raw.is_null()).then_some(raw.cast())) };
        return Ok(result);
    }
    if raw.is_null() {
        return Err("video device enumeration returned an empty array pointer".into());
    }
    // SAFETY: MFEnumDeviceSources returned an array of count COM interface slots.
    let activations = unsafe { core::slice::from_raw_parts_mut(raw, count as usize) };
    for activation in activations {
        let Some(activation) = activation.take() else {
            continue;
        };
        let name = allocated_string(&activation, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)
            .unwrap_or_else(|| "Unnamed video device".into());
        let id = allocated_string(
            &activation,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
        )
        .unwrap_or_default();
        let is_virtual = name.contains("Automatic Screen Camera");
        if !is_virtual && !id.is_empty() {
            result.push(InputDevice {
                id,
                name,
                is_virtual: false,
            });
        }
    }
    // SAFETY: Media Foundation allocated the interface-array block with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(raw.cast())) };
    Ok(result)
}

fn allocated_string(activation: &IMFActivate, key: &windows_core::GUID) -> Option<String> {
    let mut value = PWSTR::null();
    let mut length = 0;
    // SAFETY: both output pointers are writable and value is freed below.
    if unsafe { activation.GetAllocatedString(key, &mut value, &mut length) }.is_err() {
        return None;
    }
    if length == 0 {
        unsafe { CoTaskMemFree((!value.is_null()).then_some(value.0.cast())) };
        return Some(String::new());
    }
    if value.is_null() {
        return None;
    }
    // SAFETY: GetAllocatedString returned `length` UTF-16 code units.
    let result =
        unsafe { String::from_utf16(core::slice::from_raw_parts(value.0, length as usize)) }.ok();
    // SAFETY: Media Foundation allocated this string with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    result
}

#[derive(Default)]
struct CaptureState {
    reader: Mutex<Option<IMFSourceReader>>,
    latest: Mutex<Option<Arc<Frame>>>,
    format: Mutex<Size>,
    failure: Mutex<Option<String>>,
    running: AtomicBool,
    generation: AtomicU64,
    sequence: AtomicU64,
}

// SAFETY: IMFSourceReader documents asynchronous callbacks from Media
// Foundation worker threads; this state exposes it only through a mutex.
unsafe impl Send for CaptureState {}
// SAFETY: every mutable field is atomic or mutex-protected.
unsafe impl Sync for CaptureState {}

impl CaptureState {
    fn is_current(&self, expected_generation: u64) -> bool {
        self.running.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire) == expected_generation
    }

    fn request_next(&self, expected_generation: u64) {
        if !self.is_current(expected_generation) {
            return;
        }
        let reader = self.reader.lock().ok().and_then(|reader| reader.clone());
        if let Some(reader) = reader {
            // SAFETY: asynchronous mode requires all output pointers to be null.
            let _ = unsafe { reader.ReadSample(STREAM, 0, None, None, None, None) };
        }
    }

    fn set_failure(&self, message: impl Into<String>) {
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(message.into());
        }
    }
}

#[implement(IMFSourceReaderCallback)]
struct ReaderCallback {
    state: Arc<CaptureState>,
    expected_generation: u64,
}

impl IMFSourceReaderCallback_Impl for ReaderCallback_Impl {
    fn OnReadSample(
        &self,
        status: HRESULT,
        _stream: u32,
        flags: u32,
        timestamp: i64,
        sample: Ref<IMFSample>,
    ) -> windows_core::Result<()> {
        if !self.state.is_current(self.expected_generation) {
            return Ok(());
        }
        if status.is_err() {
            self.state.set_failure(format!(
                "webcam sample failed: {}",
                windows_core::Error::from_hresult(status)
            ));
        } else if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            self.state
                .set_failure("webcam source reader reported a capture error");
        }
        if status.is_ok()
            && flags & MF_SOURCE_READERF_ERROR.0 as u32 == 0
            && let Some(sample) = sample.as_ref()
        {
            // SAFETY: the sample and buffer remain live for this callback.
            if let Ok(buffer) = unsafe { sample.ConvertToContiguousBuffer() } {
                let mut bytes = core::ptr::null_mut();
                let mut length = 0;
                // SAFETY: all output pointers are writable; Unlock balances Lock.
                if unsafe { buffer.Lock(&mut bytes, None, Some(&mut length)) }.is_ok() {
                    let size = self
                        .state
                        .format
                        .lock()
                        .map_or(PIPELINE_SIZE, |format| *format);
                    let required = (size.width * size.height * 4) as usize;
                    if length as usize >= required {
                        // SAFETY: the locked buffer contains at least `required` bytes.
                        let pixels = unsafe { core::slice::from_raw_parts(bytes, required) };
                        let sequence = self.state.sequence.fetch_add(1, Ordering::Relaxed) + 1;
                        if let Ok(frame) = Frame::new(
                            pixels.to_vec().into(),
                            size,
                            size.width * 4,
                            sequence,
                            timestamp,
                            Instant::now(),
                        ) && self.state.generation.load(Ordering::Acquire)
                            == self.expected_generation
                            && let Ok(mut latest) = self.state.latest.lock()
                        {
                            *latest = Some(Arc::new(frame));
                            if let Ok(mut failure) = self.state.failure.lock() {
                                *failure = None;
                            }
                        }
                    } else {
                        self.state.set_failure(format!(
                            "webcam returned a short frame buffer: {length} bytes for {}×{} RGB32",
                            size.width, size.height
                        ));
                    }
                    // SAFETY: this callback successfully locked the buffer.
                    let _ = unsafe { buffer.Unlock() };
                }
            }
        }
        self.state.request_next(self.expected_generation);
        Ok(())
    }

    fn OnFlush(&self, _stream: u32) -> windows_core::Result<()> {
        Ok(())
    }

    fn OnEvent(&self, _stream: u32, _event: Ref<IMFMediaEvent>) -> windows_core::Result<()> {
        Ok(())
    }
}

pub struct MediaFoundationVideoInput {
    state: Arc<CaptureState>,
    callback: Option<IMFSourceReaderCallback>,
    initialization_error: Option<String>,
    mf_started: bool,
    com_initialized: bool,
}

// SAFETY: the source reader runs in asynchronous mode. Interface access and
// frame state that cross callback threads are guarded by mutexes.
unsafe impl Send for MediaFoundationVideoInput {}

impl Default for MediaFoundationVideoInput {
    fn default() -> Self {
        // SAFETY: this adapter is created, used, and dropped on its owning runtime thread.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok();
        let com_initialized = com.is_ok();
        let (mf_started, initialization_error) = match com {
            Ok(()) => {
                // SAFETY: balanced by MFShutdown in Drop.
                match unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) } {
                    Ok(()) => (true, None),
                    Err(error) => (
                        false,
                        Some(format!("could not initialize Media Foundation: {error}")),
                    ),
                }
            }
            Err(error) => (false, Some(format!("could not initialize COM: {error}"))),
        };
        Self {
            state: Arc::new(CaptureState::default()),
            callback: None,
            initialization_error,
            mf_started,
            com_initialized,
        }
    }
}

impl VideoInput for MediaFoundationVideoInput {
    fn enumerate(&self) -> Result<Vec<InputDevice>, String> {
        if let Some(error) = &self.initialization_error {
            return Err(error.clone());
        }
        enumerate_video_devices()
    }

    fn start(&mut self, device_id: &str) -> Result<(), String> {
        if let Some(error) = &self.initialization_error {
            return Err(error.clone());
        }
        if device_id.is_empty() {
            return Err("video source identifier is empty".into());
        }
        self.stop();
        let mut source_attributes = None;
        let mut reader_attributes = None;
        (|| -> windows_core::Result<()> {
            // SAFETY: all outputs are writable and the runtime initialized MF.
            unsafe {
                MFCreateAttributes(&mut source_attributes, 2)?;
                MFCreateAttributes(&mut reader_attributes, 2)?;
            }
            Ok(())
        })()
        .map_err(|error| format!("could not create video capture attributes: {error}"))?;
        let source_attributes = source_attributes.expect("attributes were created");
        let reader_attributes = reader_attributes.expect("attributes were created");
        let id: Vec<u16> = device_id.encode_utf16().chain([0]).collect();
        (|| -> windows_core::Result<()> {
            // SAFETY: all attributes and the terminated identifier are valid.
            unsafe {
                source_attributes.SetGUID(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                )?;
                source_attributes.SetString(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                    PCWSTR(id.as_ptr()),
                )?;
            }
            Ok(())
        })()
        .map_err(|error| format!("could not configure video source: {error}"))?;
        let source = unsafe { MFCreateDeviceSource(&source_attributes) }
            .map_err(|error| format!("could not open video source: {error}"))?;
        let expected_generation = self.state.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let callback: IMFSourceReaderCallback = ReaderCallback {
            state: Arc::clone(&self.state),
            expected_generation,
        }
        .into();
        (|| -> windows_core::Result<()> {
            // SAFETY: reader attributes and callback remain live.
            unsafe {
                reader_attributes.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)?;
                reader_attributes
                    .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
            }
            Ok(())
        })()
        .map_err(|error| format!("could not configure video reader: {error}"))?;
        let reader = unsafe { MFCreateSourceReaderFromMediaSource(&source, &reader_attributes) }
            .map_err(|error| format!("could not create video reader: {error}"))?;
        let output: IMFMediaType = unsafe { MFCreateMediaType() }
            .map_err(|error| format!("could not create video output type: {error}"))?;
        (|| -> windows_core::Result<()> {
            let frame_size =
                (u64::from(PIPELINE_SIZE.width) << 32) | u64::from(PIPELINE_SIZE.height);
            let frame_rate = (30u64 << 32) | 1;
            // SAFETY: reader, media type, and attributes are live.
            unsafe {
                output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                output.SetUINT64(&MF_MT_FRAME_SIZE, frame_size)?;
                output.SetUINT64(&MF_MT_FRAME_RATE, frame_rate)?;
                output.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                output.SetUINT32(&MF_MT_DEFAULT_STRIDE, PIPELINE_SIZE.width * 4)?;
                reader.SetCurrentMediaType(STREAM, None, &output)?;
            }
            Ok(())
        })()
        .map_err(|error| format!("could not negotiate RGB32 1280x720@30 webcam output: {error}"))?;
        let current = unsafe { reader.GetCurrentMediaType(STREAM) }
            .map_err(|error| format!("could not read negotiated webcam format: {error}"))?;
        let frame_size = unsafe { current.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map_err(|error| format!("negotiated webcam format has no frame size: {error}"))?;
        let size = Size::new((frame_size >> 32) as u32, frame_size as u32);
        if size.width == 0 || size.height == 0 {
            return Err("negotiated webcam format has empty dimensions".into());
        }
        *self
            .state
            .format
            .lock()
            .map_err(|_| "webcam format state is poisoned")? = size;
        if let Ok(mut failure) = self.state.failure.lock() {
            *failure = None;
        }
        *self
            .state
            .reader
            .lock()
            .map_err(|_| "video reader state is poisoned")? = Some(reader);
        self.state.running.store(true, Ordering::Release);
        self.callback = Some(callback);
        self.state.request_next(expected_generation);
        Ok(())
    }

    fn stop(&mut self) {
        self.state.generation.fetch_add(1, Ordering::AcqRel);
        self.state.running.store(false, Ordering::Release);
        if let Ok(mut reader) = self.state.reader.lock()
            && let Some(reader) = reader.take()
        {
            // SAFETY: this reader was created by this adapter.
            let _ = unsafe { reader.Flush(STREAM) };
        }
        self.callback = None;
        if let Ok(mut latest) = self.state.latest.lock() {
            *latest = None;
        }
    }

    fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.state
            .latest
            .lock()
            .ok()
            .and_then(|frame| frame.clone())
    }
}

impl MediaFoundationVideoInput {
    pub fn last_error(&self) -> Option<String> {
        self.state
            .failure
            .lock()
            .ok()
            .and_then(|failure| failure.clone())
    }
}

impl Drop for MediaFoundationVideoInput {
    fn drop(&mut self) {
        self.stop();
        if self.mf_started {
            // SAFETY: balances this adapter's successful MFStartup.
            let _ = unsafe { MFShutdown() };
        }
        if self.com_initialized {
            // SAFETY: balances this adapter's successful CoInitializeEx.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_generation_rejects_callbacks_from_stopped_reader() {
        let state = CaptureState::default();
        let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
        state.running.store(true, Ordering::Release);
        assert!(state.is_current(generation));
        state.generation.fetch_add(1, Ordering::AcqRel);
        assert!(!state.is_current(generation));
    }
}
