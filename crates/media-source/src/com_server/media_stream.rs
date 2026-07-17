use super::OBJECTS;
use super::pipe_reader::PipeReader;
use asc_core::{PIPELINE_SIZE, SharedFrameCache};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{E_POINTER, S_OK};
use windows::Win32::Media::KernelStreaming::PINNAME_VIDEO_CAPTURE;
use windows::Win32::Media::MediaFoundation::{
    IMFAsyncCallback, IMFAsyncResult, IMFAttributes, IMFMediaEvent, IMFMediaEventGenerator_Impl,
    IMFMediaEventQueue, IMFMediaSource, IMFMediaStream_Impl, IMFMediaStream2, IMFMediaStream2_Impl,
    IMFSample, IMFStreamDescriptor, MEMediaSample, MEStreamStarted, MEStreamStopped,
    MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MF_DEVICESTREAM_FRAMESERVER_SHARED,
    MF_DEVICESTREAM_STREAM_CATEGORY, MF_DEVICESTREAM_STREAM_ID, MF_E_INVALID_STATE_TRANSITION,
    MF_E_INVALIDREQUEST, MF_E_SHUTDOWN, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FIXED_SIZE_SAMPLES, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SAMPLE_SIZE,
    MF_MT_SUBTYPE, MF_STREAM_STATE, MF_STREAM_STATE_PAUSED, MF_STREAM_STATE_RUNNING,
    MF_STREAM_STATE_STOPPED, MFCreateAttributes, MFCreateEventQueue, MFCreateMediaType,
    MFCreateMemoryBuffer, MFCreateSample, MFCreateStreamDescriptor, MFFrameSourceTypes_Color,
    MFGetSystemTime, MFMediaType_Video, MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_core::{Error, GUID, HRESULT, IUnknown, Interface, Ref, implement};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const STRIDE: u32 = WIDTH * 4;
const FRAME_BYTES: u32 = STRIDE * HEIGHT;
const FRAME_DURATION_100NS: i64 = 10_000_000 / 30;

struct StreamState {
    parent: Option<IMFMediaSource>,
    state: MF_STREAM_STATE,
    shutdown: bool,
    next_time_100ns: i64,
    placeholder_bgra: u32,
}

#[implement(IMFMediaStream2)]
pub(super) struct MediaStream {
    events: IMFMediaEventQueue,
    descriptor: IMFStreamDescriptor,
    attributes: IMFAttributes,
    frames: Arc<Mutex<SharedFrameCache>>,
    _pipe_reader: PipeReader,
    state: Mutex<StreamState>,
}

impl MediaStream {
    pub(super) fn new(placeholder_bgra: u32, pipe_name: String) -> windows_core::Result<Self> {
        // SAFETY: Media Foundation constructors return owned COM interfaces.
        let events = unsafe { MFCreateEventQueue()? };
        // SAFETY: Media Foundation constructors return owned COM interfaces.
        let media_type = unsafe { MFCreateMediaType()? };
        // SAFETY: the media type is exclusively initialized before publication.
        unsafe {
            media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            media_type.SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(WIDTH) << 32) | u64::from(HEIGHT),
            )?;
            media_type.SetUINT64(&MF_MT_FRAME_RATE, (30_u64 << 32) | 1)?;
            media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1_u64 << 32) | 1)?;
            media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            media_type.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
            media_type.SetUINT32(&MF_MT_DEFAULT_STRIDE, STRIDE)?;
            media_type.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
            media_type.SetUINT32(&MF_MT_SAMPLE_SIZE, FRAME_BYTES)?;
            media_type.SetUINT32(&MF_MT_AVG_BITRATE, WIDTH * HEIGHT * 32 * 30)?;
        }
        // SAFETY: the one-element media-type array remains alive for the call.
        let descriptor = unsafe { MFCreateStreamDescriptor(0, &[Some(media_type.clone())])? };
        // SAFETY: descriptor owns its handler and media type.
        unsafe {
            descriptor
                .GetMediaTypeHandler()?
                .SetCurrentMediaType(&media_type)?
        };
        let mut attributes = None;
        // SAFETY: MFCreateAttributes initializes the provided COM out slot.
        unsafe { MFCreateAttributes(&mut attributes, 4)? };
        let attributes = attributes.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        let descriptor_attributes: IMFAttributes = descriptor.cast()?;
        for store in [&attributes, &descriptor_attributes] {
            // SAFETY: both stores are exclusively initialized before publication.
            unsafe {
                store.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
                store.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
                store.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
                store.SetUINT32(
                    &MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES,
                    MFFrameSourceTypes_Color.0 as u32,
                )?;
            }
        }
        OBJECTS.fetch_add(1, Ordering::Relaxed);
        let frames = Arc::new(Mutex::new(SharedFrameCache::default()));
        let pipe_reader = PipeReader::start(pipe_name, Arc::clone(&frames));
        Ok(Self {
            events,
            descriptor,
            attributes,
            frames,
            _pipe_reader: pipe_reader,
            state: Mutex::new(StreamState {
                parent: None,
                state: MF_STREAM_STATE_STOPPED,
                shutdown: false,
                next_time_100ns: 0,
                placeholder_bgra: placeholder_bgra | 0xff00_0000,
            }),
        })
    }

    pub(super) fn attach_parent(&self, parent: IMFMediaSource) {
        self.state
            .lock()
            .expect("stream state lock poisoned")
            .parent = Some(parent);
    }

    pub(super) fn descriptor(&self) -> IMFStreamDescriptor {
        self.descriptor.clone()
    }
    pub(super) fn attributes(&self) -> IMFAttributes {
        self.attributes.clone()
    }
    pub(super) fn start(&self) -> windows_core::Result<()> {
        let mut state = self.state.lock().expect("stream state lock poisoned");
        if state.shutdown {
            return Err(Error::from_hresult(MF_E_SHUTDOWN));
        }
        state.state = MF_STREAM_STATE_RUNNING;
        // SAFETY: MFGetSystemTime returns the current monotonic MF clock value.
        state.next_time_100ns = unsafe { MFGetSystemTime() };
        drop(state);
        // SAFETY: queue and GUID pointer remain valid for the call.
        unsafe {
            self.events.QueueEventParamVar(
                MEStreamStarted.0 as u32,
                &GUID::zeroed(),
                S_OK,
                core::ptr::null(),
            )
        }
    }

    pub(super) fn stop(&self, send_event: bool) -> windows_core::Result<()> {
        let mut state = self.state.lock().expect("stream state lock poisoned");
        if state.shutdown {
            return Err(Error::from_hresult(MF_E_SHUTDOWN));
        }
        state.state = MF_STREAM_STATE_STOPPED;
        drop(state);
        if send_event {
            // SAFETY: queue and GUID pointer remain valid for the call.
            unsafe {
                self.events.QueueEventParamVar(
                    MEStreamStopped.0 as u32,
                    &GUID::zeroed(),
                    S_OK,
                    core::ptr::null(),
                )
            }
        } else {
            Ok(())
        }
    }

    pub(super) fn shutdown(&self) {
        let mut state = self.state.lock().expect("stream state lock poisoned");
        if state.shutdown {
            return;
        }
        state.shutdown = true;
        state.state = MF_STREAM_STATE_STOPPED;
        state.parent = None;
        drop(state);
        // SAFETY: shutting down an owned event queue is idempotent for our state machine.
        let _ = unsafe { self.events.Shutdown() };
    }

    fn make_placeholder_sample(&self, token: Option<IUnknown>) -> windows_core::Result<IMFSample> {
        let (timestamp, color) = {
            let mut state = self.state.lock().expect("stream state lock poisoned");
            if state.shutdown {
                return Err(Error::from_hresult(MF_E_SHUTDOWN));
            }
            if state.state != MF_STREAM_STATE_RUNNING {
                return Err(Error::from_hresult(MF_E_INVALIDREQUEST));
            }
            let timestamp = state.next_time_100ns;
            state.next_time_100ns = state.next_time_100ns.saturating_add(FRAME_DURATION_100NS);
            (timestamp, state.placeholder_bgra)
        };
        let live_frame = self
            .frames
            .lock()
            .expect("shared frame cache lock poisoned")
            .latest(std::time::Instant::now())
            .filter(|frame| frame.size == PIPELINE_SIZE && frame.stride == STRIDE);
        // SAFETY: Media Foundation constructors return owned COM interfaces.
        let buffer = unsafe { MFCreateMemoryBuffer(FRAME_BYTES)? };
        let mut destination = core::ptr::null_mut();
        let mut maximum = 0;
        // SAFETY: Lock initializes destination and maximum for this owned buffer.
        unsafe { buffer.Lock(&mut destination, Some(&mut maximum), None)? };
        if destination.is_null() || maximum < FRAME_BYTES {
            // SAFETY: balances the successful Lock call.
            let _ = unsafe { buffer.Unlock() };
            return Err(Error::from_hresult(E_POINTER));
        }
        // SAFETY: Lock guarantees FRAME_BYTES writable bytes and alignment is not assumed.
        unsafe {
            let output = core::slice::from_raw_parts_mut(destination, FRAME_BYTES as usize);
            if let Some(frame) = live_frame {
                output.copy_from_slice(frame.pixels());
            } else {
                for pixel in output.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&color.to_le_bytes());
                }
            }
            buffer.Unlock()?;
            buffer.SetCurrentLength(FRAME_BYTES)?;
        }
        // SAFETY: Media Foundation constructors return owned COM interfaces.
        let sample = unsafe { MFCreateSample()? };
        // SAFETY: sample and buffer are valid owned interfaces.
        unsafe {
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(timestamp)?;
            sample.SetSampleDuration(FRAME_DURATION_100NS)?;
            if let Some(token) = token {
                sample.SetUnknown(
                    &windows::Win32::Media::MediaFoundation::MFSampleExtension_Token,
                    &token,
                )?;
            }
        }
        Ok(sample)
    }
}

impl Drop for MediaStream {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::Release);
    }
}

impl IMFMediaEventGenerator_Impl for MediaStream_Impl {
    fn GetEvent(
        &self,
        flags: windows::Win32::Media::MediaFoundation::MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
    ) -> windows_core::Result<IMFMediaEvent> {
        // SAFETY: delegates to the owned event queue.
        unsafe { self.events.GetEvent(flags.0) }
    }
    fn BeginGetEvent(
        &self,
        callback: Ref<IMFAsyncCallback>,
        state: Ref<IUnknown>,
    ) -> windows_core::Result<()> {
        // SAFETY: borrowed interfaces stay alive for the delegated call.
        unsafe { self.events.BeginGetEvent(callback.as_ref(), state.as_ref()) }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> windows_core::Result<IMFMediaEvent> {
        // SAFETY: borrowed result stays alive for the delegated call.
        unsafe { self.events.EndGetEvent(result.as_ref()) }
    }
    fn QueueEvent(
        &self,
        event_type: u32,
        extended_type: *const GUID,
        status: HRESULT,
        value: *const PROPVARIANT,
    ) -> windows_core::Result<()> {
        // SAFETY: forwards the exact Media Foundation event arguments.
        unsafe {
            self.events
                .QueueEventParamVar(event_type, extended_type, status, value)
        }
    }
}

impl IMFMediaStream_Impl for MediaStream_Impl {
    fn GetMediaSource(&self) -> windows_core::Result<IMFMediaSource> {
        let state = self.state.lock().expect("stream state lock poisoned");
        if state.shutdown {
            return Err(Error::from_hresult(MF_E_SHUTDOWN));
        }
        state
            .parent
            .clone()
            .ok_or_else(|| Error::from_hresult(MF_E_SHUTDOWN))
    }
    fn GetStreamDescriptor(&self) -> windows_core::Result<IMFStreamDescriptor> {
        if self
            .state
            .lock()
            .expect("stream state lock poisoned")
            .shutdown
        {
            return Err(Error::from_hresult(MF_E_SHUTDOWN));
        }
        Ok(self.descriptor.clone())
    }
    fn RequestSample(&self, token: Ref<IUnknown>) -> windows_core::Result<()> {
        let sample = self.make_placeholder_sample(token.cloned())?;
        // SAFETY: the sample remains alive for the queue call and is AddRef'd by the event.
        unsafe {
            self.events
                .QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), S_OK, &sample)
        }
    }
}

impl IMFMediaStream2_Impl for MediaStream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> windows_core::Result<()> {
        if value != MF_STREAM_STATE_STOPPED
            && value != MF_STREAM_STATE_RUNNING
            && value != MF_STREAM_STATE_PAUSED
        {
            return Err(Error::from_hresult(MF_E_INVALID_STATE_TRANSITION));
        }
        let mut state = self.state.lock().expect("stream state lock poisoned");
        if state.shutdown {
            return Err(Error::from_hresult(MF_E_SHUTDOWN));
        }
        if value == MF_STREAM_STATE_PAUSED && state.state != MF_STREAM_STATE_RUNNING {
            return Err(Error::from_hresult(MF_E_INVALID_STATE_TRANSITION));
        }
        state.state = value;
        Ok(())
    }
    fn GetStreamState(&self) -> windows_core::Result<MF_STREAM_STATE> {
        let state = self.state.lock().expect("stream state lock poisoned");
        if state.shutdown {
            return Err(Error::from_hresult(MF_E_SHUTDOWN));
        }
        Ok(state.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::MediaFoundation::{
        MF_VERSION, MFSTARTUP_FULL, MFShutdown, MFStartup,
    };

    struct MediaFoundation;

    impl Drop for MediaFoundation {
        fn drop(&mut self) {
            // SAFETY: balances the successful MFStartup in this test.
            let _ = unsafe { MFShutdown() };
        }
    }

    #[test]
    fn placeholder_samples_have_fixed_color_and_increasing_timestamps() -> windows_core::Result<()>
    {
        let _test_lock = super::super::TEST_LOCK
            .lock()
            .expect("media-source test lock poisoned");
        // SAFETY: initializes Media Foundation for this test process.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
        let _foundation = MediaFoundation;
        let stream = MediaStream::new(
            0xff33_2211,
            r"\\.\pipe\AutomaticScreenCameraRust.Nonexistent.Test".into(),
        )?;
        stream.start()?;
        let first = stream.make_placeholder_sample(None)?;
        let second = stream.make_placeholder_sample(None)?;
        // SAFETY: both samples are live and initialized by make_placeholder_sample.
        let first_time = unsafe { first.GetSampleTime()? };
        let second_time = unsafe { second.GetSampleTime()? };
        assert_eq!(second_time - first_time, FRAME_DURATION_100NS);

        // SAFETY: the sample owns one contiguous RGB32 buffer.
        let buffer = unsafe { first.ConvertToContiguousBuffer()? };
        let mut bytes = core::ptr::null_mut();
        let mut length = 0;
        // SAFETY: Lock initializes bytes and length for this live buffer.
        unsafe { buffer.Lock(&mut bytes, None, Some(&mut length))? };
        assert_eq!(length, FRAME_BYTES);
        // SAFETY: the locked buffer exposes at least one complete pixel.
        let pixel = unsafe { core::slice::from_raw_parts(bytes, 4) };
        assert_eq!(pixel, 0xff33_2211_u32.to_le_bytes());
        // SAFETY: balances the successful Lock.
        unsafe { buffer.Unlock()? };
        stream.stop(false)?;
        stream.shutdown();
        Ok(())
    }
}
