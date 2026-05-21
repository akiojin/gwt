//! `ConsoleTeeLayer` — tracing layer that forwards selected log events
//! into the `ProcessConsoleHub` so the per-kind Console window tabs see
//! both the actual gh / git / docker / agent / runner spawns **and** the
//! gwt-side operational notes (e.g. "refreshing workspace projection",
//! "launch preflight cleared"). This is what makes the Console window
//! feel like VS Code's Output panel rather than a raw `git` log.
//!
//! Mapping (target prefix → ProcessKind):
//!
//! | target prefix                       | kind             |
//! |-------------------------------------|------------------|
//! | `gwt_git*`, `gwt::git*`              | Git              |
//! | `gwt_github*`, `gwt::pr*`            | Gh               |
//! | `gwt_docker*`                        | Docker           |
//! | `gwt::launch*`, `gwt::wizard*`,     | AgentBootstrap   |
//! | `gwt_agent*`, `gwt::agent_launch`    |                  |
//! | `gwt::index*`, `gwt_core::index*`    | IndexRunner      |
//!
//! Targets that have no mapping (e.g. `gwt::startup`,
//! `gwt::open_server_url`) are silently ignored. The
//! `gwt.process.summary` and `gwt.process.line` targets are also
//! ignored because their data is already pushed directly to the hub by
//! `spawn_logged` / `run_git_logged`; forwarding them here would
//! duplicate every spawn footer.

use std::fmt::Write;

use tracing::{Event, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

use crate::process_console::{ProcessConsoleHub, ProcessKind, ProcessLine, ProcessStream};

/// Tee gwt-domain tracing events into the global `ProcessConsoleHub`
/// after mapping `target` to a `ProcessKind`. Intentionally cheap: the
/// `on_event` hook never blocks because the hub push is non-blocking.
pub struct ConsoleTeeLayer;

impl ConsoleTeeLayer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConsoleTeeLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for ConsoleTeeLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let target = metadata.target();
        let Some(kind) = classify_target(target) else {
            return;
        };
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = if visitor.message.is_empty() && visitor.fields.is_empty() {
            return;
        } else if visitor.message.is_empty() {
            format!("[{target}] {}", visitor.fields)
        } else if visitor.fields.is_empty() {
            format!("[{target}] {}", visitor.message)
        } else {
            format!("[{target}] {} ({})", visitor.message, visitor.fields)
        };
        let hub = crate::process_console::global();
        push_to_hub(&hub, kind, &message);
    }
}

fn push_to_hub(hub: &ProcessConsoleHub, kind: ProcessKind, message: &str) {
    hub.push(ProcessLine::new(
        kind,
        0, // tee events share spawn_id 0 — the frontend renders them
        //    as plain lines (no header) because the prefix is `[target]`
        //    rather than `$ ` / `[stage] ` / `→ `.
        ProcessStream::Stdout,
        message.to_string(),
    ));
}

/// Map a tracing `target` to a `ProcessKind`. Returns `None` when the
/// target is not a domain we want to surface in the Console window
/// (e.g. internal startup chatter or the spawn-summary stream that is
/// already pushed directly to the hub).
pub(crate) fn classify_target(target: &str) -> Option<ProcessKind> {
    // Self-loop guard: the direct-push paths emit summary / line targets
    // that are already in the hub. Re-pushing them would duplicate.
    if target == "gwt.process.summary" || target.starts_with("gwt.process.line") {
        return None;
    }
    if has_prefix(target, &["gwt_git", "gwt::git"]) {
        return Some(ProcessKind::Git);
    }
    if has_prefix(target, &["gwt_github", "gwt::pr", "gwt::github"]) {
        return Some(ProcessKind::Gh);
    }
    if has_prefix(target, &["gwt_docker", "gwt::docker"]) {
        return Some(ProcessKind::Docker);
    }
    if has_prefix(
        target,
        &[
            "gwt::launch",
            "gwt::wizard",
            "gwt::agent",
            "gwt_agent",
            "gwt::startup::agent",
        ],
    ) {
        return Some(ProcessKind::AgentBootstrap);
    }
    if has_prefix(
        target,
        &["gwt::index", "gwt_core::index", "gwt-core::index"],
    ) {
        return Some(ProcessKind::IndexRunner);
    }
    None
}

fn has_prefix(target: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| {
        target == *prefix
            || target.starts_with(&format!("{prefix}::"))
            || target.starts_with(&format!("{prefix}_"))
            || target.starts_with(&format!("{prefix}/"))
            || target.starts_with(&format!("{prefix}."))
    })
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            append_field(&mut self.fields, field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let mut buf = String::new();
            let _ = write!(buf, "{value:?}");
            append_field(&mut self.fields, field.name(), &buf);
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        append_field(&mut self.fields, field.name(), &value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        append_field(&mut self.fields, field.name(), &value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        append_field(&mut self.fields, field.name(), &value.to_string());
    }
}

fn append_field(buf: &mut String, name: &str, value: &str) {
    if !buf.is_empty() {
        buf.push(' ');
    }
    let _ = write!(buf, "{name}={value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_git_targets() {
        assert_eq!(classify_target("gwt_git"), Some(ProcessKind::Git));
        assert_eq!(classify_target("gwt_git::branch"), Some(ProcessKind::Git));
        assert_eq!(
            classify_target("gwt::git::worktree"),
            Some(ProcessKind::Git)
        );
    }

    #[test]
    fn classify_gh_targets() {
        assert_eq!(classify_target("gwt_github"), Some(ProcessKind::Gh));
        assert_eq!(classify_target("gwt::pr"), Some(ProcessKind::Gh));
        assert_eq!(classify_target("gwt::pr::checks"), Some(ProcessKind::Gh));
    }

    #[test]
    fn classify_docker_targets() {
        assert_eq!(classify_target("gwt_docker"), Some(ProcessKind::Docker));
        assert_eq!(
            classify_target("gwt_docker::container"),
            Some(ProcessKind::Docker)
        );
    }

    #[test]
    fn classify_agent_targets() {
        assert_eq!(
            classify_target("gwt::launch::preflight"),
            Some(ProcessKind::AgentBootstrap)
        );
        assert_eq!(
            classify_target("gwt::launch::probe"),
            Some(ProcessKind::AgentBootstrap)
        );
        assert_eq!(
            classify_target("gwt::agent_launch"),
            Some(ProcessKind::AgentBootstrap)
        );
        assert_eq!(
            classify_target("gwt::wizard::resume"),
            Some(ProcessKind::AgentBootstrap)
        );
    }

    #[test]
    fn classify_index_targets() {
        assert_eq!(
            classify_target("gwt::index"),
            Some(ProcessKind::IndexRunner)
        );
        assert_eq!(
            classify_target("gwt::index::watcher"),
            Some(ProcessKind::IndexRunner)
        );
        assert_eq!(
            classify_target("gwt_core::index::runtime"),
            Some(ProcessKind::IndexRunner)
        );
    }

    #[test]
    fn ignore_process_summary_to_prevent_double_push() {
        assert_eq!(classify_target("gwt.process.summary"), None);
        assert_eq!(classify_target("gwt.process.line"), None);
        assert_eq!(classify_target("gwt.process.line.gh"), None);
    }

    #[test]
    fn ignore_unrelated_targets() {
        assert_eq!(classify_target("gwt::startup"), None);
        assert_eq!(classify_target("gwt::open_server_url"), None);
        assert_eq!(classify_target("gwt_input_trace"), None);
        assert_eq!(classify_target("gwt_access"), None);
        assert_eq!(classify_target("hyper"), None);
        assert_eq!(classify_target(""), None);
    }
}
