//! Session-kind signal for lane-specific managed assets (SPEC-3247).
//!
//! The 2-lane work model (SPEC-3245) splits launched agent sessions into a
//! **Curate** lane (branchless ephemeral *intake* sessions that register
//! Issues/SPECs and produce no Work) and an **Execute** lane (branch-backed
//! *producing-work* sessions). Managed coordination guidance and managed
//! hooks must adapt to the lane, but they had no signal to branch on: launch
//! exported `GWT_SESSION_ID` but nothing describing the session kind.
//!
//! This module owns the `SessionKind` type and the `GWT_SESSION_KIND`
//! environment key. It lives in `gwt-skills` because that is the lowest crate
//! shared by the guidance generator (this crate), the launch signal exporter
//! (`gwt-agent`, which depends on `gwt-skills`), and the hooks (`gwt`).

/// Environment variable carrying the launched session's kind so managed hooks
/// and coordination guidance can adapt to the Curate (intake) vs Execute
/// (execution) lane.
///
/// Absent, empty, or unknown values are treated as [`SessionKind::Execution`]
/// so older launches and already-materialized worktrees keep the current
/// producing-work behavior (SPEC-3247 FR-004).
pub const GWT_SESSION_KIND_ENV: &str = "GWT_SESSION_KIND";

/// Which lane a launched agent session belongs to (SPEC-3245 / SPEC-3247).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// Curate lane: a branchless, ephemeral intake session that discusses,
    /// plans, and registers Issues/SPECs. It produces no Work, so it is not
    /// asked to maintain Work state (`workspace.update`).
    Intake,
    /// Execute lane: a branch-backed producing-work session (implement,
    /// verify, PR). This is the default and keeps the full producing-work
    /// guidance and hooks.
    Execution,
}

impl SessionKind {
    /// Derive the kind from the launch's ephemeral flag
    /// (`LaunchConfig.is_ephemeral`, SPEC-3214): an ephemeral intake launch is
    /// [`SessionKind::Intake`]; every other launch is
    /// [`SessionKind::Execution`].
    #[must_use]
    pub fn from_is_ephemeral(is_ephemeral: bool) -> Self {
        if is_ephemeral {
            Self::Intake
        } else {
            Self::Execution
        }
    }

    /// The env string written to [`GWT_SESSION_KIND_ENV`].
    #[must_use]
    pub fn as_env_str(self) -> &'static str {
        match self {
            Self::Intake => "intake",
            Self::Execution => "execution",
        }
    }

    /// Parse from a [`GWT_SESSION_KIND_ENV`] value. Absent, empty, or unknown
    /// values fall back to [`SessionKind::Execution`] — a fail-safe default
    /// that preserves the current producing-work behavior (FR-004).
    #[must_use]
    pub fn from_env_str(value: Option<&str>) -> Self {
        match value {
            Some(raw) if raw.trim().eq_ignore_ascii_case("intake") => Self::Intake,
            _ => Self::Execution,
        }
    }

    /// Read the kind from the process environment ([`GWT_SESSION_KIND_ENV`]).
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_env_str(std::env::var(GWT_SESSION_KIND_ENV).ok().as_deref())
    }

    /// True for the Curate (intake) lane.
    #[must_use]
    pub fn is_intake(self) -> bool {
        matches!(self, Self::Intake)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_is_ephemeral_maps_intake_and_execution() {
        assert_eq!(SessionKind::from_is_ephemeral(true), SessionKind::Intake);
        assert_eq!(
            SessionKind::from_is_ephemeral(false),
            SessionKind::Execution
        );
    }

    #[test]
    fn as_env_str_roundtrips_through_from_env_str() {
        assert_eq!(SessionKind::Intake.as_env_str(), "intake");
        assert_eq!(SessionKind::Execution.as_env_str(), "execution");
        assert_eq!(
            SessionKind::from_env_str(Some(SessionKind::Intake.as_env_str())),
            SessionKind::Intake
        );
        assert_eq!(
            SessionKind::from_env_str(Some(SessionKind::Execution.as_env_str())),
            SessionKind::Execution
        );
    }

    #[test]
    fn from_env_str_defaults_to_execution_when_absent_or_unknown() {
        assert_eq!(SessionKind::from_env_str(None), SessionKind::Execution);
        assert_eq!(SessionKind::from_env_str(Some("")), SessionKind::Execution);
        assert_eq!(
            SessionKind::from_env_str(Some("garbage")),
            SessionKind::Execution
        );
        // Case-insensitive + surrounding whitespace tolerant for intake.
        assert_eq!(
            SessionKind::from_env_str(Some("  Intake ")),
            SessionKind::Intake
        );
    }

    #[test]
    fn is_intake_reflects_variant() {
        assert!(SessionKind::Intake.is_intake());
        assert!(!SessionKind::Execution.is_intake());
    }

    #[test]
    fn session_kind_env_key_is_stable() {
        assert_eq!(GWT_SESSION_KIND_ENV, "GWT_SESSION_KIND");
    }
}
