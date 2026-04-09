//! Configuration management for Git Worktree Manager.
//!
//! Provides typed configuration structs backed by TOML files with
//! atomic writes and sensible defaults.

pub mod agent_config;
pub mod ai_settings;
mod atomic;
pub mod error;
pub mod profile;
pub mod settings;
pub mod voice_config;

pub use agent_config::AgentConfig;
pub use ai_settings::AISettings;
pub use error::{ConfigError, Result};
pub use profile::{Profile, ProfilesConfig};
pub use settings::Settings;
pub use voice_config::VoiceConfig;
