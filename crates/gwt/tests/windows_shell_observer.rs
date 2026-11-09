//! Shared input-readiness observation for the deterministic Windows Shell test.

pub(crate) fn input_ready(screen: &vt100::Screen) -> bool {
    screen.contents().trim_end().ends_with('>')
}
