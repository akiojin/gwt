//! Configuration management for Git Worktree Manager.
//!
//! Provides typed configuration structs backed by TOML files with
//! atomic writes and sensible defaults.

pub mod agent_config;
pub mod ai_settings;
pub mod atomic;
pub mod board_config;
pub mod error;
pub mod locale;
pub mod profile;
pub mod settings;
pub mod voice_config;

pub use agent_config::AgentConfig;
pub use ai_settings::AISettings;
pub use board_config::{
    BoardConfig, BoardProviderKind, SlackConfig, TeamsConfig, DEFAULT_OAUTH_REDIRECT_PORT,
};
pub use error::{ConfigError, Result};
pub use locale::{
    detect_user_locale, detect_user_locale_from, detect_user_locale_from_env_and_system,
};
pub use profile::{Profile, ProfilesConfig};
pub use settings::Settings;
pub use voice_config::VoiceConfig;
