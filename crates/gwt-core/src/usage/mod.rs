//! Provider usage & rate-limit domain (SPEC-2970).
//!
//! Models two axes of usage for the Codex and Claude Code CLIs:
//! - account-level rate-limit windows (shared per provider account)
//! - per-session token / context occupancy
//!
//! Pure parsers are separated from filesystem / network I/O so the parsing
//! logic is deterministic under test. The GUI process owns the polling loop
//! and converts [`UsageSnapshot`] into frontend protocol views.

pub mod account_limits;
pub mod api_error;
pub mod cache;
pub mod claude;
pub mod codex;
pub mod consumption;
pub mod grok;
pub(crate) mod incremental;
pub mod limit_notice;
pub mod model_context;
pub mod snapshot_store;
pub mod state;
pub mod types;

pub use api_error::{describe_provider_api_error, detect_provider_api_error, ProviderApiError};
pub use consumption::{ConsumptionBreakdown, DayConsumption, ProviderConsumption};
pub use limit_notice::{
    describe_provider_limit_notice, detect_provider_limit_notice, ProviderLimitNotice,
};
pub use snapshot_store::{ProviderUsageReading, ProviderUsageWindowReading};
pub use types::{
    CodexCredits, LimitObservation, ProviderUsage, SessionUsage, UsageProvider, UsageSnapshot,
    UsageState, UsageWindow, WindowKind,
};

// Resolve the offset at this instant so local calendar operations respect
// historical and seasonal offset changes.
fn local_time_at(seconds: i64) -> time::OffsetDateTime {
    let utc = time::OffsetDateTime::from_unix_timestamp(seconds)
        .expect("UTC timestamp in the supported range");
    utc.to_offset(time::UtcOffset::local_offset_at(utc).unwrap_or(time::UtcOffset::UTC))
}
