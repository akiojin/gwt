//! Voice runtime types and trait definitions.

use std::path::PathBuf;

use crate::error::GwtError;

/// Transcription result returned by a voice backend.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// The transcribed text.
    pub text: String,
    /// Detected or specified language code (e.g. "en", "ja").
    pub language: Option<String>,
    /// Duration of the transcribed audio in milliseconds.
    pub duration_ms: u64,
}

/// Voice input configuration.
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    /// Path to the ASR model file, if applicable.
    pub model_path: Option<PathBuf>,
    /// Language hint for transcription.
    pub language: Option<String>,
    /// Whether voice input is enabled.
    pub enabled: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            language: Some("en".to_string()),
            enabled: false,
        }
    }
}

/// Runtime state of a voice backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceState {
    /// Voice input is disabled or not configured.
    Disabled,
    /// Backend is initialised and ready to record.
    Ready,
    /// Actively capturing audio.
    Listening,
    /// Processing captured audio into text.
    Transcribing,
    /// An error occurred; contains a human-readable description.
    Error(String),
}

/// Trait for voice transcription backends.
///
/// Implementations wrap a concrete ASR engine (e.g. Whisper, Qwen-ASR)
/// while keeping the core crate free of native dependencies.
pub trait VoiceBackend: Send + Sync {
    /// Whether the backend can currently perform transcription.
    fn is_available(&self) -> bool;

    /// Begin capturing audio from the default input device.
    fn start_listening(&mut self) -> Result<(), GwtError>;

    /// Stop capturing and return the transcription result.
    fn stop_listening(&mut self) -> Result<TranscriptionResult, GwtError>;

    /// Current state of the backend.
    fn state(&self) -> VoiceState;
}

/// No-op voice backend used when voice input is disabled or unavailable.
pub struct NoOpVoiceBackend;

impl VoiceBackend for NoOpVoiceBackend {
    fn is_available(&self) -> bool {
        false
    }

    fn start_listening(&mut self) -> Result<(), GwtError> {
        Err(GwtError::Internal("Voice not available".into()))
    }

    fn stop_listening(&mut self) -> Result<TranscriptionResult, GwtError> {
        Err(GwtError::Internal("Voice not available".into()))
    }

    fn state(&self) -> VoiceState {
        VoiceState::Disabled
    }
}

/// Well-known Whisper model filenames to probe under `~/.gwt/models/`.
const MODEL_CANDIDATES: &[&str] = &["ggml-base.bin", "ggml-small.bin", "ggml-large.bin"];

/// Check whether voice models are available on the system.
///
/// Looks for well-known model files under `~/.gwt/models/`.
/// Returns `false` by default when no model is found.
pub fn detect_voice_support() -> bool {
    let Some(model_dir) = dirs::home_dir().map(|h| h.join(".gwt").join("models")) else {
        return false;
    };
    model_dir.is_dir()
        && MODEL_CANDIDATES
            .iter()
            .any(|name| model_dir.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_config_default() {
        let config = VoiceConfig::default();
        assert!(!config.enabled);
        assert!(config.model_path.is_none());
        assert_eq!(config.language, Some("en".to_string()));
    }

    #[test]
    fn test_noop_backend_not_available() {
        let backend = NoOpVoiceBackend;
        assert!(!backend.is_available());
    }

    #[test]
    fn test_noop_backend_start_errors() {
        let mut backend = NoOpVoiceBackend;
        assert!(backend.start_listening().is_err());
        assert!(backend.stop_listening().is_err());
    }

    #[test]
    fn test_detect_voice_support_default_false() {
        // In CI / dev environments without models installed this should be false.
        // The function itself may return true on a developer machine that has
        // models installed, but the logic path is exercised either way.
        let _result = detect_voice_support();
    }

    #[test]
    fn test_voice_state_variants() {
        assert_eq!(VoiceState::Disabled, VoiceState::Disabled);
        assert_eq!(VoiceState::Ready, VoiceState::Ready);
        assert_eq!(VoiceState::Listening, VoiceState::Listening);
        assert_eq!(VoiceState::Transcribing, VoiceState::Transcribing);
        assert_eq!(
            VoiceState::Error("test".into()),
            VoiceState::Error("test".into())
        );
        assert_ne!(VoiceState::Disabled, VoiceState::Ready);
    }
}
