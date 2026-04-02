//! Voice input configuration.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

fn default_hotkey() -> String {
    "Ctrl+G,v".to_string()
}

fn default_input_device() -> String {
    "system_default".to_string()
}

fn default_language() -> String {
    "auto".to_string()
}

/// Voice input configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Path to the local ASR model (e.g. Qwen3-ASR).
    pub model_path: Option<PathBuf>,
    /// Hotkey to toggle voice input.
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Audio input device name.
    #[serde(default = "default_input_device")]
    pub input_device: String,
    /// Recognition language.
    #[serde(default = "default_language")]
    pub language: String,
    /// Whether voice input is enabled.
    pub enabled: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            hotkey: default_hotkey(),
            input_device: default_input_device(),
            language: default_language(),
            enabled: false,
        }
    }
}

impl VoiceConfig {
    /// Validate the configuration.
    ///
    /// Checks that `model_path`, if set, points to an existing directory.
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.model_path.is_none() {
            return Err(ConfigError::ValidationError {
                reason: "voice model path is required when voice input is enabled".into(),
            });
        }

        if self.hotkey.trim().is_empty() {
            return Err(ConfigError::ValidationError {
                reason: "voice hotkey cannot be empty".into(),
            });
        }

        if self.input_device.trim().is_empty() {
            return Err(ConfigError::ValidationError {
                reason: "voice input device cannot be empty".into(),
            });
        }

        if let Some(ref path) = self.model_path {
            if !path.exists() {
                return Err(ConfigError::ValidationError {
                    reason: format!("voice model path does not exist: {}", path.display()),
                });
            }
            if !path.is_dir() {
                return Err(ConfigError::ValidationError {
                    reason: format!("voice model path must be a directory: {}", path.display()),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled() {
        let v = VoiceConfig::default();
        assert!(!v.enabled);
        assert_eq!(v.hotkey, "Ctrl+G,v");
        assert_eq!(v.input_device, "system_default");
        assert_eq!(v.language, "auto");
        assert!(v.model_path.is_none());
    }

    #[test]
    fn validate_ok_when_no_model_path() {
        let v = VoiceConfig::default();
        assert!(v.validate().is_ok());
    }

    #[test]
    fn validate_fails_when_model_path_missing() {
        let v = VoiceConfig {
            model_path: Some(PathBuf::from("/nonexistent/model")),
            ..Default::default()
        };
        let err = v.validate().unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn validate_ok_when_model_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("model");
        std::fs::create_dir(&model_dir).unwrap();

        let v = VoiceConfig {
            model_path: Some(model_dir),
            ..Default::default()
        };
        assert!(v.validate().is_ok());
    }

    #[test]
    fn validate_fails_when_model_path_is_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.bin");
        std::fs::write(&model, b"fake").unwrap();

        let v = VoiceConfig {
            model_path: Some(model),
            ..Default::default()
        };
        let err = v.validate().unwrap_err();
        assert!(err.to_string().contains("must be a directory"));
    }

    #[test]
    fn roundtrip_toml() {
        let v = VoiceConfig {
            model_path: Some(PathBuf::from("/tmp/model")),
            hotkey: "Ctrl+M".to_string(),
            input_device: "mic0".to_string(),
            language: "ja".to_string(),
            enabled: true,
        };
        let toml_str = toml::to_string_pretty(&v).unwrap();
        let loaded: VoiceConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.model_path, v.model_path);
        assert_eq!(loaded.hotkey, v.hotkey);
        assert_eq!(loaded.input_device, v.input_device);
        assert_eq!(loaded.language, v.language);
        assert!(loaded.enabled);
    }

    #[test]
    fn validate_fails_when_enabled_without_model_path() {
        let v = VoiceConfig {
            enabled: true,
            ..Default::default()
        };
        let err = v.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("required when voice input is enabled"));
    }
}
