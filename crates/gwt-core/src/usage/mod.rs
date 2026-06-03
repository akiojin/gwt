//! Provider usage & rate-limit domain (SPEC-2970).
//!
//! Models two axes of usage for the Codex and Claude Code CLIs:
//! - account-level rate-limit windows (shared per provider account)
//! - per-session token / context occupancy
//!
//! Pure parsers are separated from filesystem / network I/O so the parsing
//! logic is deterministic under test. The GUI process owns the polling loop
//! and converts [`UsageSnapshot`] into frontend protocol views.

pub mod claude;
pub mod codex;
pub mod consumption;
pub mod model_context;
pub mod state;
pub mod types;

pub use consumption::{ConsumptionBreakdown, DayConsumption, ProviderConsumption};
pub use types::{
    ProviderUsage, SessionUsage, UsageProvider, UsageSnapshot, UsageState, UsageWindow, WindowKind,
};
