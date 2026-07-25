use crate::dialog::{TaskDialogIcon, TaskDialogSpec, show_task_dialog};
use notify_rust::Notification;
use windows::Win32::UI::Controls::TDCBF_OK_BUTTON;

pub fn notify_warning(message: &str) -> Result<(), String> {
    Notification::new()
        .appname("StageSwap")
        .summary("StageSwap needs attention")
        .body(message)
        .show()
        .map(|_| ())
        .map_err(|error| format!("could not show Windows notification: {error}"))
}

pub fn show_error_dialog(instruction: &str, details: &str) {
    let _ = show_task_dialog(TaskDialogSpec {
        instruction,
        content: details,
        icon: TaskDialogIcon::Error,
        choices: &[],
        default_button: 1,
        common_buttons: TDCBF_OK_BUTTON,
        command_links: false,
    });
}
