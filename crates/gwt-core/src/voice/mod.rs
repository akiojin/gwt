//! Voice input abstractions for speech-to-text integration.
//!
//! This module provides trait-based voice backend abstractions
//! that can be implemented by different speech recognition engines.
//! The core module itself does not depend on any specific ASR library.

mod runtime;

pub use runtime::{
    detect_voice_support, NoOpVoiceBackend, TranscriptionResult, VoiceBackend, VoiceConfig,
    VoiceState,
};
