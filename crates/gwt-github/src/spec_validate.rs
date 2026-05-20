//! SPEC body validation for `gwt-register-spec` (SPEC-2784).
//!
//! Pure validation of a SPEC body string against the 7-section canonical
//! contract. Returns a list of issues rather than `Result<_, _>` so the
//! caller can present every problem at once instead of fix-one-then-rerun.

use serde::{Deserialize, Serialize};

/// Severity of a single validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// Cannot be processed downstream without a manual fix.
    Structural,
    /// Processable but does not meet the project's format convention.
    Format,
}

/// One validation finding. `location` is either `"title"`, the section
/// heading (`"## 機能要件"`), or a free-form line / span hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub location: String,
    pub message: String,
}

/// Validation configuration. `default_rules()` returns the canonical
/// 7-section / FR-NNN rule set used by `gwt-register-spec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationRules {
    pub title_prefix: &'static str,
    pub required_sections: &'static [&'static str],
    pub frbidden_marker: &'static str,
}

/// Canonical rules: `SPEC: ` prefix, 7 sections, no `[NEEDS CLARIFICATION]`.
pub fn default_rules() -> ValidationRules {
    ValidationRules {
        title_prefix: "SPEC: ",
        required_sections: &[
            "## 背景",
            "## ユビキタス言語",
            "## ユーザーシナリオと受け入れシナリオ",
            "## 機能要件",
            "## 成功基準",
            "## Out of Scope",
            "## Related Artifacts",
        ],
        frbidden_marker: "[NEEDS CLARIFICATION]",
    }
}

/// Validate the body against the rules. Empty `Vec` means PASS.
pub fn validate_spec_body(
    body: &str,
    title: &str,
    rules: &ValidationRules,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    if !title.starts_with(rules.title_prefix) || title.len() <= rules.title_prefix.len() {
        issues.push(ValidationIssue {
            severity: Severity::Structural,
            location: "title".into(),
            message: format!(
                "title must match `^{}.+$`, got: {}",
                rules.title_prefix, title
            ),
        });
    }

    let expected_h1 = format!("# {title}");
    let first_h1 = body
        .lines()
        .find(|line| line.starts_with("# "))
        .unwrap_or("");
    if first_h1 != expected_h1 {
        issues.push(ValidationIssue {
            severity: Severity::Structural,
            location: "h1".into(),
            message: format!("body H1 must equal `{expected_h1}`, got: `{first_h1}`"),
        });
    }

    for required in rules.required_sections {
        if !body
            .lines()
            .any(|line| line == *required || line.starts_with(&format!("{required} (")))
        {
            issues.push(ValidationIssue {
                severity: Severity::Structural,
                location: (*required).into(),
                message: format!("required section `{required}` is missing"),
            });
        }
    }

    let fr_section = extract_section(body, "## 機能要件");
    let fr_numbers = collect_fr_numbers(&fr_section);
    if fr_numbers.is_empty() {
        issues.push(ValidationIssue {
            severity: Severity::Structural,
            location: "## 機能要件".into(),
            message: "at least one `- **FR-NNN**: ...` line is required".into(),
        });
    } else {
        for (i, &n) in fr_numbers.iter().enumerate() {
            let expected = (i + 1) as u32;
            if n != expected {
                issues.push(ValidationIssue {
                    severity: Severity::Format,
                    location: "## 機能要件".into(),
                    message: format!(
                        "FR identifiers must be contiguous starting at FR-001; \
                         expected FR-{expected:03}, found FR-{n:03}"
                    ),
                });
                break;
            }
        }
    }

    if body.contains(rules.frbidden_marker) {
        issues.push(ValidationIssue {
            severity: Severity::Structural,
            location: "body".into(),
            message: format!(
                "`{}` markers must be resolved before registration",
                rules.frbidden_marker
            ),
        });
    }

    issues
}

fn extract_section(body: &str, heading_prefix: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in body.lines() {
        if inside {
            if line.starts_with("## ") {
                break;
            }
            out.push_str(line);
            out.push('\n');
        } else if line == heading_prefix || line.starts_with(&format!("{heading_prefix} (")) {
            inside = true;
        }
    }
    out
}

fn collect_fr_numbers(section: &str) -> Vec<u32> {
    let mut numbers = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim_start();
        let rest = match trimmed.strip_prefix("- **FR-") {
            Some(rest) => rest,
            None => continue,
        };
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(n) = digits.parse::<u32>() {
            numbers.push(n);
        }
    }
    numbers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_body() -> String {
        r#"# SPEC: Demo Feature

## 背景

The why.

## ユビキタス言語

- **Term**: meaning

## ユーザーシナリオと受け入れシナリオ

### Primary User Story

A story.

### Acceptance Scenarios

1. Scenario one.

## 機能要件

- **FR-001**: first
- **FR-002**: second

## 成功基準

- All tests pass.

## Out of Scope (v1)

- nothing

## Related Artifacts

- none
"#
        .to_string()
    }

    #[test]
    fn passes_a_well_formed_body() {
        let issues = validate_spec_body(&good_body(), "SPEC: Demo Feature", &default_rules());
        assert!(issues.is_empty(), "expected no issues, got: {issues:#?}");
    }

    #[test]
    fn flags_title_without_spec_prefix() {
        let issues = validate_spec_body(&good_body(), "feat: missing prefix", &default_rules());
        assert!(issues
            .iter()
            .any(|i| i.location == "title" && i.severity == Severity::Structural));
    }

    #[test]
    fn flags_h1_mismatch() {
        let issues = validate_spec_body(&good_body(), "SPEC: Different Title", &default_rules());
        assert!(issues
            .iter()
            .any(|i| i.location == "h1" && i.severity == Severity::Structural));
    }

    #[test]
    fn flags_missing_section() {
        let body = good_body().replace("## Out of Scope (v1)", "## OutOfScope (typo)");
        let issues = validate_spec_body(&body, "SPEC: Demo Feature", &default_rules());
        assert!(issues.iter().any(|i| i.location.contains("Out of Scope")));
    }

    #[test]
    fn flags_no_fr_lines() {
        let body = good_body().replace("- **FR-001**: first\n- **FR-002**: second", "- something");
        let issues = validate_spec_body(&body, "SPEC: Demo Feature", &default_rules());
        assert!(issues
            .iter()
            .any(|i| i.location == "## 機能要件" && i.severity == Severity::Structural));
    }

    #[test]
    fn flags_non_contiguous_fr_numbers() {
        let body = good_body().replace("- **FR-002**: second", "- **FR-003**: third");
        let issues = validate_spec_body(&body, "SPEC: Demo Feature", &default_rules());
        assert!(
            issues
                .iter()
                .any(|i| i.severity == Severity::Format && i.location == "## 機能要件"),
            "expected Format issue for FR gap, got: {issues:#?}"
        );
    }

    #[test]
    fn flags_needs_clarification_marker() {
        let body = good_body().replace("The why.", "The why. [NEEDS CLARIFICATION]");
        let issues = validate_spec_body(&body, "SPEC: Demo Feature", &default_rules());
        assert!(issues
            .iter()
            .any(|i| i.message.contains("NEEDS CLARIFICATION")));
    }

    #[test]
    fn empty_body_reports_multiple_issues() {
        let issues = validate_spec_body("", "SPEC: Empty", &default_rules());
        // At least missing H1 + every required section + missing FR.
        assert!(issues.len() >= 8, "expected many issues, got: {issues:#?}");
    }

    #[test]
    fn out_of_scope_without_version_suffix_passes() {
        let body = good_body().replace("## Out of Scope (v1)", "## Out of Scope");
        let issues = validate_spec_body(&body, "SPEC: Demo Feature", &default_rules());
        assert!(
            issues.is_empty(),
            "`## Out of Scope` alone should pass, got: {issues:#?}"
        );
    }
}
