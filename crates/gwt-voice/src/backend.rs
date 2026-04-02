//! Voice backend trait definition.

/// Trait for voice input backends (recording + transcription).
pub trait VoiceBackend {
    /// Start recording audio from the microphone.
    fn start_recording(&mut self) -> Result<(), VoiceError>;

    /// Stop recording and return the captured audio data.
    fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError>;

    /// Transcribe raw audio data into text.
    fn transcribe(&self, audio: &[u8]) -> Result<String, VoiceError>;

    /// Check whether this backend is available on the current system.
    fn is_available(&self) -> bool;
}

/// Errors produced by voice operations.
#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("Voice backend not available")]
    NotAvailable,

    #[error("Not currently recording")]
    NotRecording,

    #[error("Already recording")]
    AlreadyRecording,

    #[error("Transcription failed: {0}")]
    TranscriptionFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
