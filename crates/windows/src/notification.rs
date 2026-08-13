use crate::dialog::{TaskDialogIcon, TaskDialogSpec, show_task_dialog};
use stageswap_i18n::{Locale, text};
use windows::Win32::UI::Controls::TDCBF_OK_BUTTON;

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
