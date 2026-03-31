//! Voice input state for the TUI status bar and indicators.

/// Voice input state for the TUI.
#[derive(Debug, Default)]
pub struct VoiceInputState {
    /// Whether voice input is enabled in configuration.
    pub enabled: bool,
    /// Current visual indicator state.
    pub state: VoiceIndicator,
}

/// Visual indicator for voice input status, shown in the TUI status bar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VoiceIndicator {
    /// Voice input is off or not available.
    #[default]
    Off,
    /// Microphone is ready and waiting for activation.
    Ready,
    /// Actively capturing audio input.
    Listening,
    /// Processing captured audio into text.
    Transcribing,
}

impl VoiceInputState {
    /// Returns a short status string suitable for display in the TUI status bar.
    pub fn status_text(&self) -> &'static str {
        match self.state {
            VoiceIndicator::Off => "",
            VoiceIndicator::Ready => "\u{1f3a4}",
            VoiceIndicator::Listening => "\u{1f3a4} Listening...",
            VoiceIndicator::Transcribing => "\u{1f3a4} Transcribing...",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_input_state_default() {
        let state = VoiceInputState::default();
        assert!(!state.enabled);
        assert_eq!(state.state, VoiceIndicator::Off);
    }

    #[test]
    fn test_voice_indicator_status_text() {
        let mut state = VoiceInputState::default();

        assert_eq!(state.status_text(), "");

        state.state = VoiceIndicator::Ready;
        assert_eq!(state.status_text(), "\u{1f3a4}");

        state.state = VoiceIndicator::Listening;
        assert!(state.status_text().contains("Listening"));

        state.state = VoiceIndicator::Transcribing;
        assert!(state.status_text().contains("Transcribing"));
    }
}
