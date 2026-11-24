use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::error::{io_error, Error, Result};

const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Canonical hotkey string. `fn` is the macOS-only modifier binding.
    pub hotkey: String,
    /// Input device name as reported by cpal. `None` means system default.
    pub microphone: Option<String>,
    /// `auto` or an ISO-639-1 Whisper language code.
    pub language: String,
    /// Catalog id of the selected Whisper model, or `custom`.
    pub model_id: String,
    /// Path used when `model_id` is `custom`. Empty otherwise.
    pub custom_model_path: String,
    pub mute_system_audio: bool,
    pub launch_at_login: bool,
    pub onboarding_complete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: if cfg!(target_os = "macos") {
                "fn".to_string()
            } else {
                "Ctrl+Space".to_string()
            },
            microphone: None,
            language: "auto".to_string(),
            model_id: crate::model::DEFAULT_MODEL_ID.to_string(),
            custom_model_path: String::new(),
            mute_system_audio: false,
            launch_at_login: false,
            onboarding_complete: false,
        }
    }
}

impl Settings {
    /// Trims user-provided strings and rejects values that cannot be honored.
    pub fn normalized(mut self) -> Result<Self> {
        self.hotkey = self.hotkey.trim().to_string();
        self.language = self.language.trim().to_string();
        self.model_id = self.model_id.trim().to_string();
        if self.model_id.is_empty() {
            self.model_id = crate::model::DEFAULT_MODEL_ID.to_string();
        }
        self.custom_model_path = self
            .custom_model_path
            .trim()
            .trim_matches('"')
            .trim()
            .to_string();
        // A custom selection without a path cannot work; fall back rather than
        // leaving the app with no usable model.
        if self.model_id == crate::model::CUSTOM_MODEL_ID && self.custom_model_path.is_empty() {
            self.model_id = crate::model::DEFAULT_MODEL_ID.to_string();
        }
        if self.model_id != crate::model::CUSTOM_MODEL_ID
            && !crate::model::MODELS.iter().any(|spec| spec.id == self.model_id)
        {
            self.model_id = crate::model::DEFAULT_MODEL_ID.to_string();
        }
        self.microphone = self
            .microphone
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());

        if self.hotkey.is_empty() || self.hotkey.chars().count() > 64 {
            return Err(Error::Invalid(
                "The activation hotkey must be between 1 and 64 characters.".into(),
            ));
        }

        if self.language != "auto"
            && (self.language.len() < 2
                || self.language.len() > 5
                || !self
                    .language
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'))
        {
            return Err(Error::Invalid(format!(
                "`{}` is not a recognized language code.",
                self.language
            )));
        }

        if self
            .microphone
            .as_ref()
            .is_some_and(|name| name.chars().count() > 512)
        {
            return Err(Error::Invalid("The microphone name is too long.".into()));
        }

        if self.custom_model_path.chars().count() > 4096 {
            return Err(Error::Invalid("The model path is too long.".into()));
        }

        Ok(self)
    }
}

pub struct SettingsStore {
    path: PathBuf,
    current: RwLock<Settings>,
}

impl SettingsStore {
    /// Loads settings, materializing defaults on first run. A corrupt file is
    /// moved aside rather than blocking startup.
    pub fn load(config_dir: &Path) -> Result<Self> {
        fs::create_dir_all(config_dir).map_err(|err| io_error(config_dir, err))?;
        let path = config_dir.join(SETTINGS_FILE);

        let settings = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                Ok(parsed) => match parsed.normalized() {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        warn!(%err, "settings failed validation, restoring defaults");
                        backup(&path);
                        Settings::default()
                    }
                },
                Err(err) => {
                    warn!(%err, "settings are not valid JSON, restoring defaults");
                    backup(&path);
                    Settings::default()
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(err) => return Err(io_error(&path, err)),
        };

        write_atomic(&path, &settings)?;
        info!(path = %path.display(), "settings loaded");

        Ok(Self {
            path,
            current: RwLock::new(settings),
        })
    }

    pub fn get(&self) -> Settings {
        self.current.read().clone()
    }

    pub fn save(&self, settings: Settings) -> Result<Settings> {
        let settings = settings.normalized()?;
        write_atomic(&self.path, &settings)?;
        *self.current.write() = settings.clone();
        Ok(settings)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn backup(path: &Path) {
    let backup_path = path.with_extension("json.bak");
    if let Err(err) = fs::rename(path, &backup_path) {
        warn!(%err, "could not back up unreadable settings");
    }
}

fn write_atomic(path: &Path, settings: &Settings) -> Result<()> {
    let temporary = path.with_file_name(format!(".{SETTINGS_FILE}.tmp"));
    let json = serde_json::to_vec_pretty(settings)?;
    fs::write(&temporary, json).map_err(|err| io_error(&temporary, err))?;

    // Windows rename does not replace an existing file.
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|err| io_error(path, err))?;
    }

    fs::rename(&temporary, path).map_err(|err| io_error(path, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let settings = Settings::default().normalized().expect("valid defaults");
        assert_eq!(settings.language, "auto");
        assert_eq!(settings.model_id, crate::model::DEFAULT_MODEL_ID);
        assert!(settings.custom_model_path.is_empty());
    }

    #[test]
    fn custom_model_requires_a_path() {
        let settings = Settings {
            model_id: crate::model::CUSTOM_MODEL_ID.into(),
            custom_model_path: "   ".into(),
            ..Settings::default()
        }
        .normalized()
        .expect("valid");

        assert_eq!(settings.model_id, crate::model::DEFAULT_MODEL_ID);
    }

    #[test]
    fn strips_quotes_from_the_custom_model_path() {
        let settings = Settings {
            model_id: crate::model::CUSTOM_MODEL_ID.into(),
            custom_model_path: "  \"C:\\models\\voice.bin\"  ".into(),
            ..Settings::default()
        }
        .normalized()
        .expect("valid");

        assert_eq!(settings.custom_model_path, "C:\\models\\voice.bin");
        assert_eq!(settings.model_id, crate::model::CUSTOM_MODEL_ID);
    }

    #[test]
    fn rejects_empty_hotkey() {
        let settings = Settings {
            hotkey: "   ".into(),
            ..Settings::default()
        };
        assert!(settings.normalized().is_err());
    }

    #[test]
    fn blanks_out_whitespace_microphone() {
        let settings = Settings {
            microphone: Some("  ".into()),
            ..Settings::default()
        };
        assert_eq!(settings.normalized().expect("valid").microphone, None);
    }
}
