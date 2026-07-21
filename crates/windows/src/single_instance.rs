use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows_core::w;

pub struct SingleInstance(HANDLE);

impl SingleInstance {
    pub fn acquire() -> Result<Option<Self>, String> {
        // A Local mutex scopes ownership to the signed-in Windows session.
        let handle = unsafe {
            CreateMutexW(
                None,
                false,
                w!("Local\\AutomaticScreenCameraRust.Application"),
            )
        }
        .map_err(|error| format!("could not create application instance lock: {error}"))?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let _ = unsafe { CloseHandle(handle) };
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
