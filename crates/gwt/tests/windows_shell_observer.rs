//! Shared input-readiness observation for the deterministic Windows Shell test.

pub(crate) fn input_ready(screen: &vt100::Screen) -> bool {
    let (row, _) = screen.cursor_position();
    let (_, columns) = screen.size();
    screen
        .rows(0, columns)
        .nth(usize::from(row))
        .is_some_and(|line| line.trim_end().ends_with('>'))
}
