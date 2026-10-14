//! Tests for the Intake Inspection Snapshot, the Finding Disposition Ledger,
//! and the completion evidence gate (Issue #4541 AC-3 / AC-4 / AC-6).

#![cfg(test)]

use chrono::TimeZone;

use crate::cli::spec_artifact_lint::{lint, ArtifactInput, LintCode, SectionInput};

use super::*;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap()
}

fn dirty_report() -> LintReport {
    lint(
        &ArtifactInput {
            owner_number: 4541,
            sections: vec![SectionInput {
                name: "spec".to_string(),
                content: "- **FR-001**: one\n- **FR-001**: one again\n- **FR-004**: four\n"
                    .to_string(),
            }],
            marker_header: None,
            roundtrip: Vec::new(),
        },
        now(),
    )
}

fn clean_report() -> LintReport {
    lint(
        &ArtifactInput {
            owner_number: 4541,
            sections: vec![SectionInput {
                name: "spec".to_string(),
                content: "- **FR-001**: one\n- **FR-002**: two\n".to_string(),
            }],
            marker_header: None,
            roundtrip: Vec::new(),
        },
        now(),
    )
}

fn hashes() -> BTreeMap<String, String> {
    BTreeMap::from([("spec".to_string(), "sha-spec".to_string())])
}

// --- AC-3: lint is a pre-gate, recorded in the snapshot ----------------

#[test]
fn a_snapshot_records_the_lint_run_that_preceded_it() {
    let report = dirty_report();
    let snapshot = IntakeInspectionSnapshot::capture(4541, hashes(), &report, now());
    assert_eq!(snapshot.lint.ran_at, report.ran_at);
    assert_eq!(snapshot.lint.finding_count, report.findings.len());
    assert_eq!(snapshot.lint.critical_count, report.critical_count());
    assert_eq!(snapshot.lint.sections_scanned, vec!["spec".to_string()]);
    assert_eq!(snapshot.lint.findings, report.findings);
}

#[test]
fn the_lint_run_is_never_older_than_the_artifact_it_describes() {
    let report = clean_report();
    let captured_at = now();
    let snapshot = IntakeInspectionSnapshot::capture(4541, hashes(), &report, captured_at);
    assert!(
        snapshot.lint.ran_at <= snapshot.captured_at,
        "lint must run before the snapshot is finalized"
    );
}

#[test]
fn a_persisted_snapshot_without_a_lint_outcome_is_rejected() {
    // AC-3: the lint result is not optional state a hand edit can drop.
    let encoded = serde_json::json!({
        "owner_number": 4541,
        "section_hashes": {"spec": "sha-spec"},
        "captured_at": now(),
    })
    .to_string();
    let decoded = serde_json::from_str::<IntakeInspectionSnapshot>(&encoded);
    assert!(
        decoded.is_err(),
        "a snapshot without `lint` must not deserialize"
    );
    assert!(
        decoded.unwrap_err().to_string().contains("lint"),
        "the error should name the missing field"
    );
}

#[test]
fn a_snapshot_round_trips_through_json() {
    let snapshot = IntakeInspectionSnapshot::capture(4541, hashes(), &dirty_report(), now())
        .with_context(
            Some("epoch-7".to_string()),
            Some("P7C".to_string()),
            vec!["comment:1".to_string()],
        );
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let decoded: IntakeInspectionSnapshot = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, snapshot);
    assert_eq!(decoded.phase_slice.as_deref(), Some("P7C"));
}

// --- AC-4: lint findings skip the reviewer -----------------------------

#[test]
fn lint_findings_enter_the_ledger_without_consuming_reviewer_budget() {
    let report = dirty_report();
    assert!(!report.findings.is_empty(), "fixture must produce findings");
    let ledger = FindingDispositionLedger::seed_from_lint(&report, now());

    assert_eq!(ledger.owner_number, 4541);
    assert_eq!(ledger.entries.len(), report.findings.len());
    assert!(
        ledger
            .entries
            .iter()
            .all(|entry| entry.source == FindingSource::MachineLint),
        "seeded entries must be attributed to the machine lint"
    );
    assert!(
        ledger
            .entries
            .iter()
            .all(|entry| !entry.consumes_reviewer_budget),
        "a machine-decidable finding must not bill the reviewer"
    );
    assert!(
        ledger.reviewer_budget_entries().is_empty(),
        "the reviewer queue must be empty after seeding lint findings"
    );
}

#[test]
fn seeded_entries_preserve_the_finding_identity() {
    let report = dirty_report();
    let ledger = FindingDispositionLedger::seed_from_lint(&report, now());
    let duplicate = report.by_code(LintCode::NumberingDuplicate)[0];
    let entry = ledger
        .entries
        .iter()
        .find(|entry| entry.id == duplicate.id)
        .expect("seeded entry keeps the lint finding id");
    assert_eq!(entry.refs, duplicate.refs);
    assert_eq!(entry.section, duplicate.section);
    assert_eq!(entry.severity, duplicate.severity);
    assert_eq!(entry.disposition, Disposition::Pending);
}

#[test]
fn an_independent_reviewer_finding_does_consume_budget() {
    let mut ledger = FindingDispositionLedger::seed_from_lint(&dirty_report(), now());
    ledger.entries.push(LedgerEntry {
        id: "R-001".to_string(),
        source: FindingSource::IndependentInspection,
        severity: LintSeverity::Critical,
        section: "spec".to_string(),
        refs: vec!["FR-004".to_string()],
        message: "owner scope is ambiguous".to_string(),
        disposition: Disposition::Pending,
        rationale: None,
        readback_refs: Vec::new(),
        consumes_reviewer_budget: true,
    });
    let billed = ledger.reviewer_budget_entries();
    assert_eq!(billed.len(), 1);
    assert_eq!(billed[0].id, "R-001");
}

// --- AC-6: completion evidence requires a GitHub readback --------------

fn github_readback(sha: &str) -> ReadbackEvidence {
    ReadbackEvidence {
        source: ReadbackSource::GitHubEntity,
        sha256: sha.to_string(),
        observed_at: now(),
        refs: vec!["comment:99".to_string()],
    }
}

fn clean_snapshot_and_ledger() -> (IntakeInspectionSnapshot, FindingDispositionLedger) {
    let report = clean_report();
    let snapshot = IntakeInspectionSnapshot::capture(4541, hashes(), &report, now());
    let ledger = FindingDispositionLedger::seed_from_lint(&report, now());
    (snapshot, ledger)
}

#[test]
fn a_matching_github_readback_completes() {
    let (snapshot, ledger) = clean_snapshot_and_ledger();
    let evidence = CompletionEvidence {
        sections: vec![SectionCompletion {
            section: "spec".to_string(),
            bypass_path: false,
            readback: Some(github_readback("sha-spec")),
        }],
    };
    assert_eq!(
        validate_completion_evidence(&snapshot, &ledger, &evidence),
        Ok(())
    );
}

#[test]
fn a_cache_only_readback_is_not_completion_evidence() {
    let (snapshot, ledger) = clean_snapshot_and_ledger();
    let evidence = CompletionEvidence {
        sections: vec![SectionCompletion {
            section: "spec".to_string(),
            bypass_path: false,
            readback: Some(ReadbackEvidence {
                source: ReadbackSource::LocalCache,
                sha256: "sha-spec".to_string(),
                observed_at: now(),
                refs: Vec::new(),
            }),
        }],
    };
    let errors = validate_completion_evidence(&snapshot, &ledger, &evidence).unwrap_err();
    assert_eq!(
        errors,
        vec![CompletionEvidenceError::CacheOnlyReadback {
            section: "spec".to_string(),
            bypass_path: false,
        }]
    );
}

#[test]
fn a_bypass_path_write_still_requires_a_github_readback() {
    let (snapshot, ledger) = clean_snapshot_and_ledger();
    let evidence = CompletionEvidence {
        sections: vec![SectionCompletion {
            section: "spec".to_string(),
            bypass_path: true,
            readback: None,
        }],
    };
    let errors = validate_completion_evidence(&snapshot, &ledger, &evidence).unwrap_err();
    assert_eq!(
        errors,
        vec![CompletionEvidenceError::MissingGitHubReadback {
            section: "spec".to_string(),
            bypass_path: true,
        }]
    );
    assert!(
        errors[0].to_string().contains("bypass path"),
        "the error must say why the readback is mandatory: {}",
        errors[0]
    );
}

#[test]
fn a_bypass_path_write_cannot_settle_on_the_cache_either() {
    let (snapshot, ledger) = clean_snapshot_and_ledger();
    let evidence = CompletionEvidence {
        sections: vec![SectionCompletion {
            section: "spec".to_string(),
            bypass_path: true,
            readback: Some(ReadbackEvidence {
                source: ReadbackSource::LocalCache,
                sha256: "sha-spec".to_string(),
                observed_at: now(),
                refs: Vec::new(),
            }),
        }],
    };
    let errors = validate_completion_evidence(&snapshot, &ledger, &evidence).unwrap_err();
    assert_eq!(
        errors,
        vec![CompletionEvidenceError::CacheOnlyReadback {
            section: "spec".to_string(),
            bypass_path: true,
        }]
    );
}

#[test]
fn a_section_with_no_evidence_at_all_blocks_completion() {
    let (snapshot, ledger) = clean_snapshot_and_ledger();
    let evidence = CompletionEvidence {
        sections: Vec::new(),
    };
    let errors = validate_completion_evidence(&snapshot, &ledger, &evidence).unwrap_err();
    assert_eq!(
        errors,
        vec![CompletionEvidenceError::MissingGitHubReadback {
            section: "spec".to_string(),
            bypass_path: false,
        }]
    );
}

#[test]
fn a_readback_of_different_content_blocks_completion() {
    let (snapshot, ledger) = clean_snapshot_and_ledger();
    let evidence = CompletionEvidence {
        sections: vec![SectionCompletion {
            section: "spec".to_string(),
            bypass_path: false,
            readback: Some(github_readback("sha-other")),
        }],
    };
    let errors = validate_completion_evidence(&snapshot, &ledger, &evidence).unwrap_err();
    assert_eq!(
        errors,
        vec![CompletionEvidenceError::ReadbackHashMismatch {
            section: "spec".to_string(),
            expected: "sha-spec".to_string(),
            observed: "sha-other".to_string(),
        }]
    );
}

#[test]
fn an_undisposed_critical_lint_finding_blocks_completion() {
    let report = dirty_report();
    let snapshot = IntakeInspectionSnapshot::capture(4541, hashes(), &report, now());
    let ledger = FindingDispositionLedger::seed_from_lint(&report, now());
    assert!(!ledger.undisposed_critical().is_empty());

    let evidence = CompletionEvidence {
        sections: vec![SectionCompletion {
            section: "spec".to_string(),
            bypass_path: false,
            readback: Some(github_readback("sha-spec")),
        }],
    };
    let errors = validate_completion_evidence(&snapshot, &ledger, &evidence).unwrap_err();
    assert!(
        errors.iter().any(|error| matches!(
            error,
            CompletionEvidenceError::UndisposedCriticalFinding { .. }
        )),
        "errors: {errors:?}"
    );
}

#[test]
fn disposing_the_critical_finding_unblocks_completion() {
    let report = dirty_report();
    let snapshot = IntakeInspectionSnapshot::capture(4541, hashes(), &report, now());
    let mut ledger = FindingDispositionLedger::seed_from_lint(&report, now());
    for entry in &mut ledger.entries {
        entry.disposition = Disposition::Accepted;
        entry.rationale = Some("renumbered".to_string());
        entry.readback_refs = vec!["comment:99".to_string()];
    }
    let evidence = CompletionEvidence {
        sections: vec![SectionCompletion {
            section: "spec".to_string(),
            bypass_path: false,
            readback: Some(github_readback("sha-spec")),
        }],
    };
    assert_eq!(
        validate_completion_evidence(&snapshot, &ledger, &evidence),
        Ok(())
    );
}
