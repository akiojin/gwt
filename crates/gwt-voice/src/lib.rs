//! gwt-voice: Voice input backend abstraction for gwt.

pub mod backend;
pub mod config;
pub mod noop;

pub use backend::{VoiceBackend, VoiceError};
pub use config::VoiceState;
pub use noop::NoOpVoiceBackend;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_backend_is_not_available() {
        let backend = NoOpVoiceBackend::new();
        assert!(!backend.is_available());
    }

    #[test]
    fn noop_start_recording_returns_error() {
        let mut backend = NoOpVoiceBackend::new();
        let result = backend.start_recording();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VoiceError::NotAvailable
        ));
    }

    #[test]
    fn noop_stop_recording_returns_error() {
        let mut backend = NoOpVoiceBackend::new();
        let result = backend.stop_recording();
        assert!(result.is_err());
    }

    #[test]
    fn noop_transcribe_returns_error() {
        let backend = NoOpVoiceBackend::new();
        let result = backend.transcribe(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn voice_state_idle_by_default() {
        let state = VoiceState::Idle;
        assert!(state.is_idle());
        assert!(!state.is_recording());
    }

    #[test]
    fn voice_state_recording() {
        let state = VoiceState::Recording;
        assert!(state.is_recording());
        assert!(!state.is_idle());
    }

    #[test]
    fn voice_state_error_holds_message() {
        let state = VoiceState::Error("mic failed".to_string());
        assert!(!state.is_idle());
        assert!(!state.is_recording());
        assert_eq!(state, VoiceState::Error("mic failed".to_string()));
    }

    #[test]
    fn noop_default_trait() {
        let backend: NoOpVoiceBackend = Default::default();
        assert!(!backend.is_available());
    }
}
