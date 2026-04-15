//! `gwt hook forward` — stub.
//!
//! A follow-up SPEC will wire Claude Code hook events through to other
//! consumers (another gwt session, external IDE
//! listeners, etc). Until then this handler is a silent no-op that
//! exits 0, so settings_local.rs can reference it without breakage.

use super::{HookError, HookEvent};

/// No-op. Returns `Ok(())` for every event.
pub fn handle() -> Result<(), HookError> {
    // Drain stdin if present so that the writer does not get a
    // SIGPIPE on exit. Ignore deserialization errors — this hook is
    // fail-open by design until the forwarding target is defined.
    let _ = HookEvent::read_from_stdin();
    Ok(())
}
