//! Issue #4146 AC-3: regression guard over the real `tasks` section of #3700,
//! the SPEC that was closed as complete while 51 of its 61 task rows had never
//! been started.

use gwt::spec_tasks::{parse_tasks_progress, TasksProgress};

/// The `tasks` section of #3700 exactly as `issue.spec.section` returns it.
const SPEC_3700_TASKS: &str = include_str!("fixtures/spec_3700_tasks.md");

#[test]
fn spec_3700_tasks_report_ten_completed_and_fifty_one_open() {
    let progress = parse_tasks_progress(SPEC_3700_TASKS);

    assert_eq!(
        progress,
        TasksProgress {
            completed: 10,
            open: 51,
            untracked: 51,
        },
        "#3700 carries 10 [x] rows and 51 checkbox-less task rows"
    );
    assert_eq!(progress.total(), 61);
}
