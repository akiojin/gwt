//! gwt-voice: Voice input backend abstraction for gwt.

pub mod backend;
pub mod config;
pub mod noop;
pub mod qwen3;

pub use backend::{VoiceBackend, VoiceError};
pub use config::VoiceState;
pub use noop::NoOpVoiceBackend;
pub use qwen3::Qwen3AsrRecorder;
pub use session::VoiceSession;
pub mod session;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

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
        assert!(matches!(result.unwrap_err(), VoiceError::NotAvailable));
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
    fn voice_state_transcribing_and_error_helpers_work() {
        let transcribing = VoiceState::Transcribing;
        assert!(transcribing.is_transcribing());
        assert!(!transcribing.is_error());

        let error = VoiceState::Error("mic failed".to_string());
        assert!(error.is_error());
        assert!(!error.is_transcribing());
    }

    #[test]
    fn noop_default_trait() {
        let backend: NoOpVoiceBackend = Default::default();
        assert!(!backend.is_available());
    }

    // --- T-007: Recording timeout tests ---

    #[test]
    fn voice_backend_default_max_recording_is_30_seconds() {
        let backend = NoOpVoiceBackend::new();
        assert_eq!(backend.max_recording_seconds(), Some(30));
    }

    #[test]
    fn recording_timeout_error_carries_duration() {
        let err = VoiceError::RecordingTimeout(30);
        assert!(err.to_string().contains("30"));
        assert!(matches!(err, VoiceError::RecordingTimeout(30)));
    }

    #[test]
    fn fake_backend_can_enforce_recording_timeout() {
        struct TimedBackend {
            elapsed: u32,
        }
        impl VoiceBackend for TimedBackend {
            fn start_recording(&mut self) -> Result<(), VoiceError> {
                Ok(())
            }
            fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
                if self.elapsed >= self.max_recording_seconds().unwrap_or(u32::MAX) {
                    return Err(VoiceError::RecordingTimeout(
                        self.max_recording_seconds().unwrap(),
                    ));
                }
                Ok(vec![1, 2, 3])
            }
            fn transcribe(&self, _audio: &[u8]) -> Result<String, VoiceError> {
                Ok("text".to_string())
            }
            fn is_available(&self) -> bool {
                true
            }
            fn max_recording_seconds(&self) -> Option<u32> {
                Some(30)
            }
        }

        let mut backend = TimedBackend { elapsed: 31 };
        backend.start_recording().unwrap();
        let err = backend.stop_recording().unwrap_err();
        assert!(matches!(err, VoiceError::RecordingTimeout(30)));
    }

    // --- T-008: Silence detection tests ---

    #[test]
    fn voice_backend_default_silence_timeout_is_3_seconds() {
        let backend = NoOpVoiceBackend::new();
        assert_eq!(backend.silence_timeout_seconds(), Some(3));
    }

    #[test]
    fn silence_detected_error_carries_duration() {
        let err = VoiceError::SilenceDetected(3);
        assert!(err.to_string().contains("3"));
        assert!(matches!(err, VoiceError::SilenceDetected(3)));
    }

    #[test]
    fn fake_backend_can_stop_on_silence() {
        struct SilenceBackend {
            silence_seconds: u32,
        }
        impl VoiceBackend for SilenceBackend {
            fn start_recording(&mut self) -> Result<(), VoiceError> {
                Ok(())
            }
            fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
                if let Some(threshold) = self.silence_timeout_seconds() {
                    if self.silence_seconds >= threshold {
                        return Err(VoiceError::SilenceDetected(threshold));
                    }
                }
                Ok(vec![1, 2, 3])
            }
            fn transcribe(&self, _audio: &[u8]) -> Result<String, VoiceError> {
                Ok("text".to_string())
            }
            fn is_available(&self) -> bool {
                true
            }
            fn silence_timeout_seconds(&self) -> Option<u32> {
                Some(3)
            }
        }

        let mut backend = SilenceBackend { silence_seconds: 4 };
        backend.start_recording().unwrap();
        let err = backend.stop_recording().unwrap_err();
        assert!(matches!(err, VoiceError::SilenceDetected(3)));
    }

    // --- T-011: Qwen3AsrRecorder tests ---

    #[test]
    fn qwen3_backend_is_not_available() {
        let backend = Qwen3AsrRecorder::new();
        assert!(!backend.is_available());
    }

    #[test]
    fn qwen3_start_recording_returns_model_not_loaded() {
        let mut backend = Qwen3AsrRecorder::new();
        let err = backend.start_recording().unwrap_err();
        assert!(matches!(err, VoiceError::ModelNotLoaded(_)));
        assert!(err.to_string().contains("Qwen3 ASR model not loaded"));
    }

    #[test]
    fn qwen3_stop_recording_returns_not_recording() {
        let mut backend = Qwen3AsrRecorder::new();
        let err = backend.stop_recording().unwrap_err();
        assert!(matches!(err, VoiceError::NotRecording));
    }

    #[test]
    fn qwen3_transcribe_returns_model_not_loaded() {
        let backend = Qwen3AsrRecorder::new();
        let err = backend.transcribe(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, VoiceError::ModelNotLoaded(_)));
    }

    #[test]
    fn qwen3_default_trait() {
        let backend: Qwen3AsrRecorder = Default::default();
        assert!(!backend.is_available());
    }

    #[test]
    fn qwen3_session_start_surfaces_error() {
        let backend = Qwen3AsrRecorder::new();
        let mut session = VoiceSession::new(backend);

        let err = session.start_recording().unwrap_err();
        assert!(matches!(err, VoiceError::ModelNotLoaded(_)));
        assert!(session.state().is_error());
    }

    #[test]
    fn voice_session_runs_start_stop_and_transcribe_flow() {
        #[derive(Default)]
        struct FakeBackend {
            started: RefCell<bool>,
            stopped: RefCell<bool>,
        }

        impl VoiceBackend for FakeBackend {
            fn start_recording(&mut self) -> Result<(), VoiceError> {
                *self.started.borrow_mut() = true;
                Ok(())
            }

            fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
                *self.stopped.borrow_mut() = true;
                Ok(vec![1, 2, 3])
            }

            fn transcribe(&self, audio: &[u8]) -> Result<String, VoiceError> {
                assert_eq!(audio, &[1, 2, 3]);
                Ok("hello world".to_string())
            }

            fn is_available(&self) -> bool {
                true
            }
        }

        let backend = FakeBackend::default();
        let mut session = VoiceSession::new(backend);

        session.start_recording().unwrap();
        assert!(session.state().is_recording());

        let audio = session.stop_recording().unwrap();
        assert_eq!(audio, vec![1, 2, 3]);
        assert!(session.state().is_transcribing());

        let text = session.transcribe_captured_audio().unwrap();
        assert_eq!(text, "hello world");
        assert!(session.state().is_idle());
        assert_eq!(session.transcript(), Some("hello world"));
    }

    #[test]
    fn voice_session_surfaces_noop_backend_failure() {
        let backend = NoOpVoiceBackend::new();
        let mut session = VoiceSession::new(backend);

        let err = session.start_recording().unwrap_err();
        assert!(matches!(err, VoiceError::NotAvailable));
        assert!(session.state().is_error());
    }
}
