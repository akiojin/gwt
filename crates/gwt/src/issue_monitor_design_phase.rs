//! SPEC #3836 — design phase for the Issue Monitor.
//!
//! Pure decisions only: which phase an Issue launches in, whether a `fix` is
//! large enough to need design first, whether a design phase produced enough
//! to hand over to implementation, and the acceptance-criteria normalization
//! a design phase applies. No agent is invoked and nothing is read from disk,
//! so every rule is fixed by unit tests (FR-002) rather than left to the
//! launched agent's judgment.

use crate::issue_monitor_gate::{heading_text, split_explicit_id, ACCEPTANCE_HEADINGS};

/// The phase an Issue launches in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPhase {
    /// Fill spec / plan / tasks without touching production code.
    Design,
    /// The existing implementation launch.
    Implement,
}

/// What the Monitor knows about an Issue when it chooses the phase.
#[derive(Debug, Clone, Copy, Default)]
pub struct LaunchPhaseInput<'a> {
    pub labels: &'a [String],
    pub title: &'a str,
    pub body: &'a str,
    /// Section contents; `None` when the section does not exist.
    pub spec: Option<&'a str>,
    pub plan: Option<&'a str>,
    pub tasks: Option<&'a str>,
}

/// Why a design phase may not hand over to implementation yet (FR-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignGateFailure {
    /// The named section is missing or blank.
    EmptySection(&'static str),
    /// The `spec` section has no `- [ ] AC-N:` criterion.
    NoExplicitCriteria,
}

impl DesignGateFailure {
    /// Operator-facing reason naming what the design phase still owes.
    pub fn reason(&self) -> String {
        match self {
            Self::EmptySection(name) => {
                format!("design phase incomplete: `{name}` section is missing or empty")
            }
            Self::NoExplicitCriteria => "design phase incomplete: the `spec` section has no \
                 `- [ ] AC-1: ...` criterion under an acceptance criteria heading"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkKind {
    Spec,
    Feature,
    Fix,
    Other,
}

fn work_kind(labels: &[String], title: &str) -> WorkKind {
    let has = |names: &[&str]| {
        labels
            .iter()
            .any(|label| names.contains(&label.trim().to_ascii_lowercase().as_str()))
    };
    if has(&["gwt-spec"]) {
        return WorkKind::Spec;
    }
    if has(&["enhancement", "feature", "feat"]) {
        return WorkKind::Feature;
    }
    if has(&["bug", "fix"]) {
        return WorkKind::Fix;
    }
    let title = title.trim().to_ascii_lowercase();
    let prefix = title
        .split([':', '(', '!'])
        .next()
        .unwrap_or_default()
        .trim();
    match prefix {
        "feat" => WorkKind::Feature,
        "fix" => WorkKind::Fix,
        _ => WorkKind::Other,
    }
}

fn is_typo_fix(labels: &[String], title: &str) -> bool {
    title.to_ascii_lowercase().contains("typo")
        || labels
            .iter()
            .any(|label| label.trim().eq_ignore_ascii_case("typo"))
}

/// Choose the launch phase (FR-001 / FR-002). An Issue whose design is
/// complete always implements; otherwise a SPEC, a feature, or a large fix
/// designs first, and a typo or small fix implements directly (AC-3).
pub fn decide_launch_phase(input: &LaunchPhaseInput<'_>) -> LaunchPhase {
    if evaluate_design_phase_gate(input.spec, input.plan, input.tasks).is_ok() {
        return LaunchPhase::Implement;
    }
    let kind = work_kind(input.labels, input.title);
    if kind == WorkKind::Spec {
        return LaunchPhase::Design;
    }
    if is_typo_fix(input.labels, input.title) {
        return LaunchPhase::Implement;
    }
    match kind {
        WorkKind::Feature => LaunchPhase::Design,
        WorkKind::Fix => {
            let text = match input.spec {
                Some(spec) => format!("{}\n\n{spec}", input.body),
                None => input.body.to_string(),
            };
            if is_large_fix(&text) {
                LaunchPhase::Design
            } else {
                LaunchPhase::Implement
            }
        }
        WorkKind::Spec | WorkKind::Other => LaunchPhase::Implement,
    }
}

/// A fix needs design first when its text (1) lays out two or more design
/// options, (2) carries four or more acceptance criteria, or (3) refers to an
/// existing SPEC. A typo or one-line fix meets none of these.
pub fn is_large_fix(text: &str) -> bool {
    let options = text.lines().filter(|line| is_design_option(line)).count();
    options >= 2
        || crate::issue_monitor_gate::classify_acceptance_criteria(text)
            .ids
            .len()
            >= 4
        || refers_to_spec(text)
}

/// `案 A` / `案1` / `Option B` / `Approach 2` / `Alternative C` at the start
/// of a heading, bullet, or paragraph line.
fn is_design_option(line: &str) -> bool {
    let line = line
        .trim_start_matches(|c: char| c == '#' || c == '-' || c == '*' || c.is_whitespace())
        .to_ascii_lowercase();
    let rest = ["案", "option ", "approach ", "alternative "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker));
    rest.and_then(|rest| rest.trim_start().chars().next())
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// `SPEC #3200`, `SPEC-12`, `SPEC 7` — not `SPECIFICATION 2`.
fn refers_to_spec(text: &str) -> bool {
    text.match_indices("SPEC").any(|(at, _)| {
        let standalone = text[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let rest = text[at + "SPEC".len()..].trim_start_matches([' ', '-', '#']);
        standalone && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
    })
}

/// FR-006: a design phase is complete when spec / plan / tasks are all
/// non-empty and the `spec` section carries at least one `AC-N:` criterion
/// under an acceptance criteria heading. Anything less keeps the Issue in the
/// design phase (AC-9).
pub fn evaluate_design_phase_gate(
    spec: Option<&str>,
    plan: Option<&str>,
    tasks: Option<&str>,
) -> Result<(), DesignGateFailure> {
    let filled = |content: Option<&str>| content.is_some_and(|text| !text.trim().is_empty());
    for (name, content) in [("spec", spec), ("plan", plan), ("tasks", tasks)] {
        if !filled(content) {
            return Err(DesignGateFailure::EmptySection(name));
        }
    }
    let spec = spec.unwrap_or_default();
    let lines: Vec<&str> = spec.split('\n').collect();
    if criterion_items(&lines)
        .iter()
        .any(|item| item.explicit_id.is_some())
    {
        Ok(())
    } else {
        Err(DesignGateFailure::NoExplicitCriteria)
    }
}

/// A checklist item inside an acceptance criteria block.
struct CriterionItem {
    line: usize,
    /// Byte offset just past the `[ ]` / `[x]` checkbox.
    after_checkbox: usize,
    explicit_id: Option<String>,
}

/// Every checklist item the acceptance classifier would read, using its block
/// rules: a recognized heading opens a block, any other heading or a gwt-spec
/// artifact marker closes it.
fn criterion_items(lines: &[&str]) -> Vec<CriterionItem> {
    let mut in_block = false;
    let mut items = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some((heading, _)) = heading_text(line) {
            in_block = ACCEPTANCE_HEADINGS.contains(&heading.as_str());
            continue;
        }
        if !in_block {
            continue;
        }
        let item = line.trim_start();
        if item.starts_with("<!--") && item.contains("artifact:") {
            in_block = false;
            continue;
        }
        let Some(after_bullet) = item.strip_prefix("- ").or_else(|| item.strip_prefix("* ")) else {
            continue;
        };
        let after_bullet = after_bullet.trim_start();
        let Some(rest) = ["[ ]", "[x]", "[X]"]
            .iter()
            .find_map(|checkbox| after_bullet.strip_prefix(checkbox))
        else {
            continue;
        };
        items.push(CriterionItem {
            line: index,
            after_checkbox: line.len() - rest.len(),
            explicit_id: split_explicit_id(rest.trim_start()).0,
        });
    }
    items
}

/// FR-005: give every acceptance checklist item without an `AC-N:` prefix the
/// next free number. Lines are never added, removed, or reworded (AC-7), and
/// items that already carry an id are left exactly as written (AC-8). Without
/// any existing id the numbers match the classifier's positional ids, so a
/// launch snapshot taken before normalization still matches afterwards.
pub fn normalize_acceptance_criteria(text: &str) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let borrowed: Vec<&str> = text.split('\n').collect();
    let items = criterion_items(&borrowed);
    let used: Vec<String> = items
        .iter()
        .filter_map(|item| item.explicit_id.clone())
        .collect();
    let mut next = used
        .iter()
        .filter_map(|id| id.strip_prefix("AC-")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    for item in items.iter().filter(|item| item.explicit_id.is_none()) {
        while used.contains(&format!("AC-{next}")) {
            next += 1;
        }
        let line = borrowed[item.line];
        let (head, rest) = line.split_at(item.after_checkbox);
        lines[item.line] = format!("{head} AC-{next}: {}", rest.trim_start());
        next += 1;
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESIGNED_SPEC: &str = "## 背景\n\nx\n\n## 受け入れ基準\n\n- [ ] AC-1: works\n";

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn phase(labels: &[String], title: &str, body: &str) -> LaunchPhase {
        decide_launch_phase(&LaunchPhaseInput {
            labels,
            title,
            body,
            ..Default::default()
        })
    }

    fn checkbox_lines(text: &str) -> usize {
        text.lines()
            .filter(|line| {
                let item = line.trim_start();
                item.starts_with("- [ ]") || item.starts_with("- [x]") || item.starts_with("- [X]")
            })
            .count()
    }

    #[test]
    fn feature_without_design_launches_the_design_phase() {
        assert_eq!(
            phase(
                &labels(&["enhancement"]),
                "Add export",
                "Please add export."
            ),
            LaunchPhase::Design
        );
        assert_eq!(
            phase(&[], "feat(gui): add export", "Please add export."),
            LaunchPhase::Design
        );
    }

    #[test]
    fn spec_with_incomplete_sections_launches_the_design_phase() {
        let labels = labels(&["gwt-spec"]);
        let input = LaunchPhaseInput {
            labels: &labels,
            title: "SPEC: x",
            spec: Some(DESIGNED_SPEC),
            plan: Some("plan"),
            tasks: None,
            ..Default::default()
        };
        assert_eq!(decide_launch_phase(&input), LaunchPhase::Design);
    }

    #[test]
    fn completed_design_launches_the_implementation_phase() {
        for names in [&["gwt-spec"][..], &["enhancement"][..]] {
            let labels = labels(names);
            let input = LaunchPhaseInput {
                labels: &labels,
                title: "x",
                spec: Some(DESIGNED_SPEC),
                plan: Some("plan"),
                tasks: Some("- [ ] T-001 do"),
                ..Default::default()
            };
            assert_eq!(
                decide_launch_phase(&input),
                LaunchPhase::Implement,
                "{names:?}"
            );
        }
    }

    #[test]
    fn typo_and_one_line_fixes_skip_the_design_phase() {
        let bug = labels(&["bug"]);
        assert_eq!(
            phase(&bug, "fix: typo in README", "`recieve` → `receive`"),
            LaunchPhase::Implement
        );
        assert_eq!(
            phase(
                &bug,
                "Off-by-one in tab index",
                "Index starts at 1; it should start at 0."
            ),
            LaunchPhase::Implement
        );
        assert_eq!(
            phase(&labels(&["enhancement"]), "Fix typo in help text", "x"),
            LaunchPhase::Implement
        );
    }

    #[test]
    fn large_fix_launches_the_design_phase() {
        let bug = labels(&["bug"]);
        let body = "## 受け入れ条件\n\n- [ ] a\n- [ ] b\n- [ ] c\n- [ ] d\n";
        assert_eq!(phase(&bug, "fix: lease leak", body), LaunchPhase::Design);
    }

    #[test]
    fn other_work_launches_the_implementation_phase() {
        assert_eq!(
            phase(&labels(&["documentation"]), "docs: clarify", "x"),
            LaunchPhase::Implement
        );
        assert_eq!(phase(&[], "chore: bump deps", "x"), LaunchPhase::Implement);
    }

    #[test]
    fn large_fix_criteria_each_trigger_on_their_own() {
        let options = "### 案 A: retry\n\nx\n\n### 案 B: lock\n\ny";
        assert!(is_large_fix(options));
        assert!(is_large_fix("- Option A: retry\n- Option B: lock"));
        assert!(is_large_fix(
            "## Acceptance Criteria\n\n- [ ] AC-1: a\n- [ ] AC-2: b\n- [ ] AC-3: c\n- [ ] AC-4: d"
        ));
        assert!(is_large_fix("Breaks the contract in SPEC #3200."));
        assert!(is_large_fix("See SPEC-12 FR-022."));
    }

    #[test]
    fn small_fix_meets_no_large_fix_criterion() {
        assert!(!is_large_fix("`recieve` → `receive`"));
        assert!(!is_large_fix(
            "Only one 案 A here.\n\n## 受け入れ条件\n\n- [ ] a\n- [ ] b\n- [ ] c"
        ));
        assert!(!is_large_fix("The SPECIFICATION 2 wording is wrong."));
    }

    #[test]
    fn design_gate_requires_every_section_and_an_explicit_criterion() {
        assert_eq!(
            evaluate_design_phase_gate(Some(DESIGNED_SPEC), Some("plan"), Some("tasks")),
            Ok(())
        );
        assert_eq!(
            evaluate_design_phase_gate(None, Some("plan"), Some("tasks")),
            Err(DesignGateFailure::EmptySection("spec"))
        );
        assert_eq!(
            evaluate_design_phase_gate(Some(DESIGNED_SPEC), Some("  \n"), Some("tasks")),
            Err(DesignGateFailure::EmptySection("plan"))
        );
        assert_eq!(
            evaluate_design_phase_gate(Some(DESIGNED_SPEC), Some("plan"), None),
            Err(DesignGateFailure::EmptySection("tasks"))
        );
        let unnumbered = "## 受け入れ基準\n\n- [ ] works\n";
        assert_eq!(
            evaluate_design_phase_gate(Some(unnumbered), Some("plan"), Some("tasks")),
            Err(DesignGateFailure::NoExplicitCriteria)
        );
        let checklist_elsewhere = "## Tasks\n\n- [ ] AC-1: misplaced\n";
        assert_eq!(
            evaluate_design_phase_gate(Some(checklist_elsewhere), Some("plan"), Some("tasks")),
            Err(DesignGateFailure::NoExplicitCriteria)
        );
        assert!(DesignGateFailure::NoExplicitCriteria
            .reason()
            .contains("AC-1:"));
        assert!(DesignGateFailure::EmptySection("plan")
            .reason()
            .contains("plan"));
    }

    #[test]
    fn normalization_numbers_unprefixed_criteria_by_position() {
        let input = "## 受け入れ条件\n\n- [ ] first\n  - [x] second\n* [ ] third\n";
        let output = normalize_acceptance_criteria(input);
        assert_eq!(
            output,
            "## 受け入れ条件\n\n- [ ] AC-1: first\n  - [x] AC-2: second\n* [ ] AC-3: third\n"
        );
        assert_eq!(checkbox_lines(&output), checkbox_lines(input));
        // The classifier keeps the same ids, so a launch snapshot does not drift.
        let before = crate::issue_monitor_gate::classify_acceptance_criteria(input);
        let after = crate::issue_monitor_gate::classify_acceptance_criteria(&output);
        assert_eq!(before.criterion_texts, after.criterion_texts);
    }

    #[test]
    fn normalization_leaves_existing_ids_and_other_blocks_untouched() {
        let input = "## Acceptance Criteria\n\n- [ ] AC-2: kept\n- [ ] added\n- [x] AC-7b: kept too\n\n## Tasks\n\n- [ ] T-001 not a criterion\n";
        let output = normalize_acceptance_criteria(input);
        assert_eq!(
            output,
            "## Acceptance Criteria\n\n- [ ] AC-2: kept\n- [ ] AC-3: added\n- [x] AC-7b: kept too\n\n## Tasks\n\n- [ ] T-001 not a criterion\n"
        );
        assert_eq!(checkbox_lines(&output), checkbox_lines(input));
    }

    #[test]
    fn normalization_is_idempotent_and_keeps_line_count() {
        let input = "intro\n## 受入基準\n- [ ] a\n- plain bullet\n<!-- artifact:tasks BEGIN -->\n- [ ] T-1\n<!-- artifact:tasks END -->";
        let once = normalize_acceptance_criteria(input);
        assert_eq!(normalize_acceptance_criteria(&once), once);
        assert_eq!(once.lines().count(), input.lines().count());
        assert!(once.contains("- [ ] AC-1: a\n- plain bullet\n"));
        assert!(once.contains("- [ ] T-1\n"));
        assert!(!once.ends_with('\n'));
    }
}
