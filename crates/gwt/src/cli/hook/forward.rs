//! `gwt hook forward` — stub.
//!
//! A follow-up SPEC will wire Claude Code hook events through to other
//! consumers (another gwt session, external IDE
//! listeners, etc). Until then this handler is a silent no-op that
//! exits 0, so settings_local.rs can reference it without breakage.

use super::{HookError, HookEvent};
use std::io::Read;

/// No-op. Returns `Ok(())` for every event.
pub fn handle() -> Result<(), HookError> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    handle_with_input(&input)
}

pub fn handle_with_input(input: &str) -> Result<(), HookError> {
    // Drain stdin if present so that the writer does not get a
    // SIGPIPE on exit. Ignore deserialization errors — this hook is
    // fail-open by design until the forwarding target is defined.
    let _ = HookEvent::read_from_str(input);
    Ok(())
}
