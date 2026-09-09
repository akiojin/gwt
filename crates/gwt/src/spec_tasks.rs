//! Completion accounting for a gwt-spec Issue's `tasks` section (Issue #4146).
//!
//! The Issue Monitor's readiness and the SPEC audit both need one answer to
//! "how much of this `tasks` section is actually done?". Counting only
//! Markdown checkbox rows made checkbox-less task rows invisible to the total:
//! #3700 carried 10 `[x]` rows plus 51 plain `- ` task rows, read as
//! "10 done / 0 open", reported `ReadyWithCompletedTasks`, and was closed as
//! complete while 84% of the work had never been started. A row that carries
//! no checkbox is a task nobody is tracking, so it counts as open here.

/// Completed / open accounting for one `tasks` section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TasksProgress {
    /// Rows proven complete: a top-level list item whose marker is `[x]`.
    pub completed: usize,
    /// Rows not proven complete — `[ ]`, an unrecognised `[?]`-style marker,
    /// any nested checkbox row, and every untracked row.
    pub open: usize,
    /// Subset of [`Self::open`]: task rows carrying no checkbox at all. This
    /// is the shape that made #3700 look finished, so the audit reports it
    /// separately from ordinary open work.
    pub untracked: usize,
}

impl TasksProgress {
    /// Total task rows the section declares.
    pub fn total(&self) -> usize {
        self.completed + self.open
    }
}

/// Count the task rows in a `tasks` section.
///
/// Recognised as a task row:
/// - a top-level list item (bullet or ordered) whose content starts with a
///   checkbox marker — `[x]` / `[X]` counts complete, everything else in
///   brackets counts open, because an unrecognised marker must never turn a
///   partially parsed list into Issue-wide completion;
/// - a top-level *bullet* row (`-`, `*`, `+`) with no checkbox, counted open
///   and untracked. Plain *ordered* rows are left out on purpose: numbered
///   lists inside a `tasks` section are prose enumerations (open questions,
///   notes to the requester), not the task list itself;
/// - a deeply indented (>3 spaces, or any tab) checkbox row, always counted
///   open — at that depth the row may be a sub-step or indented code, so its
///   own state cannot be trusted.
///
/// Fenced code blocks are skipped so a Markdown example inside the section
/// never contributes rows.
pub fn parse_tasks_progress(tasks: &str) -> TasksProgress {
    let mut progress = TasksProgress::default();
    let mut open_fence: Option<(u8, usize)> = None;
    for line in tasks.lines() {
        let content = line.trim_start_matches([' ', '\t']);
        let indentation = &line[..line.len() - content.len()];
        if indentation.len() > 3 || indentation.contains('\t') {
            if markdown_list_item(content).is_some_and(|item| item.starts_with('[')) {
                progress.open += 1;
            }
            continue;
        }
        let fence = markdown_fence(content);
        if let Some((open_marker, open_length)) = open_fence {
            if fence.is_some_and(|(marker, length, suffix)| {
                marker == open_marker && length >= open_length && suffix.trim().is_empty()
            }) {
                open_fence = None;
            }
            continue;
        }
        if let Some((marker, length, _)) = fence {
            open_fence = Some((marker, length));
            continue;
        }
        let Some(item) = markdown_list_item(content) else {
            continue;
        };
        if item.starts_with("[x]") || item.starts_with("[X]") {
            progress.completed += 1;
        } else if item.starts_with('[') {
            progress.open += 1;
        } else if content.starts_with(['-', '*', '+']) {
            progress.open += 1;
            progress.untracked += 1;
        }
    }
    progress
}

fn markdown_fence(line: &str) -> Option<(u8, usize, &str)> {
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = line
        .bytes()
        .take_while(|candidate| *candidate == marker)
        .count();
    (length >= 3).then_some((marker, length, &line[length..]))
}

fn markdown_list_item(line: &str) -> Option<&str> {
    if let Some(item) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return Some(item.trim_start());
    }
    let (marker, item) = line.split_once(char::is_whitespace)?;
    let ordered = marker
        .strip_suffix('.')
        .or_else(|| marker.strip_suffix(')'))?;
    (!ordered.is_empty() && ordered.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(item.trim_start())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #4146 AC-1: a bullet row without a checkbox is open work, and it
    /// is reported as untracked so the audit can name the affected SPECs.
    #[test]
    fn checkbox_less_bullet_rows_count_as_open_and_untracked() {
        let progress = parse_tasks_progress("- [x] T-001\n- T-002 never tracked\n");
        assert_eq!(
            progress,
            TasksProgress {
                completed: 1,
                open: 1,
                untracked: 1,
            }
        );
        assert_eq!(progress.total(), 2);
    }

    /// A numbered row without a checkbox is a prose enumeration (#3700 ends
    /// with three numbered notes to the requester), not an untracked task.
    #[test]
    fn plain_numbered_rows_are_prose_not_tasks() {
        let progress = parse_tasks_progress("- [x] T-001\n\n1. Open question for the requester\n");
        assert_eq!(
            progress,
            TasksProgress {
                completed: 1,
                open: 0,
                untracked: 0,
            }
        );
    }
}
