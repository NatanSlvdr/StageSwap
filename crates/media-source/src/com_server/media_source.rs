use super::OBJECTS;
use super::media_stream::MediaStream;
use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::{E_INVALIDARG, E_POINTER, S_OK};
use windows::Win32::Media::KernelStreaming::{IKsControl, IKsControl_Impl, KSIDENTIFIER};
use windows::Win32::Media::MediaFoundation::{
    IMFAsyncCallback, IMFAsyncResult, IMFAttributes, IMFGetService, IMFGetService_Impl,
    IMFMediaEvent, IMFMediaEventGenerator_Impl, IMFMediaEventQueue, IMFMediaSource,
    IMFMediaSource_Impl, IMFMediaSourceEx, IMFMediaSourceEx_Impl, IMFMediaStream2,
    IMFPresentationDescriptor, MENewStream, MESourceStarted, MESourceStopped, MEUpdatedStream,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_E_INVALID_STATE_TRANSITION,
    MF_E_INVALIDSTREAMNUMBER, MF_E_SHUTDOWN, MF_E_UNSUPPORTED_SERVICE,
    MF_E_UNSUPPORTED_TIME_FORMAT, MFCreateAttributes, MFCreateEventQueue,
    MFCreatePresentationDescriptor, MFMEDIASOURCE_IS_LIVE,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_core::{ComObject, Error, GUID, HRESULT, IUnknown, Ref, implement, w};

const PLACEHOLDER_ATTRIBUTE: GUID = GUID::from_u128(0x05cd1551_bfc8_4276_8e0b_70ba4065822e);
const PIPE_ATTRIBUTE: GUID = GUID::from_u128(0x905306dd_b9a3_4385_a273_606e05b3208b);
const ERROR_SET_NOT_FOUND_HRESULT: HRESULT = HRESULT(0x8007_0492_u32 as i32);

#[derive(Clone, Copy, Eq, PartialEq)]
enum Lifecycle {
    Stopped,
    Started,
    Shutdown,
}

struct SourceState {
    lifecycle: Lifecycle,
    stream_announced: bool,
}

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl)]
pub(super) struct MediaSource {
    events: IMFMediaEventQueue,
    attributes: IMFAttributes,
    descriptor: IMFPresentationDescriptor,
    stream: ComObject<MediaStream>,
    state: Mutex<SourceState>,
}

impl MediaSource {
    pub(super) fn create(activation: &IMFAttributes) -> windows_core::Result<IMFMediaSource> {
        // SAFETY: Media Foundation constructors return owned COM interfaces.
        let events = unsafe { MFCreateEventQueue()? };
        let mut attributes = None;
        // SAFETY: MFCreateAttributes initializes the provided COM out slot.
        unsafe { MFCreateAttributes(&mut attributes, 8)? };
        let attributes = attributes.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        // SAFETY: both attribute stores remain alive for the copy and initialization calls.
        unsafe {
            activation.CopyAllItems(&attributes)?;
            attributes.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )?;
            attributes.SetString(
                &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                w!("Automatic Screen Camera"),
            )?;
        }
        let placeholder =
            unsafe { activation.GetUINT32(&PLACEHOLDER_ATTRIBUTE) }.unwrap_or(0xff17_1719);
        let pipe_name = read_string_attribute(activation, &PIPE_ATTRIBUTE).unwrap_or_default();
        let stream = ComObject::new(MediaStream::new(placeholder, pipe_name)?);
        let stream_descriptor = stream.descriptor();
        // SAFETY: the descriptor array remains alive for the constructor call.
        let descriptor =
            unsafe { MFCreatePresentationDescriptor(Some(&[Some(stream_descriptor)]))? };
        // SAFETY: the presentation descriptor contains stream index zero.
        unsafe { descriptor.SelectStream(0)? };
        OBJECTS.fetch_add(1, Ordering::Relaxed);
        let object = ComObject::new(Self {
            events,
            attributes,
            descriptor,
            stream,
            state: Mutex::new(SourceState {
                lifecycle: Lifecycle::Stopped,
                stream_announced: false,
            }),
        });
        let extended = object.to_interface::<IMFMediaSourceEx>();
        let source: IMFMediaSource = extended.into();
        object.stream.attach_parent(source.clone());
        Ok(source)
    }

    fn ensure_running_object(&self) -> windows_core::Result<()> {
        if self
            .state
            .lock()
            .expect("source state lock poisoned")
            .lifecycle
            == Lifecycle::Shutdown
        {
            Err(Error::from_hresult(MF_E_SHUTDOWN))
        } else {
            Ok(())
        }
    }
}

fn read_string_attribute(
    attributes: &IMFAttributes,
    key: *const GUID,
) -> windows_core::Result<String> {
    // SAFETY: key identifies an attribute in the borrowed store.
    let length = unsafe { attributes.GetStringLength(key)? };
    let mut buffer = vec![0_u16; length as usize + 1];
    // SAFETY: buffer has the advertised UTF-16 length plus its terminator.
    unsafe { attributes.GetString(key, &mut buffer, None)? };
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

impl Drop for MediaSource {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::Release);
    }
}

impl IMFMediaEventGenerator_Impl for MediaSource_Impl {
    fn GetEvent(
        &self,
        flags: windows::Win32::Media::MediaFoundation::MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
    ) -> windows_core::Result<IMFMediaEvent> {
        self.ensure_running_object()?;
        // SAFETY: delegates to the owned event queue.
        unsafe { self.events.GetEvent(flags.0) }
    }
    fn BeginGetEvent(
        &self,
        callback: Ref<IMFAsyncCallback>,
        state: Ref<IUnknown>,
    ) -> windows_core::Result<()> {
        self.ensure_running_object()?;
        // SAFETY: borrowed interfaces stay alive for the delegated call.
        unsafe { self.events.BeginGetEvent(callback.as_ref(), state.as_ref()) }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> windows_core::Result<IMFMediaEvent> {
        self.ensure_running_object()?;
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
        self.ensure_running_object()?;
        // SAFETY: forwards the exact Media Foundation event arguments.
        unsafe {
            self.events
                .QueueEventParamVar(event_type, extended_type, status, value)
        }
    }
}

impl IMFMediaSource_Impl for MediaSource_Impl {
    fn GetCharacteristics(&self) -> windows_core::Result<u32> {
        self.ensure_running_object()?;
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> windows_core::Result<IMFPresentationDescriptor> {
        self.ensure_running_object()?;
        // SAFETY: Clone returns an independent presentation descriptor.
        unsafe { self.descriptor.Clone() }
    }

    fn Start(
        &self,
        presentation: Ref<IMFPresentationDescriptor>,
        time_format: *const GUID,
        start_position: *const PROPVARIANT,
    ) -> windows_core::Result<()> {
        presentation.ok()?;
        if start_position.is_null() {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        if !time_format.is_null() {
            // SAFETY: non-null time_format points to a GUID for this COM call.
            if unsafe { *time_format } != GUID::zeroed() {
                return Err(Error::from_hresult(MF_E_UNSUPPORTED_TIME_FORMAT));
            }
        }
        let (event_type, stream_interface) = {
            let state = self.state.lock().expect("source state lock poisoned");
            if state.lifecycle == Lifecycle::Shutdown {
                return Err(Error::from_hresult(MF_E_SHUTDOWN));
            }
            let event_type = if state.stream_announced {
                MEUpdatedStream
            } else {
                MENewStream
            };
            (event_type, self.stream.to_interface::<IMFMediaStream2>())
        };
        self.stream.start()?;
        // SAFETY: interfaces and GUID pointers remain valid for each queue call.
        let queued = unsafe {
            self.events
                .QueueEventParamUnk(
                    event_type.0 as u32,
                    &GUID::zeroed(),
                    S_OK,
                    &stream_interface,
                )
                .and_then(|()| {
                    self.events.QueueEventParamVar(
                        MESourceStarted.0 as u32,
                        &GUID::zeroed(),
                        S_OK,
                        core::ptr::null(),
                    )
                })
        };
        if let Err(error) = queued {
            let _ = self.stream.stop(false);
            return Err(error);
        }
        let mut state = self.state.lock().expect("source state lock poisoned");
        state.stream_announced = true;
        state.lifecycle = Lifecycle::Started;
        Ok(())
    }

    fn Stop(&self) -> windows_core::Result<()> {
        {
            let state = self.state.lock().expect("source state lock poisoned");
            if state.lifecycle == Lifecycle::Shutdown {
                return Err(Error::from_hresult(MF_E_SHUTDOWN));
            }
            if state.lifecycle != Lifecycle::Started {
                return Err(Error::from_hresult(MF_E_INVALID_STATE_TRANSITION));
            }
        }
        self.stream.stop(true)?;
        // SAFETY: queue and GUID pointer remain valid for the call.
        let event_result = unsafe {
            self.events.QueueEventParamVar(
                MESourceStopped.0 as u32,
                &GUID::zeroed(),
                S_OK,
                core::ptr::null(),
            )
        };
        self.state
            .lock()
            .expect("source state lock poisoned")
            .lifecycle = Lifecycle::Stopped;
        event_result
    }

    fn Pause(&self) -> windows_core::Result<()> {
        self.ensure_running_object()?;
        Err(Error::from_hresult(MF_E_INVALID_STATE_TRANSITION))
    }

    fn Shutdown(&self) -> windows_core::Result<()> {
        {
            let mut state = self.state.lock().expect("source state lock poisoned");
            if state.lifecycle == Lifecycle::Shutdown {
                return Ok(());
            }
            state.lifecycle = Lifecycle::Shutdown;
        }
        self.stream.shutdown();
        // SAFETY: shuts down the owned event queue once according to our state machine.
        unsafe { self.events.Shutdown() }
    }
}

impl IMFMediaSourceEx_Impl for MediaSource_Impl {
    fn GetSourceAttributes(&self) -> windows_core::Result<IMFAttributes> {
        self.ensure_running_object()?;
        Ok(self.attributes.clone())
    }
    fn GetStreamAttributes(&self, stream_id: u32) -> windows_core::Result<IMFAttributes> {
        self.ensure_running_object()?;
        if stream_id != 0 {
            return Err(Error::from_hresult(MF_E_INVALIDSTREAMNUMBER));
        }
        Ok(self.stream.attributes())
    }
    fn SetD3DManager(&self, _manager: Ref<IUnknown>) -> windows_core::Result<()> {
        self.ensure_running_object()
    }
}

impl IMFGetService_Impl for MediaSource_Impl {
    fn GetService(
        &self,
        _service: *const GUID,
        _interface_id: *const GUID,
        object: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        if object.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        // SAFETY: the checked COM out pointer is writable for this call.
        unsafe { object.write(core::ptr::null_mut()) };
        Err(Error::from_hresult(MF_E_UNSUPPORTED_SERVICE))
    }
}

impl IKsControl_Impl for MediaSource_Impl {
    fn KsProperty(
        &self,
        _property: *const KSIDENTIFIER,
        _property_length: u32,
        _data: *mut c_void,
        _data_length: u32,
        bytes_returned: *mut u32,
    ) -> windows_core::Result<()> {
        if !bytes_returned.is_null() {
            unsafe { bytes_returned.write(0) };
        }
        Err(Error::from_hresult(ERROR_SET_NOT_FOUND_HRESULT))
    }
    fn KsMethod(
        &self,
        _method: *const KSIDENTIFIER,
        _method_length: u32,
        _data: *mut c_void,
        _data_length: u32,
        bytes_returned: *mut u32,
    ) -> windows_core::Result<()> {
        if !bytes_returned.is_null() {
            unsafe { bytes_returned.write(0) };
        }
        Err(Error::from_hresult(ERROR_SET_NOT_FOUND_HRESULT))
    }
    fn KsEvent(
        &self,
        _event: *const KSIDENTIFIER,
        _event_length: u32,
        _data: *mut c_void,
        _data_length: u32,
        bytes_returned: *mut u32,
    ) -> windows_core::Result<()> {
        if !bytes_returned.is_null() {
            unsafe { bytes_returned.write(0) };
        }
        Err(Error::from_hresult(ERROR_SET_NOT_FOUND_HRESULT))
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
    fn source_starts_stops_and_shuts_down_with_rgb32_descriptor() -> windows_core::Result<()> {
        let _test_lock = super::super::TEST_LOCK
            .lock()
            .expect("media-source test lock poisoned");
        // SAFETY: initializes Media Foundation for the test process.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
        let _foundation = MediaFoundation;
        let mut activation = None;
        // SAFETY: initializes the attributes out slot.
        unsafe { MFCreateAttributes(&mut activation, 4)? };
        let activation = activation.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        let source = MediaSource::create(&activation)?;
        // SAFETY: exercises the COM contract on valid owned interfaces.
        unsafe {
            assert_eq!(source.GetCharacteristics()?, MFMEDIASOURCE_IS_LIVE.0 as u32);
            let descriptor = source.CreatePresentationDescriptor()?;
            let start = PROPVARIANT::default();
            source.Start(&descriptor, core::ptr::null(), &start)?;
            source.Stop()?;
            source.Shutdown()?;
        }
        Ok(())
    }
}
