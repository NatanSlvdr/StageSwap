use crate::remove_virtual_camera;
use crate::virtual_camera::{LEGACY_SOURCE_ID, remove_virtual_camera_for_source};
use asc_core::{AppConfig, ConfigStore};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, FreeLibrary,
};
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE,
    REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64;
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_UNKNOWN;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, IsWow64Process2, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows_core::{HRESULT, PCSTR, PCWSTR};

const SOURCE_FILE: &str = "AutomaticScreenCameraSource.dll";
const OLD_CLASS_KEY: &str = r"Software\Classes\CLSID\{4B8BA04C-7A67-4DD5-B9F4-C607940A7A64}";
const OLD_DEPLOYMENT_KEY: &str = r"SOFTWARE\AutomaticScreenCamera\Deployment";
const OLD_PORTABLE_DIRECTORY: &str = "Automatic Screen Camera Portable";
const SOURCE_INPROC_KEY: &str =
    r"Software\Classes\CLSID\{402EB87C-123B-4765-9FF7-6E11CC7DA5B3}\InprocServer32";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "AutomaticScreenCameraRust";
const OLD_RUN_VALUE: &str = "AutomaticScreenCamera";

pub fn configure_startup(enabled: bool) -> Result<(), String> {
    let path: Vec<u16> = RUN_KEY.encode_utf16().chain([0]).collect();
    let value_name: Vec<u16> = RUN_VALUE.encode_utf16().chain([0]).collect();
    let mut key = HKEY::default();
    // SAFETY: path is terminated and key is writable.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if status.is_err() {
        return Err(format!("could not open Windows startup key: {status:?}"));
    }
    let result = if enabled {
        let executable =
            env::current_exe().map_err(|error| format!("could not locate executable: {error}"))?;
        let command = format!("\"{}\" --startup", executable.display());
        let command: Vec<u16> = command.encode_utf16().chain([0]).collect();
        // SAFETY: the key and both buffers are live for the call.
        unsafe {
            RegSetValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                REG_SZ,
                Some(core::slice::from_raw_parts(
                    command.as_ptr().cast::<u8>(),
                    command.len() * 2,
                )),
            )
        }
    } else {
        // SAFETY: the key is live and value name is terminated.
        unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) }
    };
    let _ = unsafe { RegCloseKey(key) };
    if result.is_ok() || (!enabled && result == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(format!(
            "could not update Windows startup preference: {result:?}"
        ))
    }
}

pub fn save_config_atomic(store: &ConfigStore, config: &AppConfig) -> std::io::Result<()> {
    store.save_with_replace(config, |source, destination| {
        let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
        let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: both paths are terminated and remain live for the call.
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(std::io::Error::other)
    })
}

/// Handles internal elevated deployment commands and ensures that a portable
/// release has extracted and registered its embedded, same-architecture DLL.
/// Returns true when the current process should exit without opening the UI.
pub fn portable_startup(payload: &[u8]) -> Result<bool, String> {
    let argument = env::args().nth(1);
    match argument.as_deref() {
        Some("--portable-register-elevated") => {
            validate_native_architecture()?;
            validate_embedded_source(payload)?;
            install(payload)?;
            Ok(true)
        }
        Some("--cleanup-legacy-elevated") => {
            cleanup_legacy_machine_install()?;
            Ok(true)
        }
        Some("--cleanup-portable-elevated") => {
            cleanup()?;
            Ok(true)
        }
        Some("--cleanup-portable") => {
            run_elevated("--cleanup-portable-elevated")?;
            Ok(true)
        }
        Some("--startup") => {
            ensure_portable_install(payload)?;
            Ok(false)
        }
        Some(_) => Ok(false),
        None if payload.is_empty() => Ok(false),
        None => {
            ensure_portable_install(payload)?;
            Ok(false)
        }
    }
}

fn ensure_portable_install(payload: &[u8]) -> Result<(), String> {
    if payload.is_empty() {
        return Ok(());
    }
    validate_native_architecture()?;
    validate_embedded_source(payload)?;
    cleanup_legacy_install()?;
    if fs::read(source_path()).ok().as_deref() != Some(payload) || !source_registration_matches() {
        run_elevated("--portable-register-elevated")?;
    }
    if fs::read(source_path()).ok().as_deref() != Some(payload) {
        return Err("portable camera source was not installed correctly".into());
    }
    if !source_registration_matches() {
        return Err("portable camera source COM registration was not installed correctly".into());
    }
    Ok(())
}

fn source_registration_matches() -> bool {
    let Some(registered) = registry_string(SOURCE_INPROC_KEY, None) else {
        return false;
    };
    let Some(threading_model) = registry_string(SOURCE_INPROC_KEY, Some("ThreadingModel")) else {
        return false;
    };
    PathBuf::from(registered)
        .to_string_lossy()
        .eq_ignore_ascii_case(&source_path().to_string_lossy())
        && threading_model
            .to_string_lossy()
            .eq_ignore_ascii_case("Both")
}

fn registry_string(key: &str, value_name: Option<&str>) -> Option<std::ffi::OsString> {
    let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let value_name = value_name.map(|value| value.encode_utf16().chain([0]).collect::<Vec<_>>());
    let value_name = value_name
        .as_ref()
        .map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr()));
    let mut bytes = 0;
    // SAFETY: the key is terminated and the first call only requests the value size.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            value_name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if status.is_err() || bytes < 2 {
        return None;
    }
    let mut value = vec![0_u16; (bytes as usize).div_ceil(size_of::<u16>())];
    // SAFETY: value has the byte capacity reported by RegGetValueW.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            value_name,
            RRF_RT_REG_SZ,
            None,
            Some(value.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.is_err() {
        return None;
    }
    let length = value
        .iter()
        .position(|word| *word == 0)
        .unwrap_or(value.len());
    Some(std::ffi::OsString::from_wide(&value[..length]))
}

fn validate_embedded_source(payload: &[u8]) -> Result<(), String> {
    if payload.get(..2) != Some(b"MZ") {
        return Err("embedded camera-source payload is not a PE file".into());
    }
    let pe_offset = payload
        .get(0x3c..0x40)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| "embedded camera-source DOS header is truncated".to_string())?
        as usize;
    if payload.get(pe_offset..pe_offset + 4) != Some(b"PE\0\0") {
        return Err("embedded camera-source PE signature is missing".into());
    }
    let machine = payload
        .get(pe_offset + 4..pe_offset + 6)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| "embedded camera-source COFF header is truncated".to_string())?;
    let expected = IMAGE_FILE_MACHINE_AMD64.0;
    if machine != expected {
        return Err(format!(
            "embedded camera-source architecture 0x{machine:04x} does not match this executable"
        ));
    }
    Ok(())
}

fn validate_native_architecture() -> Result<(), String> {
    let mut process_machine = IMAGE_FILE_MACHINE_UNKNOWN;
    let mut native_machine = IMAGE_FILE_MACHINE_UNKNOWN;
    // SAFETY: both machine outputs are writable and the pseudo-handle is valid.
    unsafe {
        IsWow64Process2(
            GetCurrentProcess(),
            &mut process_machine,
            Some(&mut native_machine),
        )
    }
    .map_err(|error| format!("could not validate native package architecture: {error}"))?;
    let package_machine = IMAGE_FILE_MACHINE_AMD64;
    if native_machine != package_machine || process_machine != IMAGE_FILE_MACHINE_UNKNOWN {
        return Err(
            "this portable executable must run on matching native Windows architecture".into(),
        );
    }
    Ok(())
}

fn previous_install_present() -> bool {
    registry_key_present(HKEY_LOCAL_MACHINE, OLD_CLASS_KEY)
        || registry_key_present(HKEY_LOCAL_MACHINE, OLD_DEPLOYMENT_KEY)
        || old_source_path().is_file()
}

fn registry_key_present(root: HKEY, key: &str) -> bool {
    let path: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let mut opened_key = HKEY::default();
    // SAFETY: path is terminated and key is writable.
    let opened = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(path.as_ptr()),
            Some(0),
            KEY_READ,
            &mut opened_key,
        )
    }
    .is_ok();
    if opened {
        let _ = unsafe { RegCloseKey(opened_key) };
    }
    opened
}

fn cleanup_legacy_install() -> Result<(), String> {
    remove_startup_value(OLD_RUN_VALUE)?;
    // Opening and removing this source identity is idempotent and also clears a
    // virtual-camera registration left behind after its COM key was removed.
    remove_virtual_camera_for_source(LEGACY_SOURCE_ID)
        .map_err(|error| format!("could not remove legacy virtual camera: {error}"))?;
    if !previous_install_present() {
        return Ok(());
    }
    run_elevated("--cleanup-legacy-elevated")?;
    if previous_install_present() {
        return Err("legacy Automatic Screen Camera installation was not removed correctly".into());
    }
    Ok(())
}

fn cleanup_legacy_machine_install() -> Result<(), String> {
    let source = old_source_path();
    if source.is_file() {
        // The direct registry removal below is the fallback for damaged DLLs.
        let _ = invoke_registration(&source, b"DllUnregisterServer\0");
    }
    delete_registry_tree(HKEY_LOCAL_MACHINE, OLD_CLASS_KEY)?;
    delete_registry_tree(HKEY_LOCAL_MACHINE, OLD_DEPLOYMENT_KEY)?;
    if let Some(directory) = source.parent()
        && directory.exists()
    {
        fs::remove_dir_all(directory)
            .map_err(|error| format!("could not remove {}: {error}", directory.display()))?;
    }
    Ok(())
}

fn delete_registry_tree(root: HKEY, key: &str) -> Result<(), String> {
    let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
    // SAFETY: the registry path is terminated for the duration of the call.
    let status = unsafe { RegDeleteTreeW(root, PCWSTR(key.as_ptr())) };
    if status.is_ok() || status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("could not remove legacy registry key: {status:?}"))
    }
}

fn remove_startup_value(value: &str) -> Result<(), String> {
    let path: Vec<u16> = RUN_KEY.encode_utf16().chain([0]).collect();
    let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let mut key = HKEY::default();
    // SAFETY: the path is terminated and key is writable.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            Some(0),
            KEY_WRITE,
            &mut key,
        )
    };
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return Ok(());
    }
    if status.is_err() {
        return Err(format!("could not open Windows startup key: {status:?}"));
    }
    // SAFETY: the key and terminated value name remain live for the call.
    let result = unsafe { RegDeleteValueW(key, PCWSTR(value.as_ptr())) };
    let _ = unsafe { RegCloseKey(key) };
    if result.is_ok() || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!(
            "could not remove legacy Windows startup entry: {result:?}"
        ))
    }
}

fn portable_directory() -> Result<PathBuf, String> {
    env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .ok_or_else(|| "ProgramFiles is not defined".into())
        .map(|path| path.join("Automatic Screen Camera Rust Portable"))
}

fn source_path() -> PathBuf {
    portable_directory()
        .unwrap_or_else(|_| {
            PathBuf::from(r"C:\Program Files\Automatic Screen Camera Rust Portable")
        })
        .join(SOURCE_FILE)
}

fn old_source_path() -> PathBuf {
    env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
        .join(OLD_PORTABLE_DIRECTORY)
        .join(SOURCE_FILE)
}

fn install(payload: &[u8]) -> Result<(), String> {
    let directory = portable_directory()?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let source = directory.join(SOURCE_FILE);
    let staging = directory.join(format!("{SOURCE_FILE}.installing"));
    let backup = directory.join(format!("{SOURCE_FILE}.backup"));
    let _ = fs::remove_file(&staging);
    if !source.exists() && backup.exists() {
        fs::rename(&backup, &source)
            .map_err(|error| format!("could not recover {}: {error}", source.display()))?;
    } else {
        let _ = fs::remove_file(&backup);
    }
    fs::write(&staging, payload)
        .map_err(|error| format!("could not stage {}: {error}", staging.display()))?;
    if source.exists() {
        if let Err(error) = invoke_registration(&source, b"DllUnregisterServer\0") {
            let _ = fs::remove_file(&staging);
            return Err(error);
        }
        if let Err(error) = fs::rename(&source, &backup) {
            let _ = invoke_registration(&source, b"DllRegisterServer\0");
            let _ = fs::remove_file(&staging);
            return Err(format!("could not back up {}: {error}", source.display()));
        }
    }
    if let Err(error) = fs::rename(&staging, &source) {
        if backup.exists() {
            let _ = fs::rename(&backup, &source);
            let _ = invoke_registration(&source, b"DllRegisterServer\0");
        }
        return Err(format!("could not install {}: {error}", source.display()));
    }
    if let Err(error) = invoke_registration(&source, b"DllRegisterServer\0") {
        let _ = fs::remove_file(&source);
        if backup.exists() {
            let _ = fs::rename(&backup, &source);
            let _ = invoke_registration(&source, b"DllRegisterServer\0");
        }
        return Err(error);
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

fn cleanup() -> Result<(), String> {
    configure_startup(false)?;
    let camera_result = remove_virtual_camera();
    let source = source_path();
    if source.is_file() {
        invoke_registration(&source, b"DllUnregisterServer\0")?;
        fs::remove_file(&source)
            .map_err(|error| format!("could not remove {}: {error}", source.display()))?;
    }
    if let Some(parent) = source.parent() {
        let _ = fs::remove_dir(parent);
    }
    camera_result
}

fn invoke_registration(path: &Path, export: &'static [u8]) -> Result<(), String> {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: the path and export name are terminated and remain live.
    let module = unsafe { LoadLibraryW(PCWSTR(path.as_ptr())) }
        .map_err(|error| format!("could not load camera source: {error}"))?;
    // SAFETY: module is live and export is a terminated static string.
    let address = unsafe { GetProcAddress(module, PCSTR(export.as_ptr())) };
    let Some(address) = address else {
        let _ = unsafe { FreeLibrary(module) };
        return Err("camera source registration export is missing".into());
    };
    type Registration = unsafe extern "system" fn() -> HRESULT;
    // SAFETY: the named DLL exports use this standard COM registration signature.
    let registration: Registration = unsafe { core::mem::transmute(address) };
    let result = unsafe { registration() };
    let _ = unsafe { FreeLibrary(module) };
    result
        .ok()
        .map_err(|error| format!("camera source registration failed: {error}"))
}

fn run_elevated(argument: &str) -> Result<(), String> {
    let executable =
        env::current_exe().map_err(|error| format!("could not locate executable: {error}"))?;
    let executable: Vec<u16> = executable.as_os_str().encode_wide().chain([0]).collect();
    let argument: Vec<u16> = argument.encode_utf16().chain([0]).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain([0]).collect();
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(executable.as_ptr()),
        lpParameters: PCWSTR(argument.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..SHELLEXECUTEINFOW::default()
    };
    // SAFETY: all referenced buffers remain live until ShellExecuteExW returns.
    unsafe { ShellExecuteExW(&mut execute) }.map_err(|error| {
        format!("portable deployment elevation was cancelled or failed: {error}")
    })?;
    if execute.hProcess.is_invalid() {
        return Err("elevated deployment process was not created".into());
    }
    // SAFETY: ShellExecuteExW returned an owned process handle.
    unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    let mut code = 1;
    let status = unsafe { GetExitCodeProcess(execute.hProcess, &mut code) };
    let _ = unsafe { CloseHandle(execute.hProcess) };
    status.map_err(|error| format!("could not read deployment result: {error}"))?;
    if code != 0 {
        return Err(format!("elevated deployment failed with exit code {code}"));
    }
    Ok(())
}

use std::os::windows::ffi::{OsStrExt, OsStringExt};
