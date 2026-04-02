//! Voice input configuration.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

fn default_hotkey() -> String {
    "Ctrl+G,v".to_string()
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
    /// Audio input device name (None = system default).
    pub input_device: Option<String>,
    /// Recognition language (None = auto-detect).
    pub language: Option<String>,
    /// Whether voice input is enabled.
    pub enabled: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            hotkey: default_hotkey(),
            input_device: None,
            language: None,
            enabled: false,
        }
    }
}

impl VoiceConfig {
    /// Validate the configuration.
    ///
    /// Checks that `model_path`, if set, points to an existing path.
    pub fn validate(&self) -> Result<()> {
        if let Some(ref path) = self.model_path {
            if !path.exists() {
                return Err(ConfigError::ValidationError {
                    reason: format!("voice model path does not exist: {}", path.display()),
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
        let model = dir.path().join("model.bin");
        std::fs::write(&model, b"fake").unwrap();

        let v = VoiceConfig {
            model_path: Some(model),
            ..Default::default()
        };
        assert!(v.validate().is_ok());
    }

    #[test]
    fn roundtrip_toml() {
        let v = VoiceConfig {
            model_path: Some(PathBuf::from("/tmp/model")),
            hotkey: "Ctrl+M".to_string(),
            input_device: Some("mic0".to_string()),
            language: Some("ja".to_string()),
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
}
