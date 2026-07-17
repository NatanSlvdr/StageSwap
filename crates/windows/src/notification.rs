use std::thread;
use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONWARNING, MB_OK, MESSAGEBOX_STYLE, MessageBoxW,
};
use windows_core::PCWSTR;

pub fn notify_warning(message: String) {
    let _ = thread::Builder::new()
        .name("asc-notification".into())
        .spawn(move || show_dialog(&message, MB_OK | MB_ICONWARNING));
}

pub fn show_error_dialog(message: &str) {
    show_dialog(message, MB_OK | MB_ICONERROR);
}

fn show_dialog(message: &str, style: MESSAGEBOX_STYLE) {
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    let title: Vec<u16> = "Automatic Screen Camera"
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: both UTF-16 buffers are terminated and live for the duration of the call.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
        )
    };
}
