use crate::OutputMode;
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppConfig {
    pub schema_version: u32,
    pub selected_video_device_id: String,
    pub crop_webcam_to_16_9: bool,
    pub selected_monitor_label: String,
    pub reference_image_path: String,
    pub similarity_threshold: f64,
    pub cursor_visible: bool,
    pub automatic_monitor_rescans: bool,
    pub automatic_screen_capture_recovery: bool,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub start_automatically: bool,
    pub close_to_tray: bool,
    pub show_notifications: bool,
    pub interface_language: String,
    pub confirm_exit: bool,
    pub output_mode: OutputMode,
    pub placeholder_color_bgra: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            selected_video_device_id: String::new(),
            crop_webcam_to_16_9: true,
            selected_monitor_label: String::new(),
            reference_image_path: String::new(),
            similarity_threshold: 0.98,
            cursor_visible: false,
            automatic_monitor_rescans: true,
            automatic_screen_capture_recovery: true,
            start_with_windows: false,
            start_minimized: true,
            start_automatically: true,
            close_to_tray: true,
            show_notifications: true,
            interface_language: "en-US".into(),
            confirm_exit: true,
            output_mode: OutputMode::Automatic,
            placeholder_color_bgra: 0xff17_1719,
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AppConfigFile {
    schema_version: u32,
    selected_video_device_id: String,
    crop_webcam_to_16_9: bool,
    selected_monitor_label: String,
    reference_image_path: String,
    similarity_threshold: f64,
    cursor_visible: bool,
    automatic_monitor_rescans: bool,
    automatic_screen_capture_recovery: Option<bool>,
    start_with_windows: bool,
    start_minimized: bool,
    start_automatically: bool,
    close_to_tray: bool,
    show_notifications: bool,
    interface_language: String,
    confirm_exit: bool,
    output_mode: OutputMode,
    placeholder_color_bgra: u32,
}

impl Default for AppConfigFile {
    fn default() -> Self {
        let config = AppConfig::default();
        Self {
            schema_version: config.schema_version,
            selected_video_device_id: config.selected_video_device_id,
            crop_webcam_to_16_9: config.crop_webcam_to_16_9,
            selected_monitor_label: config.selected_monitor_label,
            reference_image_path: config.reference_image_path,
            similarity_threshold: config.similarity_threshold,
            cursor_visible: config.cursor_visible,
            automatic_monitor_rescans: config.automatic_monitor_rescans,
            automatic_screen_capture_recovery: None,
            start_with_windows: config.start_with_windows,
            start_minimized: config.start_minimized,
            start_automatically: config.start_automatically,
            close_to_tray: config.close_to_tray,
            show_notifications: config.show_notifications,
            interface_language: config.interface_language,
            confirm_exit: config.confirm_exit,
            output_mode: config.output_mode,
            placeholder_color_bgra: config.placeholder_color_bgra,
        }
    }
}

impl<'de> Deserialize<'de> for AppConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let file = AppConfigFile::deserialize(deserializer)?;
        Ok(Self {
            schema_version: file.schema_version,
            selected_video_device_id: file.selected_video_device_id,
            crop_webcam_to_16_9: file.crop_webcam_to_16_9,
            selected_monitor_label: file.selected_monitor_label,
            reference_image_path: file.reference_image_path,
            similarity_threshold: file.similarity_threshold,
            cursor_visible: file.cursor_visible,
            automatic_monitor_rescans: file.automatic_monitor_rescans,
            automatic_screen_capture_recovery: file
                .automatic_screen_capture_recovery
                .unwrap_or(file.automatic_monitor_rescans),
            start_with_windows: file.start_with_windows,
            start_minimized: file.start_minimized,
            start_automatically: file.start_automatically,
            close_to_tray: file.close_to_tray,
            show_notifications: file.show_notifications,
            interface_language: file.interface_language,
            confirm_exit: file.confirm_exit,
            output_mode: file.output_mode,
            placeholder_color_bgra: file.placeholder_color_bgra,
        })
    }
}

impl AppConfig {
    fn validate(mut self) -> io::Result<Self> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported schema_version",
            ));
        }
        if !self.similarity_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.similarity_threshold)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "similarity_threshold must be between zero and one",
            ));
        }
        self.placeholder_color_bgra |= 0xff00_0000;
        Ok(self)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigLoad {
    pub config: AppConfig,
    pub used_backup: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ConfigStore {
    directory: PathBuf,
}

impl ConfigStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn config_path(&self) -> PathBuf {
        self.directory.join("config.json")
    }
    pub fn backup_path(&self) -> PathBuf {
        self.directory.join("config.backup.json")
    }
    pub fn reference_path(&self) -> PathBuf {
        self.directory.join("reference.png")
    }
    pub fn logs_path(&self) -> PathBuf {
        self.directory.join("logs")
    }

    pub fn parse(json: &str) -> io::Result<AppConfig> {
        serde_json::from_str::<AppConfig>(json)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .validate()
    }

    pub fn load(&self) -> ConfigLoad {
        if !self.config_path().exists() {
            return ConfigLoad::default();
        }
        match fs::read_to_string(self.config_path()).and_then(|json| Self::parse(&json)) {
            Ok(config) => ConfigLoad {
                config,
                ..ConfigLoad::default()
            },
            Err(primary_error) => {
                let mut warnings = vec![format!("Primary configuration invalid: {primary_error}")];
                let invalid = self.directory.join("config.invalid.json");
                if let Err(error) = fs::copy(self.config_path(), invalid) {
                    warnings.push(format!("Could not preserve invalid configuration: {error}"));
                }
                match fs::read_to_string(self.backup_path()).and_then(|json| Self::parse(&json)) {
                    Ok(config) => {
                        warnings.push("Loaded last valid configuration backup".into());
                        ConfigLoad {
                            config,
                            used_backup: true,
                            warnings,
                        }
                    }
                    Err(backup_error) => {
                        warnings.push(format!(
                            "Backup configuration invalid: {backup_error}; using defaults"
                        ));
                        ConfigLoad {
                            warnings,
                            ..ConfigLoad::default()
                        }
                    }
                }
            }
        }
    }

    pub fn save(&self, config: &AppConfig) -> io::Result<()> {
        self.save_with_replace(config, atomic_replace)
    }

    pub fn save_with_replace(
        &self,
        config: &AppConfig,
        replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        config.clone().validate()?;
        fs::create_dir_all(&self.directory)?;
        let temporary = self.directory.join("config.tmp.json");
        let bytes = serde_json::to_vec_pretty(config)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        if self.config_path().exists() {
            fs::copy(self.config_path(), self.backup_path())?;
        }
        replace(&temporary, &self.config_path())
    }
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_one_round_trips() {
        let config = AppConfig {
            selected_video_device_id: "camera\\id\"one".into(),
            crop_webcam_to_16_9: false,
            selected_monitor_label: "Studio Display".into(),
            automatic_monitor_rescans: false,
            output_mode: OutputMode::ForceScreen,
            ..AppConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(ConfigStore::parse(&json).unwrap(), config);
        assert_eq!(ConfigStore::parse("{}").unwrap(), AppConfig::default());
        let invalid_schema = serde_json::json!({ "schema_version": 2 }).to_string();
        assert!(ConfigStore::parse(&invalid_schema).is_err());
    }

    #[test]
    fn legacy_schema_defaults_webcam_crop_to_enabled() {
        let config = ConfigStore::parse(r#"{"schema_version":1}"#).unwrap();
        assert!(config.crop_webcam_to_16_9);
        assert!(config.selected_monitor_label.is_empty());
        assert!(config.automatic_monitor_rescans);
        assert!(config.automatic_screen_capture_recovery);
    }

    #[test]
    fn legacy_rescan_choice_is_inherited_by_screen_capture_recovery() {
        let disabled =
            ConfigStore::parse(r#"{"schema_version":1,"automatic_monitor_rescans":false}"#)
                .unwrap();
        assert!(!disabled.automatic_monitor_rescans);
        assert!(!disabled.automatic_screen_capture_recovery);

        let enabled =
            ConfigStore::parse(r#"{"schema_version":1,"automatic_monitor_rescans":true}"#).unwrap();
        assert!(enabled.automatic_monitor_rescans);
        assert!(enabled.automatic_screen_capture_recovery);
    }

    #[test]
    fn explicit_screen_capture_recovery_choice_overrides_legacy_inheritance() {
        for (automatic_monitor_rescans, automatic_screen_capture_recovery) in
            [(false, true), (true, false)]
        {
            let config = ConfigStore::parse(
                &serde_json::json!({
                    "schema_version": 1,
                    "automatic_monitor_rescans": automatic_monitor_rescans,
                    "automatic_screen_capture_recovery": automatic_screen_capture_recovery,
                })
                .to_string(),
            )
            .unwrap();
            assert_eq!(config.automatic_monitor_rescans, automatic_monitor_rescans);
            assert_eq!(
                config.automatic_screen_capture_recovery,
                automatic_screen_capture_recovery
            );
        }
    }

    #[test]
    fn backup_is_used_and_double_corruption_falls_back() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(directory.path());
        let original = AppConfig {
            selected_video_device_id: "saved".into(),
            ..AppConfig::default()
        };
        store.save(&original).unwrap();
        store
            .save(&AppConfig {
                selected_video_device_id: "new".into(),
                ..original
            })
            .unwrap();
        fs::write(store.config_path(), "bad").unwrap();
        let loaded = store.load();
        assert!(loaded.used_backup);
        assert_eq!(loaded.config.selected_video_device_id, "saved");
        fs::write(store.backup_path(), "bad too").unwrap();
        assert_eq!(store.load().config, AppConfig::default());
    }
}
