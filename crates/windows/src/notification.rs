use crate::dialog::{TaskDialogIcon, TaskDialogSpec, show_task_dialog};
use notify_rust::Notification;
use stageswap_i18n::{Locale, format_text, text};
use windows::Win32::UI::Controls::TDCBF_OK_BUTTON;

pub fn notify_warning(locale: Locale, message: &str) -> Result<(), String> {
    Notification::new()
        .appname("StageSwap")
        .summary(text(locale, "StageSwap needs attention").as_ref())
        .body(message)
        .show()
        .map(|_| ())
        .map_err(|error| format!("could not show Windows notification: {error}"))
}

pub fn notify_update_available(locale: Locale, version: &str) -> Result<(), String> {
    let body = format_text(
        locale,
        "StageSwap {0} is ready. Open Settings → Updates to install it.",
        &[version],
    );
    Notification::new()
        .appname("StageSwap")
        .summary(text(locale, "StageSwap update available").as_ref())
        .body(&body)
        .show()
        .map(|_| ())
        .map_err(|error| format!("could not show Windows notification: {error}"))
}

pub fn show_error_dialog(locale: Locale, instruction: &str, details: &str) {
    let instruction = text(locale, instruction);
    let _ = show_task_dialog(TaskDialogSpec {
        instruction: instruction.as_ref(),
        content: details,
        icon: TaskDialogIcon::Error,
        choices: &[],
        default_button: 1,
        common_buttons: TDCBF_OK_BUTTON,
        command_links: false,
    });
}
