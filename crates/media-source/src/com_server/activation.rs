use super::OBJECTS;
use super::media_source::MediaSource;
use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFActivate_Impl, IMFAttributes, IMFAttributes_Impl, MF_ATTRIBUTE_TYPE,
    MF_ATTRIBUTES_MATCH_TYPE, MFCreateAttributes,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_core::{BOOL, Error, GUID, IUnknown, Interface, PCWSTR, PWSTR, Ref, implement};

#[implement(IMFActivate)]
pub(super) struct Activation {
    attributes: IMFAttributes,
    active_source: Mutex<Option<windows::Win32::Media::MediaFoundation::IMFMediaSource>>,
}

impl Activation {
    pub(super) fn new() -> windows_core::Result<Self> {
        let mut attributes = None;
        // SAFETY: MFCreateAttributes initializes the provided COM out slot.
        unsafe { MFCreateAttributes(&mut attributes, 4)? };
        let attributes = attributes.ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?;
        OBJECTS.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            attributes,
            active_source: Mutex::new(None),
        })
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        if let Ok(active) = self.active_source.get_mut()
            && let Some(source) = active.take()
        {
            // SAFETY: best-effort cycle breaking during final COM release.
            let _ = unsafe { source.Shutdown() };
        }
        OBJECTS.fetch_sub(1, Ordering::Release);
    }
}

impl IMFActivate_Impl for Activation_Impl {
    fn ActivateObject(
        &self,
        interface_id: *const GUID,
        object: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        if object.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        // SAFETY: the checked COM out pointer is writable for this call.
        unsafe { object.write(core::ptr::null_mut()) };
        if interface_id.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let mut active = self
            .active_source
            .lock()
            .expect("activation source lock poisoned");
        if active.is_none() {
            *active = Some(MediaSource::create(&self.attributes)?);
        }
        // SAFETY: interface_id and object satisfy QueryInterface's COM contract.
        unsafe {
            active
                .as_ref()
                .expect("source initialized")
                .query(interface_id, object)
        }
        .ok()
    }

    fn ShutdownObject(&self) -> windows_core::Result<()> {
        let source = self
            .active_source
            .lock()
            .expect("activation source lock poisoned")
            .take();
        if let Some(source) = source {
            // SAFETY: invokes Shutdown on the owned media source interface.
            unsafe { source.Shutdown()? };
        }
        Ok(())
    }

    fn DetachObject(&self) -> windows_core::Result<()> {
        self.active_source
            .lock()
            .expect("activation source lock poisoned")
            .take();
        Ok(())
    }
}

impl IMFAttributes_Impl for Activation_Impl {
    fn GetItem(&self, key: *const GUID, value: *mut PROPVARIANT) -> windows_core::Result<()> {
        // SAFETY: forwards the exact IMFAttributes ABI arguments.
        unsafe { self.attributes.GetItem(key, Some(value)) }
    }

    fn GetItemType(&self, key: *const GUID) -> windows_core::Result<MF_ATTRIBUTE_TYPE> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.GetItemType(key) }
    }

    fn CompareItem(
        &self,
        key: *const GUID,
        value: *const PROPVARIANT,
    ) -> windows_core::Result<BOOL> {
        // SAFETY: forwards the exact IMFAttributes ABI arguments.
        unsafe { self.attributes.CompareItem(key, value) }
    }

    fn Compare(
        &self,
        theirs: Ref<IMFAttributes>,
        match_type: MF_ATTRIBUTES_MATCH_TYPE,
    ) -> windows_core::Result<BOOL> {
        let mut result = BOOL::default();
        let theirs = theirs
            .as_ref()
            .map_or(core::ptr::null_mut(), Interface::as_raw);
        let vtable = Interface::vtable(&self.attributes);
        // SAFETY: direct ABI-preserving delegation keeps the borrowed interface alive.
        unsafe {
            (vtable.Compare)(self.attributes.as_raw(), theirs, match_type, &mut result).ok()?;
        }
        Ok(result)
    }

    fn GetUINT32(&self, key: *const GUID) -> windows_core::Result<u32> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.GetUINT32(key) }
    }

    fn GetUINT64(&self, key: *const GUID) -> windows_core::Result<u64> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.GetUINT64(key) }
    }

    fn GetDouble(&self, key: *const GUID) -> windows_core::Result<f64> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.GetDouble(key) }
    }

    fn GetGUID(&self, key: *const GUID) -> windows_core::Result<GUID> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.GetGUID(key) }
    }

    fn GetStringLength(&self, key: *const GUID) -> windows_core::Result<u32> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.GetStringLength(key) }
    }

    fn GetString(
        &self,
        key: *const GUID,
        value: PWSTR,
        size: u32,
        length: *mut u32,
    ) -> windows_core::Result<()> {
        let vtable = Interface::vtable(&self.attributes);
        // SAFETY: this is a direct ABI-preserving delegation to IMFAttributes.
        unsafe { (vtable.GetString)(self.attributes.as_raw(), key, value, size, length).ok() }
    }

    fn GetAllocatedString(
        &self,
        key: *const GUID,
        value: *mut PWSTR,
        length: *mut u32,
    ) -> windows_core::Result<()> {
        // SAFETY: forwards valid COM allocation out pointers.
        unsafe { self.attributes.GetAllocatedString(key, value, length) }
    }

    fn GetBlobSize(&self, key: *const GUID) -> windows_core::Result<u32> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.GetBlobSize(key) }
    }

    fn GetBlob(
        &self,
        key: *const GUID,
        buffer: *mut u8,
        size: u32,
        blob_size: *mut u32,
    ) -> windows_core::Result<()> {
        let vtable = Interface::vtable(&self.attributes);
        // SAFETY: direct ABI-preserving delegation avoids constructing a slice from caller memory.
        unsafe { (vtable.GetBlob)(self.attributes.as_raw(), key, buffer, size, blob_size).ok() }
    }

    fn GetAllocatedBlob(
        &self,
        key: *const GUID,
        buffer: *mut *mut u8,
        size: *mut u32,
    ) -> windows_core::Result<()> {
        // SAFETY: forwards valid COM allocation out pointers.
        unsafe { self.attributes.GetAllocatedBlob(key, buffer, size) }
    }

    fn GetUnknown(
        &self,
        key: *const GUID,
        interface_id: *const GUID,
        object: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        let vtable = Interface::vtable(&self.attributes);
        // SAFETY: direct ABI-preserving delegation supports the caller-selected interface IID.
        unsafe { (vtable.GetUnknown)(self.attributes.as_raw(), key, interface_id, object).ok() }
    }

    fn SetItem(&self, key: *const GUID, value: *const PROPVARIANT) -> windows_core::Result<()> {
        // SAFETY: forwards the exact IMFAttributes ABI arguments.
        unsafe { self.attributes.SetItem(key, value) }
    }

    fn DeleteItem(&self, key: *const GUID) -> windows_core::Result<()> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.DeleteItem(key) }
    }

    fn DeleteAllItems(&self) -> windows_core::Result<()> {
        // SAFETY: delegates to the owned IMFAttributes store.
        unsafe { self.attributes.DeleteAllItems() }
    }

    fn SetUINT32(&self, key: *const GUID, value: u32) -> windows_core::Result<()> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.SetUINT32(key, value) }
    }

    fn SetUINT64(&self, key: *const GUID, value: u64) -> windows_core::Result<()> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.SetUINT64(key, value) }
    }

    fn SetDouble(&self, key: *const GUID, value: f64) -> windows_core::Result<()> {
        // SAFETY: key follows the IMFAttributes contract.
        unsafe { self.attributes.SetDouble(key, value) }
    }

    fn SetGUID(&self, key: *const GUID, value: *const GUID) -> windows_core::Result<()> {
        // SAFETY: both GUID pointers follow the IMFAttributes contract.
        unsafe { self.attributes.SetGUID(key, value) }
    }

    fn SetString(&self, key: *const GUID, value: &PCWSTR) -> windows_core::Result<()> {
        // SAFETY: PCWSTR remains valid for the delegated call.
        unsafe { self.attributes.SetString(key, *value) }
    }

    fn SetBlob(&self, key: *const GUID, buffer: *const u8, size: u32) -> windows_core::Result<()> {
        let vtable = Interface::vtable(&self.attributes);
        // SAFETY: direct ABI-preserving delegation avoids creating a slice from caller memory.
        unsafe { (vtable.SetBlob)(self.attributes.as_raw(), key, buffer, size).ok() }
    }

    fn SetUnknown(&self, key: *const GUID, unknown: Ref<IUnknown>) -> windows_core::Result<()> {
        let unknown = unknown
            .as_ref()
            .map_or(core::ptr::null_mut(), Interface::as_raw);
        let vtable = Interface::vtable(&self.attributes);
        // SAFETY: direct ABI-preserving delegation keeps the borrowed interface alive.
        unsafe { (vtable.SetUnknown)(self.attributes.as_raw(), key, unknown).ok() }
    }

    fn LockStore(&self) -> windows_core::Result<()> {
        // SAFETY: delegates to the owned IMFAttributes store.
        unsafe { self.attributes.LockStore() }
    }

    fn UnlockStore(&self) -> windows_core::Result<()> {
        // SAFETY: delegates to the owned IMFAttributes store.
        unsafe { self.attributes.UnlockStore() }
    }

    fn GetCount(&self) -> windows_core::Result<u32> {
        // SAFETY: delegates to the owned IMFAttributes store.
        unsafe { self.attributes.GetCount() }
    }

    fn GetItemByIndex(
        &self,
        index: u32,
        key: *mut GUID,
        value: *mut PROPVARIANT,
    ) -> windows_core::Result<()> {
        // SAFETY: forwards the exact IMFAttributes ABI arguments.
        unsafe { self.attributes.GetItemByIndex(index, key, Some(value)) }
    }

    fn CopyAllItems(&self, destination: Ref<IMFAttributes>) -> windows_core::Result<()> {
        let destination = destination
            .as_ref()
            .map_or(core::ptr::null_mut(), Interface::as_raw);
        let vtable = Interface::vtable(&self.attributes);
        // SAFETY: direct ABI-preserving delegation keeps the borrowed interface alive.
        unsafe { (vtable.CopyAllItems)(self.attributes.as_raw(), destination).ok() }
    }
}
