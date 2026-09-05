//! Process console — ephemeral hub for external process stdout / stderr.
//!
//! See SPEC-1924 Update 2026-05-20 (Process Console Domain) and SPEC-2019
//! Update 2026-05-20 (Process Console Facet) for the motivating discussion.
//!
//! ## Architecture
//!
//! Three pieces collaborate:
//!
//! 1. [`ProcessKind`] enum — closed set of process categories that gwt
//!    spawns: gh / git / docker / agent bootstrap / Python index runner.
//! 2. [`ProcessConsoleHub`] — ring-buffer + broadcast surface that the
//!    Logs window subscribes to via WebSocket. Owned by `LoggingHandles`.
//! 3. [`spawn_logged`] — single entry point that callers use to launch
//!    external processes. Emits `gwt.process.summary` tracing events to
//!    the canonical log file and forwards stdout / stderr lines (after
//!    redaction) to the hub.
//!
//! Line-level events never reach the canonical log file. They live only
//! in the hub's ring buffer (capacity 5000 lines / kind by default) and
//! the broadcast channel. The summary events (start / end / exit_code /
//! duration / line counts) are persisted to the canonical file via the
//! standard tracing pipeline.

pub mod gh_guard;
pub mod hub;
pub mod kind;
pub mod line;
pub mod redact;
pub mod runner_probe_guard;
pub mod spawn;
pub mod strip_ansi;

pub use gh_guard::{
    forbid_unsandboxed_gh_spawns_for_tests, unsandboxed_gh_denial, REAL_GH_BLOCKED_ERROR_CODE,
};
pub use runner_probe_guard::{
    forbid_real_package_runner_probes_for_tests, real_package_runner_probe_denial,
    real_package_runner_probes_forbidden, ALLOW_REAL_RUNNER_PROBE_MARKER,
    REAL_RUNNER_PROBE_BLOCKED_ERROR_CODE, RUNNER_PROBE_SANDBOX_MARKER,
};
pub use hub::{global, set_global, ProcessConsoleHub, DEFAULT_RING_CAPACITY};
pub use kind::{ParseProcessKindError, ProcessKind};
pub use line::{ProcessLine, ProcessStream};
pub use redact::{redact_line, REDACTED};
pub use spawn::{
    spawn_logged, spawn_logged_blocking, spawn_logged_blocking_with_deadline,
    spawn_logged_with_deadline, SpawnOptions, SpawnOutput,
};
pub use strip_ansi::strip_ansi;
