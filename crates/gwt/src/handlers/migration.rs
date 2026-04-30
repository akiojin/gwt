//! WebSocket bridge between the migration modal (WebView) and the
//! `gwt_core::migration::executor` orchestrator (SPEC-1934 US-6).
//!
//! Filled in by Phase 10 tasks (T-097/T-098).

use std::path::Path;

/// Placeholder entry point. Phase 10 tasks will wire this up to the embedded
/// axum server and emit `migration:detected | start | skip | quit | phase |
/// done | error` events.
pub fn handle_migration_request(_project_root: &Path) {
    // intentionally empty until T-097
}
