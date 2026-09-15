pub mod budget;
pub mod global;
mod record;
pub mod route;
pub mod self_budget;
pub mod smoothing;
pub mod startup;
mod store;
pub mod summary;

use std::io;

use chrono::{DateTime, Utc};
use gwt_config::PerfConfig;

#[doc(hidden)]
pub use record::{sanitize_ui_action_field, sanitize_ui_trace_entry};
#[doc(hidden)]
pub use record::{PerfRecord, PerfStream, PerfUnit, PerfViolationDetails};

pub use budget::PerfBudgets;
pub use global::{
    install, install_appending_to_established_log_from_settings, install_from_settings,
    is_installed, record_operation, record_route, RouteTimer,
};
pub use route::PerfRoute;

use store::PerfStore;

/// Perf-log `target` prefix for gwtd operation durations.
pub const OPERATION_TARGET_PREFIX: &str = "gwtd:";

/// `role` recorded for a read-only gwtd operation.
pub const OPERATION_ROLE_READ: &str = "read";

/// `role` recorded for a mutating gwtd operation.
pub const OPERATION_ROLE_MUTATION: &str = "mutation";

/// Kill-switch-aware entry point for all persisted performance samples.
#[doc(hidden)]
pub struct PerfSink {
    store: Option<PerfStore>,
}

impl PerfSink {
    /// Build a sink from the current performance settings.
    ///
    /// Establishes `~/.gwt/logs/perf/` when the kill switch is on. Only the
    /// always-on GUI process should use this; see
    /// [`PerfSink::appending_to_established_log`] for short-lived processes.
    pub fn from_config(config: &PerfConfig) -> io::Result<Self> {
        let store = if config.enabled {
            Some(PerfStore::new(config.retention_days)?)
        } else {
            None
        };
        Ok(Self { store })
    }

    /// Build a sink that appends only to an already-established perf log.
    ///
    /// Issue #4145: `gwtd` runs once per hook, per agent call and per contract
    /// test, so it must not be the process that brings a perf log into
    /// existence. Against a HOME the GUI has never collected in — a hermetic
    /// container fixture, most of all — this is a no-op sink and the HOME stays
    /// byte-identical.
    pub fn appending_to_established_log(config: &PerfConfig) -> io::Result<Self> {
        let store = if config.enabled {
            PerfStore::open_established(config.retention_days)?
        } else {
            None
        };
        Ok(Self { store })
    }

    /// Return whether collection and persistence are enabled.
    pub fn is_enabled(&self) -> bool {
        self.store.is_some()
    }

    /// Persist a pre-built record, or do nothing when the kill switch is off.
    pub fn append(&mut self, record: &PerfRecord) -> io::Result<()> {
        match self.store.as_mut() {
            Some(store) => store.append(record),
            None => Ok(()),
        }
    }

    /// Build and persist one sample record.
    pub fn record_sample(
        &mut self,
        timestamp: DateTime<Utc>,
        stream: PerfStream,
        target: impl AsRef<str>,
        value: f64,
        unit: PerfUnit,
    ) -> io::Result<()> {
        self.append(&PerfRecord::sample(timestamp, stream, target, value, unit))
    }

    /// Build and persist one sustained budget violation.
    pub fn record_violation(
        &mut self,
        timestamp: DateTime<Utc>,
        stream: PerfStream,
        target: impl AsRef<str>,
        value: f64,
        unit: PerfUnit,
        details: PerfViolationDetails,
    ) -> io::Result<()> {
        self.append(&PerfRecord::violation(
            timestamp, stream, target, value, unit, details,
        ))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};
    use gwt_config::PerfConfig;
    use gwt_core::{paths::gwt_logs_dir, test_support::ScopedGwtHome};

    use super::{PerfSink, PerfStream, PerfUnit};

    #[test]
    fn disabled_sink_is_a_noop_without_creating_the_perf_log_directory() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let config = PerfConfig {
            enabled: false,
            ..PerfConfig::default()
        };
        let mut sink = PerfSink::from_config(&config).expect("create disabled sink");

        sink.record_sample(
            Utc.with_ymd_and_hms(2026, 8, 20, 12, 34, 56)
                .single()
                .expect("valid timestamp"),
            PerfStream::Op,
            "gwtd:issue.view",
            12.5,
            PerfUnit::Milliseconds,
        )
        .expect("disabled sink is a no-op");

        assert!(!sink.is_enabled());
        assert!(!gwt_logs_dir().join("perf").exists());
    }
}
