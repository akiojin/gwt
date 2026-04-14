//! Stateful voice session orchestration.

use crate::backend::{VoiceBackend, VoiceError};
use crate::config::VoiceState;

/// A voice session that coordinates backend start/stop/transcription.
pub struct VoiceSession<B: VoiceBackend> {
    backend: B,
    state: VoiceState,
    last_audio: Option<Vec<u8>>,
    last_transcript: Option<String>,
}

impl<B: VoiceBackend> VoiceSession<B> {
    /// Create a new session from a backend implementation.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            state: VoiceState::Idle,
            last_audio: None,
            last_transcript: None,
        }
    }

    /// Borrow the backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Current state of the voice session.
    pub fn state(&self) -> &VoiceState {
        &self.state
    }

    /// Last transcribed text, if available.
    pub fn transcript(&self) -> Option<&str> {
        self.last_transcript.as_deref()
    }

    /// Last captured audio, if available.
    pub fn captured_audio(&self) -> Option<&[u8]> {
        self.last_audio.as_deref()
    }

    /// Start recording audio through the backend.
    pub fn start_recording(&mut self) -> Result<(), VoiceError> {
        if self.state.is_recording() || self.state.is_transcribing() {
            self.state = VoiceState::Error(VoiceError::AlreadyRecording.to_string());
            return Err(VoiceError::AlreadyRecording);
        }

        match self.backend.start_recording() {
            Ok(()) => {
                self.state = VoiceState::Recording;
                self.last_audio = None;
                self.last_transcript = None;
                Ok(())
            }
            Err(err) => {
                self.state = VoiceState::Error(err.to_string());
                Err(err)
            }
        }
    }

    /// Stop recording and capture the backend audio payload.
    pub fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
        if !self.state.is_recording() {
            self.state = VoiceState::Error(VoiceError::NotRecording.to_string());
            return Err(VoiceError::NotRecording);
        }

        match self.backend.stop_recording() {
            Ok(audio) => {
                self.state = VoiceState::Transcribing;
                self.last_audio = Some(audio.clone());
                Ok(audio)
            }
            Err(err) => {
                self.state = VoiceState::Error(err.to_string());
                Err(err)
            }
        }
    }

    /// Transcribe the most recently captured audio payload.
    pub fn transcribe_captured_audio(&mut self) -> Result<String, VoiceError> {
        if !self.state.is_transcribing() {
            self.state = VoiceState::Error(VoiceError::NotRecording.to_string());
            return Err(VoiceError::NotRecording);
        }

        let audio = self.last_audio.as_deref().ok_or(VoiceError::NotRecording)?;

        match self.backend.transcribe(audio) {
            Ok(text) => {
                self.state = VoiceState::Idle;
                self.last_transcript = Some(text.clone());
                self.last_audio = None;
                Ok(text)
            }
            Err(err) => {
                self.state = VoiceState::Error(err.to_string());
                Err(err)
            }
        }
    }

    /// Convenience helper that finishes the full recording flow.
    pub fn stop_and_transcribe(&mut self) -> Result<String, VoiceError> {
        let _ = self.stop_recording()?;
        self.transcribe_captured_audio()
    }

    /// Reset the session to an idle state and clear captured data.
    pub fn reset(&mut self) {
        self.state = VoiceState::Idle;
        self.last_audio = None;
        self.last_transcript = None;
    }

    /// Whether the backend is available.
    pub fn is_available(&self) -> bool {
        self.backend.is_available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct FakeBackend {
        started: Cell<bool>,
        stopped: Cell<bool>,
    }

    impl VoiceBackend for FakeBackend {
        fn start_recording(&mut self) -> Result<(), VoiceError> {
            if self.started.get() {
                return Err(VoiceError::AlreadyRecording);
            }
            self.started.set(true);
            Ok(())
        }

        fn stop_recording(&mut self) -> Result<Vec<u8>, VoiceError> {
            if !self.started.get() {
                return Err(VoiceError::NotRecording);
            }
            self.stopped.set(true);
            Ok(vec![1, 2, 3])
        }

        fn transcribe(&self, audio: &[u8]) -> Result<String, VoiceError> {
            if !self.stopped.get() {
                return Err(VoiceError::NotRecording);
            }
            assert_eq!(audio, &[1, 2, 3]);
            Ok("hello world".to_string())
        }

        fn is_available(&self) -> bool {
            true
        }
    }

    #[test]
    fn session_runs_full_recording_flow() {
        let backend = FakeBackend::default();
        let mut session = VoiceSession::new(backend);

        session.start_recording().unwrap();
        assert!(session.state().is_recording());

        let audio = session.stop_recording().unwrap();
        assert_eq!(audio, vec![1, 2, 3]);
        assert!(session.state().is_transcribing());
        assert_eq!(session.captured_audio(), Some(&[1, 2, 3][..]));

        let text = session.transcribe_captured_audio().unwrap();
        assert_eq!(text, "hello world");
        assert!(session.state().is_idle());
        assert_eq!(session.transcript(), Some("hello world"));
    }

    #[test]
    fn session_stop_without_start_returns_not_recording() {
        let backend = FakeBackend::default();
        let mut session = VoiceSession::new(backend);

        let err = session.stop_recording().unwrap_err();
        assert!(matches!(err, VoiceError::NotRecording));
        assert!(session.state().is_error());
    }

    #[test]
    fn session_start_on_noop_backend_surfaces_error_state() {
        let backend = crate::NoOpVoiceBackend::new();
        let mut session = VoiceSession::new(backend);

        let err = session.start_recording().unwrap_err();
        assert!(matches!(err, VoiceError::NotAvailable));
        assert!(session.state().is_error());
        assert!(!session.is_available());
    }
}
