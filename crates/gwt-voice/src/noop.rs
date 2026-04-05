//! No-op voice backend for environments without audio support.

use crate::backend::{VoiceBackend, VoiceError};

/// A voice backend that does nothing — all operations return errors or empty values.
pub struct NoOpVoiceBackend;

impl NoOpVoiceBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoOpVoiceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceBackend for NoOpVoiceBackend {
    fn start_recording(&mut self) -> Result<(), VoiceError> {
        Err(VoiceError::NotAvailable)
    }

    fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
        Err(VoiceError::NotAvailable)
    }

    fn transcribe(&self, _audio: &[u8]) -> Result<String, VoiceError> {
        Err(VoiceError::NotAvailable)
    }

    fn is_available(&self) -> bool {
        false
    }
}
