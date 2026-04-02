//! Voice state management.

/// Current state of the voice subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceState {
    /// No voice activity.
    Idle,
    /// Microphone is actively recording.
    Recording,
    /// Audio is being transcribed to text.
    Transcribing,
    /// An error occurred.
    Error(String),
}

impl VoiceState {
    /// Returns `true` when the voice subsystem is idle.
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Returns `true` when recording is in progress.
    pub fn is_recording(&self) -> bool {
        matches!(self, Self::Recording)
    }

    /// Returns `true` when transcription is in progress.
    pub fn is_transcribing(&self) -> bool {
        matches!(self, Self::Transcribing)
    }

    /// Returns `true` when the state represents an error.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}
