//! Voice input state tracking (minimal stub for Phase 1).

/// Voice input state.
#[derive(Debug, Clone, Default)]
pub struct VoiceState {
    /// Whether voice input is currently active.
    pub active: bool,
    /// Transcription buffer.
    pub buffer: String,
}

impl VoiceState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle voice input on/off.
    pub fn toggle(&mut self) {
        self.active = !self.active;
        if !self.active {
            self.buffer.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_state_default_is_inactive() {
        let state = VoiceState::new();
        assert!(!state.active);
        assert!(state.buffer.is_empty());
    }

    #[test]
    fn voice_toggle_activates_and_deactivates() {
        let mut state = VoiceState::new();
        state.toggle();
        assert!(state.active);

        state.buffer = "hello".into();
        state.toggle();
        assert!(!state.active);
        assert!(state.buffer.is_empty());
    }
}
