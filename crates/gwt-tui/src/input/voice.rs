//! Voice input state tracking and rendering for the TUI.

/// Voice input status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceStatus {
    /// No voice activity.
    #[default]
    Idle,
    /// Actively recording audio.
    Recording,
    /// Audio captured, transcription in progress.
    Transcribing,
    /// An error occurred.
    Error,
}

/// Voice input state.
#[derive(Debug, Clone, Default)]
pub struct VoiceInputState {
    /// Current status.
    pub status: VoiceStatus,
    /// Duration of the current recording in milliseconds.
    pub recording_duration_ms: u64,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Transcription buffer (last successful result).
    pub buffer: String,
}

impl VoiceInputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether voice input is currently active (recording or transcribing).
    pub fn is_active(&self) -> bool {
        matches!(
            self.status,
            VoiceStatus::Recording | VoiceStatus::Transcribing
        )
    }
}

/// Messages for voice input state transitions.
#[derive(Debug, Clone)]
pub enum VoiceInputMessage {
    /// Start recording audio.
    StartRecording,
    /// Stop recording and begin transcription.
    StopRecording,
    /// Transcription completed successfully.
    TranscriptionResult(String),
    /// Transcription failed.
    TranscriptionError(String),
    /// Periodic tick — increments recording duration.
    Tick,
}

/// Update voice input state in response to a message.
pub fn update(state: &mut VoiceInputState, msg: VoiceInputMessage) {
    match msg {
        VoiceInputMessage::StartRecording => {
            state.status = VoiceStatus::Recording;
            state.recording_duration_ms = 0;
            state.error_message = None;
        }
        VoiceInputMessage::StopRecording => {
            if state.status == VoiceStatus::Recording {
                state.status = VoiceStatus::Transcribing;
            }
        }
        VoiceInputMessage::TranscriptionResult(text) => {
            state.buffer = text;
            state.status = VoiceStatus::Idle;
            state.recording_duration_ms = 0;
        }
        VoiceInputMessage::TranscriptionError(err) => {
            state.error_message = Some(err);
            state.status = VoiceStatus::Error;
            state.recording_duration_ms = 0;
        }
        VoiceInputMessage::Tick => {
            if state.status == VoiceStatus::Recording {
                state.recording_duration_ms += 100;
            }
        }
    }
}

/// Render a compact voice indicator for the status bar.
///
/// Returns `None` when idle (nothing to show), or `Some(text)` with the indicator.
pub fn render_indicator(state: &VoiceInputState) -> Option<String> {
    match state.status {
        VoiceStatus::Idle => None,
        VoiceStatus::Recording => {
            let secs = state.recording_duration_ms / 1000;
            let tenths = (state.recording_duration_ms % 1000) / 100;
            Some(format!("\u{1F534} REC {secs}.{tenths}s"))
        }
        VoiceStatus::Transcribing => Some("\u{28FF} Transcribing...".to_string()),
        VoiceStatus::Error => {
            let msg = state.error_message.as_deref().unwrap_or("Unknown error");
            Some(format!("\u{26A0} Voice: {msg}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_idle() {
        let state = VoiceInputState::new();
        assert_eq!(state.status, VoiceStatus::Idle);
        assert_eq!(state.recording_duration_ms, 0);
        assert!(state.error_message.is_none());
        assert!(state.buffer.is_empty());
        assert!(!state.is_active());
    }

    #[test]
    fn start_recording_transitions_to_recording() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        assert_eq!(state.status, VoiceStatus::Recording);
        assert!(state.is_active());
    }

    #[test]
    fn stop_recording_transitions_to_transcribing() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        update(&mut state, VoiceInputMessage::StopRecording);
        assert_eq!(state.status, VoiceStatus::Transcribing);
        assert!(state.is_active());
    }

    #[test]
    fn stop_recording_from_idle_is_noop() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StopRecording);
        assert_eq!(state.status, VoiceStatus::Idle);
    }

    #[test]
    fn transcription_result_returns_to_idle() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        update(&mut state, VoiceInputMessage::StopRecording);
        update(
            &mut state,
            VoiceInputMessage::TranscriptionResult("hello world".into()),
        );
        assert_eq!(state.status, VoiceStatus::Idle);
        assert_eq!(state.buffer, "hello world");
        assert!(!state.is_active());
    }

    #[test]
    fn transcription_error_sets_error_state() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        update(&mut state, VoiceInputMessage::StopRecording);
        update(
            &mut state,
            VoiceInputMessage::TranscriptionError("mic not found".into()),
        );
        assert_eq!(state.status, VoiceStatus::Error);
        assert_eq!(state.error_message.as_deref(), Some("mic not found"));
    }

    #[test]
    fn tick_increments_duration_while_recording() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        update(&mut state, VoiceInputMessage::Tick);
        update(&mut state, VoiceInputMessage::Tick);
        update(&mut state, VoiceInputMessage::Tick);
        assert_eq!(state.recording_duration_ms, 300);
    }

    #[test]
    fn tick_does_not_increment_when_idle() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::Tick);
        assert_eq!(state.recording_duration_ms, 0);
    }

    #[test]
    fn render_indicator_idle_returns_none() {
        let state = VoiceInputState::new();
        assert!(render_indicator(&state).is_none());
    }

    #[test]
    fn render_indicator_recording_shows_red_dot() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        for _ in 0..25 {
            update(&mut state, VoiceInputMessage::Tick);
        }
        let indicator = render_indicator(&state).unwrap();
        assert!(indicator.contains("REC"));
        assert!(indicator.contains("2.5s"));
    }

    #[test]
    fn render_indicator_transcribing_shows_spinner() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        update(&mut state, VoiceInputMessage::StopRecording);
        let indicator = render_indicator(&state).unwrap();
        assert!(indicator.contains("Transcribing"));
    }

    #[test]
    fn render_indicator_error_shows_message() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        update(
            &mut state,
            VoiceInputMessage::TranscriptionError("timeout".into()),
        );
        let indicator = render_indicator(&state).unwrap();
        assert!(indicator.contains("timeout"));
    }

    #[test]
    fn start_recording_clears_previous_error() {
        let mut state = VoiceInputState::new();
        update(&mut state, VoiceInputMessage::StartRecording);
        update(
            &mut state,
            VoiceInputMessage::TranscriptionError("fail".into()),
        );
        assert_eq!(state.status, VoiceStatus::Error);

        update(&mut state, VoiceInputMessage::StartRecording);
        assert_eq!(state.status, VoiceStatus::Recording);
        assert!(state.error_message.is_none());
    }
}
