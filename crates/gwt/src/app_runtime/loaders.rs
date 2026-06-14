//! Branch / Logs window loaders split out of `app_runtime/mod.rs` for
//! SPEC-3064 Phase 1 (Pass 1).
//!
//! Owns:
//! - [`AppRuntime::load_branches_events`] — Branches/Work surface async
//!   branch list load via `crate::repo_browser::spawn_branch_load_async`
//! - [`AppRuntime::load_logs_events`] — Logs surface load through the
//!   SPEC-1924 FR-035 reader ([`load_log_entries_from_dir`]) with the
//!   FR-036 skipped-lines warning ([`skipped_lines_warning`])
//!
//! Behavior-preserving move: the Board window loaders live in `board.rs`,
//! Knowledge loaders in `knowledge.rs`.

use std::path::Path;

use super::{spawn_branch_load_async, AppRuntime, BackendEvent, OutboundEvent, WindowPreset};

/// Read the active canonical log file via the SPEC-1924 FR-035 reader.
///
/// Returns the decoded snapshot together with `ReadDiagnostics` so the caller
/// can surface a non-blocking warning when malformed lines were skipped
/// (FR-036 / SC-010). IO errors other than `NotFound` are forwarded as a
/// human-readable message so the Logs window can switch to an error state
/// without crashing the agent.
pub(super) fn load_log_entries_from_dir(
    log_dir: &Path,
) -> Result<gwt_core::logging::ReadOutcome, String> {
    let path = gwt_core::logging::current_log_file(log_dir);
    gwt_core::logging::read_log_file(&path)
        .map_err(|error| format!("Failed to read log file {}: {error}", path.display()))
}

/// Build the synthetic warning event surfaced when `read_log_file` skipped
/// malformed lines. Keeps the message phrasing consistent with the Logs
/// window expectation of a single notice per load (FR-036 / SC-010).
pub(super) fn skipped_lines_warning(
    diagnostics: &gwt_core::logging::ReadDiagnostics,
) -> gwt_core::logging::LogEvent {
    let count = diagnostics.skipped;
    let plural = if count == 1 { "line" } else { "lines" };
    gwt_core::logging::LogEvent::new(
        gwt_core::logging::LogLevel::Warn,
        "gwt_core::logging::reader",
        format!(
            "Skipped {count} malformed {plural} while reading {}",
            diagnostics.path.display()
        ),
    )
}

impl AppRuntime {
    pub(crate) fn load_branches_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };

        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: format!("Window preset {:?} is not a Work surface", window.preset),
                },
            )];
        }

        spawn_branch_load_async(
            self.proxy.clone(),
            id.to_string(),
            tab.project_root.clone(),
            self.active_session_branches_for_tab(&address.tab_id),
            // Pass the sessions dir so the async branch load reads resume
            // candidates fresh from disk instead of the stale in-memory cache
            // snapshot (#2995).
            self.sessions_dir.clone(),
        );
        Vec::new()
    }

    pub(crate) fn load_logs_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Logs {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Window is not a Logs surface".to_string(),
                },
            )];
        }

        match load_log_entries_from_dir(&self.log_dir) {
            Ok(outcome) => {
                let mut events = vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::LogEntries {
                        id: id.to_string(),
                        entries: outcome.entries,
                    },
                )];
                if outcome.diagnostics.skipped > 0 {
                    events.push(OutboundEvent::reply(
                        client_id,
                        BackendEvent::LogEntryAppended {
                            entry: skipped_lines_warning(&outcome.diagnostics),
                        },
                    ));
                }
                events
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: error,
                },
            )],
        }
    }
}
