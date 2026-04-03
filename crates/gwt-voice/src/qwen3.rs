//! Qwen3 ASR voice backend stub.
//!
//! This module provides a stub implementation of the `VoiceBackend` trait for the
//! Qwen3 ASR model. The actual model integration requires the Qwen3 model binary;
//! this stub returns appropriate errors when operations are attempted.

use crate::backend::{VoiceBackend, VoiceError};

/// Stub backend for Qwen3 ASR (Automatic Speech Recognition).
///
/// All operations return errors because the actual Qwen3 model binary is not
/// bundled. This struct exists to define the integration surface for future
/// model loading.
pub struct Qwen3AsrRecorder;

impl Qwen3AsrRecorder {
    /// Create a new Qwen3 ASR recorder stub.
    pub fn new() -> Self {
        Self
    }
}

impl Default for Qwen3AsrRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceBackend for Qwen3AsrRecorder {
    fn start_recording(&mut self) -> Result<(), VoiceError> {
        Err(VoiceError::ModelNotLoaded(
            "Qwen3 ASR model not loaded".to_string(),
        ))
    }

    fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
        Err(VoiceError::NotRecording)
    }

    fn transcribe(&self, _audio: &[u8]) -> Result<String, VoiceError> {
        Err(VoiceError::ModelNotLoaded(
            "Qwen3 ASR model not loaded".to_string(),
        ))
    }

    fn is_available(&self) -> bool {
        false
    }
}
