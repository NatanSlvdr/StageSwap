use stageswap_core::ConfigStore;
use stageswap_i18n::Locale;
use std::{env, fs, path::PathBuf};
use windows::Win32::Globalization::GetUserDefaultLocaleName;

pub fn user_interface_locale() -> Locale {
    let mut buffer = [0_u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if length <= 1 {
        return Locale::English;
    }
    let tag = String::from_utf16_lossy(&buffer[..length as usize - 1]);
    Locale::from_tag(&tag).unwrap_or_default()
}

pub fn preferred_interface_locale() -> Locale {
    let system = user_interface_locale();
    let Some(directory) = env::var_os("LOCALAPPDATA") else {
        return system;
    };
    let path = PathBuf::from(directory)
        .join("StageSwap")
        .join("config.json");
    let Ok(json) = fs::read_to_string(path) else {
        return system;
    };
    ConfigStore::parse(&json)
        .ok()
        .and_then(|config| Locale::from_tag(&config.interface_language))
        .unwrap_or(system)
}
