//! The user-facing routes that carry a perf budget (Issue #4145 AC-1).
//!
//! A route is one end-to-end path a person actually waits on. Naming them here
//! — rather than deriving a label from whatever event happened to be dispatched
//! — keeps the perf log stable across protocol refactors and lets
//! `perf.summary` report per-route budgets without re-deriving them.

use super::budget::PerfBudgets;

/// Individual ceiling for process start to canvas-ready, in milliseconds.
///
/// FR-005 exempts launch-class work from the 100ms interaction budget and asks
/// for an individual ceiling instead. Startup pays for logging init, session
/// restore, worktree enumeration and the embedded server bind.
pub const DEFAULT_STARTUP_BUDGET_MS: f64 = 3_000.0;

/// Individual ceiling for opening a project, in milliseconds.
///
/// Covers the blocking-pool prepare (git dirty/lock probes, workspace restore)
/// plus the event-loop commit.
pub const DEFAULT_PROJECT_OPEN_BUDGET_MS: f64 = 2_000.0;

/// Individual ceiling for creating an agent pane, in milliseconds.
///
/// Covers worktree resolution, Docker runtime probing and the PTY spawn.
pub const DEFAULT_PANE_CREATE_BUDGET_MS: f64 = 5_000.0;

/// Individual ceiling for one index search attempt, in milliseconds.
///
/// Covers the embedding model query and the batched scope search.
pub const DEFAULT_SEARCH_BUDGET_MS: f64 = 2_000.0;

/// Individual ceiling for one Work events ingest trigger, in milliseconds.
///
/// Issue #4397 AC-2: runs on a background worker after a tab switch, agent
/// launch or project open, and must stay proportional to what changed.
pub const DEFAULT_WORK_EVENTS_INGEST_BUDGET_MS: f64 = 5_000.0;

const TARGET_PREFIX: &str = "route:";
/// Perf-log `target` prefix for one phase inside a route (Issue #4283 AC-5).
const PHASE_TARGET_PREFIX: &str = "phase:";
/// Perf-log `target` prefix for a non-time quantity of a route (Issue #4397).
const METRIC_TARGET_PREFIX: &str = "metric:";

/// One instrumented end-to-end route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PerfRoute {
    /// Process start to the canvas reporting bounds.
    Startup,
    /// Project open request to the committed navigation.
    ProjectOpen,
    /// Project tab switch, handled synchronously on the GUI event loop.
    ProjectSwitch,
    /// Agent pane spawn request to launch completion.
    PaneCreate,
    /// Accepted pane close, handled synchronously on the GUI event loop.
    PaneClose,
    /// Prompt submission reaching the PTY.
    PromptSend,
    /// One index search attempt.
    Search,
    /// One Work events ingest trigger, off the GUI event loop.
    WorkEventsIngest,
}

impl PerfRoute {
    /// Every instrumented route, in the order `perf.summary` reports them.
    pub const ALL: [PerfRoute; 8] = [
        PerfRoute::Startup,
        PerfRoute::ProjectOpen,
        PerfRoute::ProjectSwitch,
        PerfRoute::PaneCreate,
        PerfRoute::PaneClose,
        PerfRoute::PromptSend,
        PerfRoute::Search,
        PerfRoute::WorkEventsIngest,
    ];

    /// Stable short name, without the perf-log target prefix.
    pub fn name(self) -> &'static str {
        match self {
            PerfRoute::Startup => "startup",
            PerfRoute::ProjectOpen => "project.open",
            PerfRoute::ProjectSwitch => "project.switch",
            PerfRoute::PaneCreate => "pane.create",
            PerfRoute::PaneClose => "pane.close",
            PerfRoute::PromptSend => "prompt.send",
            PerfRoute::Search => "search",
            PerfRoute::WorkEventsIngest => "work_events.ingest",
        }
    }

    /// The perf-log `target` field for this route.
    pub fn target(self) -> String {
        format!("{TARGET_PREFIX}{}", self.name())
    }

    /// The perf-log `target` field for one named phase of this route.
    ///
    /// Phases attribute a route's total to its dominant step; they carry no
    /// budget of their own, so `from_target` never resolves one.
    pub fn phase_target(self, phase: &str) -> String {
        format!("{PHASE_TARGET_PREFIX}{}.{phase}", self.name())
    }

    /// The perf-log `target` field for one named quantity of this route, such
    /// as a state size or an item count. Metrics carry no budget either.
    pub fn metric_target(self, metric: &str) -> String {
        format!("{METRIC_TARGET_PREFIX}{}.{metric}", self.name())
    }

    /// Recover a route from a perf-log `target` field.
    pub fn from_target(target: &str) -> Option<Self> {
        let name = target.strip_prefix(TARGET_PREFIX)?;
        PerfRoute::ALL
            .into_iter()
            .find(|route| route.name() == name)
    }

    /// The millisecond budget for this route.
    ///
    /// Interaction-class routes run synchronously on the GUI event loop and are
    /// held to the RAIL response budget. Launch-class routes carry the
    /// individual ceilings FR-005 asks for.
    pub fn budget_ms(self, budgets: &PerfBudgets) -> f64 {
        match self {
            PerfRoute::ProjectSwitch | PerfRoute::PaneClose | PerfRoute::PromptSend => {
                budgets.ui_response_ms
            }
            PerfRoute::Startup => DEFAULT_STARTUP_BUDGET_MS,
            PerfRoute::ProjectOpen => DEFAULT_PROJECT_OPEN_BUDGET_MS,
            PerfRoute::PaneCreate => DEFAULT_PANE_CREATE_BUDGET_MS,
            PerfRoute::Search => DEFAULT_SEARCH_BUDGET_MS,
            PerfRoute::WorkEventsIngest => DEFAULT_WORK_EVENTS_INGEST_BUDGET_MS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_round_trips_through_its_perf_log_target() {
        for route in PerfRoute::ALL {
            let target = route.target();
            assert!(target.starts_with("route:"), "unexpected target {target}");
            assert_eq!(PerfRoute::from_target(&target), Some(route));
        }
        assert_eq!(PerfRoute::from_target("gwtd:issue.view"), None);
        assert_eq!(PerfRoute::from_target("route:unknown"), None);
    }

    #[test]
    fn route_names_are_unique_so_summaries_do_not_collapse_paths() {
        let mut names: Vec<&str> = PerfRoute::ALL.iter().map(|route| route.name()).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique);
    }

    #[test]
    fn interaction_routes_follow_the_configured_ui_response_budget() {
        let budgets = PerfBudgets {
            ui_response_ms: 42.0,
            ..PerfBudgets::default()
        };

        assert_eq!(PerfRoute::ProjectSwitch.budget_ms(&budgets), 42.0);
        assert_eq!(PerfRoute::PaneClose.budget_ms(&budgets), 42.0);
        assert_eq!(PerfRoute::PromptSend.budget_ms(&budgets), 42.0);
    }

    #[test]
    fn launch_class_routes_use_their_individual_ceilings() {
        let budgets = PerfBudgets::default();

        assert_eq!(
            PerfRoute::Startup.budget_ms(&budgets),
            DEFAULT_STARTUP_BUDGET_MS
        );
        assert_eq!(
            PerfRoute::ProjectOpen.budget_ms(&budgets),
            DEFAULT_PROJECT_OPEN_BUDGET_MS
        );
        assert_eq!(
            PerfRoute::PaneCreate.budget_ms(&budgets),
            DEFAULT_PANE_CREATE_BUDGET_MS
        );
        assert_eq!(
            PerfRoute::Search.budget_ms(&budgets),
            DEFAULT_SEARCH_BUDGET_MS
        );
        assert_eq!(
            PerfRoute::WorkEventsIngest.budget_ms(&budgets),
            DEFAULT_WORK_EVENTS_INGEST_BUDGET_MS
        );
    }

    /// Issue #4397 AC-4: the Work events ingest is its own route.
    #[test]
    fn work_events_ingest_route_names_its_metrics_beside_the_route() {
        assert!(PerfRoute::ALL.contains(&PerfRoute::WorkEventsIngest));
        assert_eq!(
            PerfRoute::WorkEventsIngest.target(),
            "route:work_events.ingest"
        );
        assert_eq!(
            PerfRoute::WorkEventsIngest.metric_target("state_bytes"),
            "metric:work_events.ingest.state_bytes"
        );
        assert_eq!(
            PerfRoute::from_target("metric:work_events.ingest.state_bytes"),
            None
        );
    }
}
