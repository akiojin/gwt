use super::*;
use crate::test_support::ScopedGwtHome;
use serde_json::json;

fn submission(request: &str, revision: u64) -> Submission {
    serde_json::from_value(json!({
        "schema_version": 1, "request_id": request,
        "events": [{
            "episode_id": "incident-1", "event_id": "worker", "expected_revision": revision,
            "kind": "blocker", "state": {"status":"waiting", "severity":"routine", "impact":"project"},
            "summary": {"result":"Waiting for a worker", "owner":"Assigned agent", "next_action":"Retry when available"},
            "details": "Original failure remains unresolved.", "refs": ["Issue #3431"]
        }]
    })).unwrap()
}

#[test]
fn structured_reports_preserve_details_and_replay_suppress_or_report_material_changes() {
    let home = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());
    let repo = home.path().join("repo");
    let mut first = submission("first", 0);
    first.events[0].details = "詳しい調査結果。".repeat(2200);
    let receipt = post_submission(&repo, &first).unwrap();
    assert_eq!(receipt.reports.len(), 1);
    assert_eq!(receipt.decisions[0].revision, 1);
    assert_eq!(
        receipt.reports[0].structured.as_ref().unwrap().details,
        first.events[0].details
    );
    assert!(receipt.reports[0].body.lines().count() <= 6);
    assert_eq!(
        post_submission(&repo, &first).unwrap(),
        receipt,
        "lost response retry"
    );
    let mut tick = submission("next-tick", 1);
    tick.events[0].summary.result = "Reworded observation".into();
    tick.events[0].observed_at = Some(Utc::now());
    assert!(post_submission(&repo, &tick).unwrap().reports.is_empty());
    assert_eq!(load_reports(&repo).unwrap().len(), 1);
    tick.request_id = "fatal".into();
    tick.events[0].state.severity = ReportSeverity::Fatal;
    let fatal = post_submission(&repo, &tick).unwrap();
    assert_eq!(fatal.reports.len(), 1);
    assert_eq!(fatal.decisions[0].reason, PostingReason::Immediate);
    assert_eq!(
        post_submission(&repo, &first).unwrap(),
        receipt,
        "retry precedes revision check"
    );
    assert!(post_submission(&repo, &submission("stale", 1)).is_err());
    first.events[0].details.push_str("changed");
    assert!(
        post_submission(&repo, &first).is_err(),
        "same request cannot change payload"
    );
    let mut resolved = submission("resolved", 2);
    resolved.events[0].state.status = ReportStatus::Resolved;
    assert_eq!(post_submission(&repo, &resolved).unwrap().reports.len(), 1);
    let recurrence = submission("recurrence", 3);
    assert_eq!(
        post_submission(&repo, &recurrence).unwrap().reports.len(),
        1
    );
    let mut episode = submission("new-episode", 0);
    episode.events[0].episode_id = "incident-2".into();
    assert_eq!(post_submission(&repo, &episode).unwrap().reports.len(), 1);
    assert_eq!(
        post_submission(&home.path().join("other-project"), &submission("first", 0))
            .unwrap()
            .reports
            .len(),
        1
    );
    assert_eq!(load_report_journal(&repo).unwrap().receipts.len(), 6);
}

#[test]
fn structured_batch_emits_urgent_first_and_one_routine_digest() {
    let home = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());
    let repo = home.path().join("repo");
    let mut input = submission("batch", 0);
    let mut second = input.events[0].clone();
    second.event_id = "second".into();
    input.events.push(second.clone());
    second.event_id = "fatal".into();
    second.state.severity = ReportSeverity::Fatal;
    input.events.push(second);
    assert!(
        post_submission(&repo, &input).is_err(),
        "multiple routine events require a short authored digest"
    );
    input.digest = Some(ReportSummary {
        result: "Two workers waiting".into(),
        ..Default::default()
    });
    let receipt = post_submission(&repo, &input).unwrap();
    assert_eq!(receipt.reports.len(), 2);
    assert_eq!(receipt.decisions[2].reason, PostingReason::Immediate);
    assert_eq!(receipt.reports[1].body, "Two workers waiting");
    assert_eq!(
        receipt.reports[1]
            .structured
            .as_ref()
            .unwrap()
            .details
            .matches("Original failure")
            .count(),
        2
    );
    assert_eq!(load_reports(&repo).unwrap(), receipt.reports);
}

#[test]
fn structured_journal_preserves_legacy_and_refuses_old_readers_or_torn_unknown_rows() {
    let home = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());
    let repo = home.path().join("repo");
    let legacy = post_report(&repo, PmReportKind::Progress, "Legacy detail", None).unwrap();
    let before = fs::read(reports_path(&repo)).unwrap();
    let receipt = post_submission(&repo, &submission("first", 0)).unwrap();
    let bytes = fs::read(reports_path(&repo)).unwrap();
    assert!(bytes.starts_with(&before));
    let last = std::str::from_utf8(&bytes[before.len()..]).unwrap();
    assert!(
        serde_json::from_str::<PmReport>(last).is_err(),
        "old reader must fail closed"
    );
    assert_eq!(
        load_reports(&repo).unwrap(),
        vec![legacy, receipt.reports[0].clone()]
    );
    for corrupt in ["{", "{\"journal_schema\":99}\n", "{\"journal_schema\":2}\n", "{\"id\":\"one\",\"id\":\"two\",\"kind\":\"progress\",\"body\":\"duplicate key\",\"created_at\":\"2026-11-02T00:00:00Z\"}\n"] {
        fs::write(reports_path(&repo), [&bytes, corrupt.as_bytes()].concat()).unwrap();
        assert!(load_reports(&repo).is_err());
        assert!(post_submission(&repo, &submission("second", 1)).is_err());
        assert_eq!(
            fs::read(reports_path(&repo)).unwrap(),
            [&bytes, corrupt.as_bytes()].concat()
        );
    }
    for phase in ["hash", "receipt"] {
        let mut entry: serde_json::Value = serde_json::from_str(last).unwrap();
        if phase == "hash" {
            entry["payload_sha256"] = json!("0".repeat(64));
        } else {
            let mut payload: serde_json::Value =
                serde_json::from_str(entry["payload"].as_str().unwrap()).unwrap();
            payload["receipt"]["decisions"][0]["revision"] = json!(9);
            let payload = serde_json::to_string(&payload).unwrap();
            entry["payload_sha256"] = json!(crate::pm_authorization::digest(payload.as_bytes()));
            entry["payload"] = json!(payload);
        }
        let damaged = [before.clone(), format!("{}\n", entry).into_bytes()].concat();
        fs::write(reports_path(&repo), &damaged).unwrap();
        assert!(load_reports(&repo).is_err(), "{phase}");
        assert!(
            post_submission(&repo, &submission("second", 1)).is_err(),
            "{phase}"
        );
        assert_eq!(fs::read(reports_path(&repo)).unwrap(), damaged);
    }
}

#[test]
fn structured_concurrent_retries_publish_one_card_and_revision_conflicts_do_not_append() {
    let home = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());
    let repo = home.path().join("repo");
    let barrier = std::sync::Barrier::new(2);
    let receipts = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let _thread_home = ScopedGwtHome::set(home.path());
                    barrier.wait();
                    post_submission(&repo, &submission("same-request", 0)).unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(load_report_journal(&repo).unwrap().receipts.len(), 1);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|n| {
                let mut input = submission(&format!("request-{n}"), 1);
                input.events[0].state.milestone_id = Some(format!("correction-{n}"));
                let repo = &repo;
                let barrier = &barrier;
                let home = home.path();
                scope.spawn(move || {
                    let _thread_home = ScopedGwtHome::set(home);
                    barrier.wait();
                    post_submission(repo, &input)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(load_reports(&repo).unwrap().len(), 2);
}

#[test]
fn structured_decisions_and_corrections_are_material_but_internal_actions_still_digest() {
    let home = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());
    let repo = home.path().join("repo");
    let mut input = submission("decision", 0);
    input.events[0].kind = PmReportKind::Decision;
    input.events[0].state.action = Some(ReportAction {
        id: "choose".into(),
        kind: ReportActionKind::Decide,
        deadline: None,
    });
    input.events[0].summary.decision = Some(ReportDecision {
        question: "Ship?".into(),
        recommendation: "Wait".into(),
        reason: "CI is red".into(),
        options: vec!["Wait".into(), "Investigate".into()],
    });
    let first = post_submission(&repo, &input).unwrap();
    assert_eq!(first.decisions[0].reason, PostingReason::Immediate);
    input.request_id = "same-decision".into();
    input.events[0].expected_revision = 1;
    assert!(post_submission(&repo, &input).unwrap().reports.is_empty());
    input.request_id = "changed-decision".into();
    input.events[0].summary.decision.as_mut().unwrap().question = "Investigate now?".into();
    assert_eq!(post_submission(&repo, &input).unwrap().reports.len(), 1);
    input.request_id = "correction".into();
    input.events[0].expected_revision = 2;
    input.events[0].state.milestone_id = Some("correction-to-decision".into());
    assert_eq!(post_submission(&repo, &input).unwrap().reports.len(), 1);
    let mut bad = submission("bad", 0);
    bad.events[0].summary.result.push('\n');
    assert!(post_submission(&repo, &bad).is_err());
    bad.events[0].summary.result = "Result\u{2028}Hidden line\u{2029}Extra line".into();
    assert!(
        post_submission(&repo, &bad).is_err(),
        "Unicode separators cannot bypass summary line bounds"
    );
    let mut batch = submission("routine-actions", 0);
    batch.events[0].event_id = "routine-1".into();
    batch.events[0].state.action = Some(ReportAction {
        id: "retry".into(),
        kind: ReportActionKind::Retry,
        deadline: None,
    });
    let mut other = batch.events[0].clone();
    other.event_id = "routine-2".into();
    batch.events.push(other);
    batch.digest = Some(ReportSummary {
        result: "Workers retrying".into(),
        ..Default::default()
    });
    assert_eq!(post_submission(&repo, &batch).unwrap().reports.len(), 1);
}

#[test]
fn journal_transaction_rolls_back_partial_sync_and_readback_failures() {
    let home = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());
    let repo = home.path().join("repo");
    post_submission(&repo, &submission("first", 0)).unwrap();
    let path = reports_path(&repo);
    let before = fs::read(&path).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.lock_exclusive().unwrap();
    let record = json!({"journal_schema":2,"payload":"not committed"});
    for phase in ["partial", "sync", "readback"] {
        let result = append_record_with_sync(
            &mut file,
            &path,
            &record,
            |file, bytes| match phase {
                "partial" => {
                    file.write_all(&bytes[..7])?;
                    Err(io::Error::other("partial write"))
                }
                "readback" => file.write_all(b"{}\n"),
                _ => file.write_all(bytes),
            },
            |file, path| {
                file.sync_all()?;
                if phase == "sync" {
                    return Err(io::Error::other("injected parent fsync failure"));
                }
                sync_report_parent(path)
            },
        );
        assert!(result.is_err(), "{phase}");
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut restored = Vec::new();
        file.read_to_end(&mut restored).unwrap();
        assert_eq!(restored, before, "{phase} restores original bytes");
    }
    drop(file);
    assert_eq!(load_report_journal(&repo).unwrap().receipts.len(), 1);
}
