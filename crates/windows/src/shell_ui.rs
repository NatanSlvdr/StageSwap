use stageswap_i18n::{Locale, text};
use std::path::{Path, PathBuf};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows_core::{PCWSTR, PWSTR};

pub fn pick_reference_image(locale: Locale) -> Option<PathBuf> {
    let title = text(locale, "Import reference image");
    let image_files = text(locale, "Image files");
    let png_files = text(locale, "PNG files");
    let jpeg_files = text(locale, "JPEG files");
    let bmp_files = text(locale, "BMP files");
    let all_files = text(locale, "All files");
    let filter = format!(
        "{image_files}\0*.png;*.jpg;*.jpeg;*.bmp\0{png_files}\0*.png\0{jpeg_files}\0*.jpg;*.jpeg\0{bmp_files}\0*.bmp\0{all_files}\0*.*\0\0"
    );
    file_dialog(false, title.as_ref(), &filter, "png")
}

pub fn pick_log_export_path(locale: Locale) -> Option<PathBuf> {
    let title = text(locale, "Export diagnostic logs");
    let json_lines = text(locale, "JSON Lines files");
    let all_files = text(locale, "All files");
    let filter = format!("{json_lines}\0*.jsonl\0{all_files}\0*.*\0\0");
    file_dialog(true, title.as_ref(), &filter, "jsonl")
}

fn file_dialog(save: bool, title: &str, filter: &str, extension: &str) -> Option<PathBuf> {
    let mut file = vec![0u16; 32_768];
    let title = wide(title);
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let extension = wide(extension);
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_PATHMUSTEXIST
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            },
        ..OPENFILENAMEW::default()
    };
    // SAFETY: the dialog structure references terminated buffers that remain live for the call.
    let accepted = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    }
    .as_bool();
    if !accepted {
        return None;
    }
    let length = file.iter().position(|value| *value == 0)?;
    Some(PathBuf::from(String::from_utf16_lossy(&file[..length])))
}

pub fn open_directory(path: &Path) -> Result<(), String> {
    open_shell_target(&path.display().to_string(), "log directory")
}

pub fn open_camera_privacy_settings() -> Result<(), String> {
    open_shell_target(
        "ms-settings:privacy-webcam",
        "Windows camera privacy settings",
    )
}

fn open_shell_target(target: &str, description: &str) -> Result<(), String> {
    let operation = wide("open");
    let target = wide(target);
    // SAFETY: both UTF-16 strings are terminated and remain live for the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as usize <= 32 {
        Err(format!(
            "could not open {description} (ShellExecute code {})",
            result.0 as usize
        ))
    } else {
        Ok(())
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
