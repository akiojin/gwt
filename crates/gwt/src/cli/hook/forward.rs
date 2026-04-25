//! `gwtd hook forward` payload parser.
//!
//! The public CLI surface stays fail-open and only drains/parses the hook
//! payload. Runtime fanout to the daemon-owned live event bridge is layered in
//! `crate::daemon_runtime`, so this module remains a no-op on malformed or
//! absent input.

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
