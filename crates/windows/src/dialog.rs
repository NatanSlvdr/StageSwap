use std::mem::size_of;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOG_COMMON_BUTTON_FLAGS, TASKDIALOG_FLAGS, TASKDIALOGCONFIG,
    TASKDIALOGCONFIG_0, TD_ERROR_ICON, TD_INFORMATION_ICON, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_SIZE_TO_CONTENT, TDF_USE_COMMAND_LINKS, TaskDialogIndirect,
};
use windows_core::PCWSTR;

pub(crate) const CANCEL_BUTTON_ID: i32 = 2;

#[derive(Clone, Copy)]
pub(crate) enum TaskDialogIcon {
    Information,
    Error,
}

impl TaskDialogIcon {
    const fn value(self) -> PCWSTR {
        match self {
            Self::Information => TD_INFORMATION_ICON,
            Self::Error => TD_ERROR_ICON,
        }
    }
}

pub(crate) struct TaskDialogSpec<'a> {
    pub instruction: &'a str,
    pub content: &'a str,
    pub icon: TaskDialogIcon,
    pub choices: &'a [(i32, &'a str)],
    pub default_button: i32,
    pub common_buttons: TASKDIALOG_COMMON_BUTTON_FLAGS,
    pub command_links: bool,
}

pub(crate) fn show_task_dialog(spec: TaskDialogSpec<'_>) -> Result<i32, String> {
    let title = wide("StageSwap");
    let instruction = wide(spec.instruction);
    let content = wide(spec.content);
    let labels: Vec<Vec<u16>> = spec.choices.iter().map(|(_, label)| wide(label)).collect();
    let buttons: Vec<TASKDIALOG_BUTTON> = spec
        .choices
        .iter()
        .zip(&labels)
        .map(|((id, _), label)| TASKDIALOG_BUTTON {
            nButtonID: *id,
            pszButtonText: PCWSTR(label.as_ptr()),
        })
        .collect();
    let mut flags: TASKDIALOG_FLAGS = TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT;
    if spec.command_links {
        flags |= TDF_USE_COMMAND_LINKS;
    }
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: HWND::default(),
        dwFlags: flags,
        dwCommonButtons: spec.common_buttons,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: spec.icon.value(),
        },
        pszMainInstruction: PCWSTR(instruction.as_ptr()),
        pszContent: PCWSTR(content.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: spec.default_button,
        ..TASKDIALOGCONFIG::default()
    };
    let mut selected = CANCEL_BUTTON_ID;
    unsafe { TaskDialogIndirect(&config, Some(&mut selected), None, None) }
        .map_err(|error| format!("could not show StageSwap dialog: {error}"))?;
    Ok(selected)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
