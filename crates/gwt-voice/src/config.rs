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
}
