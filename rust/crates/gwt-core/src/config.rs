//! Configuration management module
//!
//! Handles TOML configuration files with automatic migration from JSON.

mod migration;
mod profile;
mod session;
mod settings;

pub use migration::migrate_json_to_toml;
pub use profile::Profile;
pub use session::Session;
pub use settings::Settings;
