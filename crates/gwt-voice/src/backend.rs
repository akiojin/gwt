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

    /// Maximum recording duration in seconds before auto-stop.
    /// Returns `None` if there is no limit.
    fn max_recording_seconds(&self) -> Option<u32> {
        Some(30)
    }

    /// Silence duration in seconds before auto-stop.
    /// Returns `None` if silence detection is disabled.
    fn silence_timeout_seconds(&self) -> Option<u32> {
        Some(3)
    }
}

impl<T> VoiceBackend for Box<T>
where
    T: VoiceBackend + ?Sized,
{
    fn start_recording(&mut self) -> Result<(), VoiceError> {
        (**self).start_recording()
    }

    fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
        (**self).stop_recording()
    }

    fn transcribe(&self, audio: &[u8]) -> Result<String, VoiceError> {
        (**self).transcribe(audio)
    }

    fn is_available(&self) -> bool {
        (**self).is_available()
    }

    fn max_recording_seconds(&self) -> Option<u32> {
        (**self).max_recording_seconds()
    }

    fn silence_timeout_seconds(&self) -> Option<u32> {
        (**self).silence_timeout_seconds()
    }
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

    #[error("Recording timed out after {0} seconds")]
    RecordingTimeout(u32),

    #[error("Silence detected for {0} seconds, recording stopped")]
    SilenceDetected(u32),

    #[error("Transcription failed: {0}")]
    TranscriptionFailed(String),

    #[error("Model not loaded: {0}")]
    ModelNotLoaded(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
