use super::OBJECTS;
use super::pipe_reader::PipeReader;
use asc_core::{PIPELINE_SIZE, SharedFrameCache, off_frame_pixels};
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
    MFGetSystemTime, MFMediaType_Video, MFVideoFormat_NV12, MFVideoFormat_RGB32,
    MFVideoInterlace_Progressive,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_core::{Error, GUID, HRESULT, IUnknown, Interface, Ref, implement};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const STRIDE: u32 = WIDTH * 4;
const FRAME_BYTES: u32 = STRIDE * HEIGHT;
const NV12_FRAME_BYTES: u32 = WIDTH * HEIGHT * 3 / 2;
const FRAME_DURATION_100NS: i64 = 10_000_000 / 30;

fn create_video_type(
    subtype: &GUID,
    stride: u32,
    sample_bytes: u32,
    bits_per_pixel: u32,
) -> windows_core::Result<windows::Win32::Media::MediaFoundation::IMFMediaType> {
    // SAFETY: Media Foundation returns an owned, initially empty media type.
    let media_type = unsafe { MFCreateMediaType()? };
    // SAFETY: the media type is exclusively initialized before publication.
    unsafe {
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        media_type.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (u64::from(WIDTH) << 32) | u64::from(HEIGHT),
        )?;
        media_type.SetUINT64(&MF_MT_FRAME_RATE, (30_u64 << 32) | 1)?;
        media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1_u64 << 32) | 1)?;
        media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        media_type.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        media_type.SetUINT32(&MF_MT_DEFAULT_STRIDE, stride)?;
        media_type.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        media_type.SetUINT32(&MF_MT_SAMPLE_SIZE, sample_bytes)?;
        media_type.SetUINT32(&MF_MT_AVG_BITRATE, WIDTH * HEIGHT * bits_per_pixel * 30)?;
    }
    Ok(media_type)
}

fn limited_y(r: i32, g: i32, b: i32) -> u8 {
    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(0, 255) as u8
}

fn limited_u(r: i32, g: i32, b: i32) -> u8 {
    (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8
}

fn limited_v(r: i32, g: i32, b: i32) -> u8 {
    (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8
}

fn bgra_to_nv12(output: &mut [u8], bgra: &[u8]) {
    let y_plane_bytes = (WIDTH * HEIGHT) as usize;
    let (y_plane, uv_plane) = output.split_at_mut(y_plane_bytes);
    for y in (0..HEIGHT as usize).step_by(2) {
        for x in (0..WIDTH as usize).step_by(2) {
            let mut red = 0;
            let mut green = 0;
            let mut blue = 0;
            for offset_y in 0..2 {
                for offset_x in 0..2 {
                    let pixel = ((y + offset_y) * WIDTH as usize + x + offset_x) * 4;
                    let b = i32::from(bgra[pixel]);
                    let g = i32::from(bgra[pixel + 1]);
                    let r = i32::from(bgra[pixel + 2]);
                    y_plane[(y + offset_y) * WIDTH as usize + x + offset_x] = limited_y(r, g, b);
                    red += r;
                    green += g;
                    blue += b;
                }
            }
            let uv = (y / 2) * WIDTH as usize + x;
            uv_plane[uv] = limited_u(red / 4, green / 4, blue / 4);
            uv_plane[uv + 1] = limited_v(red / 4, green / 4, blue / 4);
        }
    }
}

struct StreamState {
    parent: Option<IMFMediaSource>,
    state: MF_STREAM_STATE,
    shutdown: bool,
    next_time_100ns: i64,
}

#[implement(IMFMediaStream2)]
pub(super) struct MediaStream {
    events: IMFMediaEventQueue,
    descriptor: IMFStreamDescriptor,
    attributes: IMFAttributes,
    frames: Arc<Mutex<SharedFrameCache>>,
    off_bgra: Arc<[u8]>,
    off_nv12: Arc<[u8]>,
    _pipe_reader: PipeReader,
    state: Mutex<StreamState>,
}

impl MediaStream {
    pub(super) fn new(pipe_name: String) -> windows_core::Result<Self> {
        // SAFETY: Media Foundation constructors return owned COM interfaces.
        let events = unsafe { MFCreateEventQueue()? };
        let rgb32_type = create_video_type(&MFVideoFormat_RGB32, STRIDE, FRAME_BYTES, 32)?;
        let nv12_type = create_video_type(&MFVideoFormat_NV12, WIDTH, NV12_FRAME_BYTES, 12)?;
        // SAFETY: both media types remain alive for the descriptor construction call.
        let descriptor =
            unsafe { MFCreateStreamDescriptor(0, &[Some(rgb32_type.clone()), Some(nv12_type)])? };
        // SAFETY: descriptor owns its handler and advertised media types.
        unsafe {
            descriptor
                .GetMediaTypeHandler()?
                .SetCurrentMediaType(&rgb32_type)?
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
        let off_bgra = off_frame_pixels();
        let mut off_nv12 = vec![0; NV12_FRAME_BYTES as usize];
        bgra_to_nv12(&mut off_nv12, &off_bgra);
        Ok(Self {
            events,
            descriptor,
            attributes,
            frames,
            off_bgra,
            off_nv12: off_nv12.into(),
            _pipe_reader: pipe_reader,
            state: Mutex::new(StreamState {
                parent: None,
                state: MF_STREAM_STATE_STOPPED,
                shutdown: false,
                next_time_100ns: 0,
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
    pub(super) fn set_media_type(
        &self,
        media_type: &windows::Win32::Media::MediaFoundation::IMFMediaType,
    ) -> windows_core::Result<()> {
        let handler = unsafe { self.descriptor.GetMediaTypeHandler()? };
        unsafe {
            handler.IsMediaTypeSupported(media_type, None)?;
            handler.SetCurrentMediaType(media_type)
        }
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

    fn make_output_sample(&self, token: Option<IUnknown>) -> windows_core::Result<IMFSample> {
        let timestamp = {
            let mut state = self.state.lock().expect("stream state lock poisoned");
            if state.shutdown {
                return Err(Error::from_hresult(MF_E_SHUTDOWN));
            }
            if state.state != MF_STREAM_STATE_RUNNING {
                return Err(Error::from_hresult(MF_E_INVALIDREQUEST));
            }
            let timestamp = state.next_time_100ns;
            state.next_time_100ns = state.next_time_100ns.saturating_add(FRAME_DURATION_100NS);
            timestamp
        };
        let live_frame = self
            .frames
            .lock()
            .expect("shared frame cache lock poisoned")
            .latest(std::time::Instant::now())
            .filter(|frame| frame.size == PIPELINE_SIZE && frame.stride == STRIDE);
        // Consumers select one of the advertised stream types on the descriptor.
        let media_type = unsafe {
            self.descriptor
                .GetMediaTypeHandler()?
                .GetCurrentMediaType()?
        };
        let subtype = unsafe { media_type.GetGUID(&MF_MT_SUBTYPE)? };
        let frame_bytes = if subtype == MFVideoFormat_NV12 {
            NV12_FRAME_BYTES
        } else if subtype == MFVideoFormat_RGB32 {
            FRAME_BYTES
        } else {
            return Err(Error::from_hresult(MF_E_INVALIDREQUEST));
        };
        // SAFETY: Media Foundation constructors return owned COM interfaces.
        let buffer = unsafe { MFCreateMemoryBuffer(frame_bytes)? };
        let mut destination = core::ptr::null_mut();
        let mut maximum = 0;
        // SAFETY: Lock initializes destination and maximum for this owned buffer.
        unsafe { buffer.Lock(&mut destination, Some(&mut maximum), None)? };
        if destination.is_null() || maximum < frame_bytes {
            // SAFETY: balances the successful Lock call.
            let _ = unsafe { buffer.Unlock() };
            return Err(Error::from_hresult(E_POINTER));
        }
        // SAFETY: Lock guarantees frame_bytes writable bytes and alignment is not assumed.
        unsafe {
            let output = core::slice::from_raw_parts_mut(destination, frame_bytes as usize);
            if subtype == MFVideoFormat_NV12 {
                if let Some(frame) = live_frame {
                    bgra_to_nv12(output, frame.pixels());
                } else {
                    output.copy_from_slice(&self.off_nv12);
                }
            } else if let Some(frame) = live_frame {
                output.copy_from_slice(frame.pixels());
            } else {
                output.copy_from_slice(&self.off_bgra);
            }
            buffer.Unlock()?;
            buffer.SetCurrentLength(frame_bytes)?;
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
        let sample = self.make_output_sample(token.cloned())?;
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
    fn rgb32_is_default_and_nv12_remains_selectable() -> windows_core::Result<()> {
        let _test_lock = super::super::TEST_LOCK
            .lock()
            .expect("media-source test lock poisoned");
        // SAFETY: initializes Media Foundation for this test process.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
        let _foundation = MediaFoundation;
        let stream =
            MediaStream::new(r"\\.\pipe\AutomaticScreenCameraRust.Nonexistent.Test".into())?;
        let handler = unsafe { stream.descriptor.GetMediaTypeHandler()? };
        assert_eq!(unsafe { handler.GetMediaTypeCount()? }, 2);
        let current = unsafe { handler.GetCurrentMediaType()? };
        assert_eq!(
            unsafe { current.GetGUID(&MF_MT_SUBTYPE)? },
            MFVideoFormat_RGB32
        );
        stream.start()?;
        let first = stream.make_output_sample(None)?;
        // SAFETY: the sample owns one contiguous RGB32 buffer.
        let buffer = unsafe { first.ConvertToContiguousBuffer()? };
        let mut bytes = core::ptr::null_mut();
        let mut length = 0;
        // SAFETY: Lock initializes bytes and length for this live buffer.
        unsafe { buffer.Lock(&mut bytes, None, Some(&mut length))? };
        assert_eq!(length, FRAME_BYTES);
        let pixels = unsafe { core::slice::from_raw_parts(bytes, length as usize) };
        assert_eq!(pixels, off_frame_pixels().as_ref());
        // SAFETY: balances the successful Lock.
        unsafe { buffer.Unlock()? };

        let nv12 = unsafe { handler.GetMediaTypeByIndex(1)? };
        assert_eq!(unsafe { nv12.GetGUID(&MF_MT_SUBTYPE)? }, MFVideoFormat_NV12);
        stream.set_media_type(&nv12)?;
        let second = stream.make_output_sample(None)?;
        assert_eq!(
            unsafe { second.GetSampleTime()? } - unsafe { first.GetSampleTime()? },
            FRAME_DURATION_100NS
        );
        let buffer = unsafe { second.ConvertToContiguousBuffer()? };
        unsafe { buffer.Lock(&mut bytes, None, Some(&mut length))? };
        assert_eq!(length, NV12_FRAME_BYTES);
        let pixels = unsafe { core::slice::from_raw_parts(bytes, length as usize) };
        let mut expected = vec![0; NV12_FRAME_BYTES as usize];
        bgra_to_nv12(&mut expected, &off_frame_pixels());
        assert_eq!(pixels, expected);
        unsafe { buffer.Unlock()? };
        stream.stop(false)?;
        stream.shutdown();
        Ok(())
    }
}
