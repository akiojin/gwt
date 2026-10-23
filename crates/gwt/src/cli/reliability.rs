//! Issue #5876: evidence for the fleet reliability observation, not a delivery gate.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use gwt_core::coordination::{BoardEntry, BoardEntryKind};
use serde::Serialize;
use serde_json::{json, Value};

pub(super) const MANUAL_RECOVERY_TOPIC: &str = "gwt-manual-recovery:";
pub(super) const MANUAL_RECOVERY_AT_TOPIC: &str = "gwt-manual-recovery-at:";

const CATEGORIES: [(&str, &str); 10] = [
    ("A", "verification lease"),
    ("B", "restart or update"),
    ("C", "execution lifecycle"),
    ("D", "workspace or session identity"),
    ("E", "CI or flake"),
    ("F", "PR or delivery gate"),
    ("G", "hook, trust or permission"),
    ("H", "GitHub or network"),
    ("I", "specification or decision"),
    ("Other", "unclassified or ambiguous"),
];

#[derive(Serialize)]
struct Day {
    date: chrono::NaiveDate,
    complete_day: bool,
    blocked_total: usize,
    categories: BTreeMap<&'static str, usize>,
    gwt_resolvable: usize,
    manual_recoveries: usize,
}

fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    (now.date_naive() - Duration::days(7))
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
}

pub(super) fn load_report(repo: &Path, now: DateTime<Utc>) -> Value {
    if crate::board_provider::routing_for(repo).provider != "local" {
        return json!({
            "available": false,
            "ac_10": "not_evaluated",
            "error": "Reliability history is available only for the local Board provider; status does not fetch remote history.",
        });
    }
    // The provider history API reads segments beyond the 500-entry projection.
    // Its updated_at cutoff is strict, so read across the inclusive boundary.
    match crate::board_provider::load_entries_since(repo, window_start(now) - Duration::seconds(1))
    {
        Ok(entries) => summarize(&entries, now),
        Err(error) => json!({
            "available": false,
            "ac_10": "not_evaluated",
            "error": error.to_string(),
        }),
    }
}

fn summarize(entries: &[BoardEntry], now: DateTime<Utc>) -> Value {
    let start = window_start(now);
    let mut days = (0..8)
        .map(|offset| Day {
            date: (start + Duration::days(offset)).date_naive(),
            complete_day: offset < 7,
            blocked_total: 0,
            categories: CATEGORIES.iter().map(|(key, _)| (*key, 0)).collect(),
            gwt_resolvable: 0,
            manual_recoveries: 0,
        })
        .collect::<Vec<_>>();
    let mut seen_entries = HashSet::new();
    let mut seen_recoveries = HashSet::new();
    let mut blocked_records = Vec::new();
    let mut manual_recovery_records = Vec::new();
    let mut ordered = entries.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    for entry in ordered {
        if entry.created_at > now || !seen_entries.insert(&entry.id) {
            continue;
        }
        if entry.kind == BoardEntryKind::Blocked && entry.created_at >= start {
            let day =
                &mut days[(entry.created_at.date_naive() - start.date_naive()).num_days() as usize];
            let category = classify(&entry.body);
            day.blocked_total += 1;
            *day.categories.get_mut(category).expect("known category") += 1;
            if matches!(category, "A" | "B" | "C" | "D" | "E" | "F" | "G") {
                day.gwt_resolvable += 1;
            }
            blocked_records.push(json!({
                "entry_id": entry.id,
                "created_at": entry.created_at,
                "owners": entry.related_owners,
                "session_id": entry.origin_session_id,
                "category": category,
            }));
        }
        if entry.kind == BoardEntryKind::Decision {
            // Use the time of the actual intervention, not a delayed post or
            // the time a successor PM retries its durable record.
            let recovered_at = entry.related_topics.iter().find_map(|topic| {
                topic
                    .strip_prefix(MANUAL_RECOVERY_AT_TOPIC)
                    .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| at.with_timezone(&Utc))
            });
            for topic in &entry.related_topics {
                if topic.starts_with(MANUAL_RECOVERY_TOPIC) && seen_recoveries.insert(topic) {
                    let Some(at) = recovered_at.filter(|at| *at >= start && *at <= now) else {
                        continue;
                    };
                    let day = &mut days[(at.date_naive() - start.date_naive()).num_days() as usize];
                    day.manual_recoveries += 1;
                    manual_recovery_records.push(json!({
                        "recovery_id": topic.trim_start_matches(MANUAL_RECOVERY_TOPIC),
                        "entry_id": entry.id,
                        "created_at": entry.created_at,
                        "recovered_at": at,
                        "owners": entry.related_owners,
                        "author": entry.author,
                        "session_id": entry.origin_session_id,
                        "body": entry.body,
                    }));
                }
            }
        }
    }
    json!({
        "available": true,
        "timezone": "UTC",
        "window_start": start,
        "window_end": now,
        "classification_version": 1,
        "category_labels": CATEGORIES.into_iter().collect::<BTreeMap<_, _>>(),
        "days": days,
        "blocked_records": blocked_records,
        "manual_recovery_records": manual_recovery_records,
        "manual_recovery_coverage": "explicit_records_only",
        "ac_10": "not_evaluated",
        "assessment_reason": "Counts describe retained Board declarations and explicitly recorded operator recoveries. Missing records, unclassified causes and zero rows do not prove a successful seven-day observation.",
    })
}

fn classify(body: &str) -> &'static str {
    let cause = body
        .lines()
        .filter_map(|line| {
            let line = line
                .trim()
                .trim_start_matches(['-', '*', '#', '>'])
                .trim_start();
            let (label, content) = line.split_once([':', '：'])?;
            let label = label.trim().trim_matches(['*', '_']).trim().to_lowercase();
            matches!(label.as_str(), "原因" | "cause" | "why" | "root cause")
                .then_some(content.trim_start_matches(['*', '_']))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    let rules: [(&str, &[&str]); 9] = [
        ("A", &["lease", "fifo", "検証枠"]),
        (
            "B",
            &[
                "restart",
                "再起動",
                "daemon owner exited",
                "holder stopped",
                "自動更新",
            ],
        ),
        (
            "C",
            &[
                "execution",
                "ecr",
                "binding",
                "generation",
                "reopen",
                "世代",
            ],
        ),
        (
            "D",
            &[
                "workspace",
                "session ledger",
                "canonical work",
                "session identity",
            ],
        ),
        ("E", &["ci", "flake", "flaky", "フレーク", "required check"]),
        (
            "F",
            &[
                "ready pr",
                "pr gate",
                "delivery gate",
                "fresh verification",
                "verification evidence",
                "配送ゲート",
            ],
        ),
        ("G", &["hook", "trust", "permission", "権限"]),
        ("H", &["github", "network", "ネットワーク"]),
        (
            "I",
            &["spec", "specification", "decision", "裁定", "承認", "仕様"],
        ),
    ];
    let mut matches = rules.iter().filter(|(_, terms)| {
        terms.iter().any(|term| {
            if term.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
                cause
                    .split(|ch: char| !ch.is_ascii_alphanumeric())
                    .any(|word| word == *term)
            } else {
                cause.contains(term)
            }
        })
    });
    match (matches.next(), matches.next()) {
        (Some((category, _)), None) => category,
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

    fn entry(id: &str, at: &str, kind: BoardEntryKind, body: &str) -> BoardEntry {
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "PM",
            kind,
            body,
            None,
            None,
            vec![],
            vec!["5876".into(), "4740".into()],
        );
        entry.id = id.into();
        entry.created_at = at.parse().unwrap();
        entry.updated_at = entry.created_at;
        entry
    }

    fn now() -> DateTime<Utc> {
        "2026-10-21T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn daily_report_counts_declarations_including_restatements_and_resolved_posts() {
        let first = entry(
            "first",
            "2026-10-14T00:00:00Z",
            BoardEntryKind::Blocked,
            "原因: verification lease held\n依頼: CI rerun",
        );
        let mut again = first.clone();
        again.id = "again".into();
        let mut resolved = entry(
            "resolved",
            "2026-10-15T01:00:00Z",
            BoardEntryKind::Decision,
            "resolved",
        );
        resolved.resolves_entry_ids = vec![first.id.clone()];
        let mut entries = vec![first.clone(), first, again, resolved];
        for index in 0..501 {
            entries.push(entry(
                &format!("status-{index}"),
                "2026-10-20T00:00:00Z",
                BoardEntryKind::Status,
                "status",
            ));
        }
        entries.push(entry(
            "today",
            "2026-10-21T00:00:00Z",
            BoardEntryKind::Blocked,
            "原因: execution binding is settled",
        ));
        entries.push(entry(
            "old",
            "2026-10-13T23:59:59Z",
            BoardEntryKind::Blocked,
            "原因: lease",
        ));
        entries.push(entry(
            "future",
            "2026-10-21T12:00:01Z",
            BoardEntryKind::Blocked,
            "原因: lease",
        ));
        let report = super::summarize(&entries, now());
        assert_eq!(report["days"].as_array().unwrap().len(), 8);
        assert_eq!(report["days"][0]["date"], "2026-10-14");
        assert_eq!(report["days"][0]["blocked_total"], 2);
        assert_eq!(report["days"][0]["categories"]["A"], 2);
        assert_eq!(report["days"][0]["gwt_resolvable"], 2);
        assert_eq!(report["days"][0]["complete_day"], true);
        assert_eq!(report["days"][7]["blocked_total"], 1);
        assert_eq!(report["days"][7]["complete_day"], false);
        assert_eq!(report["ac_10"], "not_evaluated");
    }

    #[test]
    fn classification_uses_only_the_cause_and_leaves_ambiguity_unclassified() {
        for (cause, expected) in [
            ("verification lease held", "A"),
            ("GUI restart lost holder", "B"),
            ("execution binding is settled", "C"),
            ("canonical Workspace identity missing", "D"),
            ("CI flaky required check", "E"),
            ("Ready PR gate needs fresh verification", "F"),
            ("hook trust hash mismatch", "G"),
            ("GitHub network timeout", "H"),
            ("SPEC 裁定待ち", "I"),
            ("specification decision pending", "I"),
            ("unknown", "Other"),
            ("release deferred", "Other"),
            ("verification lease and execution binding", "Other"),
        ] {
            let body = format!("事象: hook error\n原因: {cause}\n依頼: lease\n再開条件: CI green");
            assert_eq!(super::classify(&body), expected, "{cause}");
        }
        assert_eq!(
            super::classify("Symptom: lease\nCause: execution binding\nRequest: CI"),
            "C"
        );
        assert_eq!(
            super::classify("No cause line; lease mentioned in a request"),
            "Other"
        );
        assert_eq!(
            super::classify("- **Root cause:** execution binding\nRequest: lease"),
            "C"
        );
        assert_eq!(super::classify("**原因：** verification lease held"), "A");
    }

    #[test]
    fn explicit_manual_recoveries_keep_provenance_and_dedupe_across_sessions() {
        let mut recovery = entry(
            "recovery-a",
            "2026-10-20T01:00:00Z",
            BoardEntryKind::Decision,
            "停止窓を再投入した",
        );
        recovery.related_topics = vec![
            "gwt-manual-recovery:5876:incident-a".into(),
            "gwt-manual-recovery-at:2026-10-20T01:00:00Z".into(),
        ];
        recovery.origin_session_id = Some("pm-a".into());
        let mut retry = recovery.clone();
        retry.id = "recovery-b".into();
        retry.origin_session_id = Some("pm-b".into());
        let mut separate = recovery.clone();
        separate.id = "recovery-c".into();
        separate.related_topics[0] = "gwt-manual-recovery:5876:incident-b".into();
        let mut old = recovery.clone();
        old.id = "old-recovery".into();
        old.created_at = "2026-10-13T12:00:00Z".parse().unwrap();
        old.updated_at = old.created_at;
        old.related_topics = vec![
            "gwt-manual-recovery:5876:old-incident".into(),
            "gwt-manual-recovery-at:2026-10-13T12:00:00Z".into(),
        ];
        let mut delayed = old.clone();
        delayed.id = "delayed-record".into();
        delayed.created_at = now();
        delayed.updated_at = now();
        let report = super::summarize(&[recovery, retry, separate, old, delayed], now());
        assert_eq!(report["days"][6]["manual_recoveries"], 2);
        assert_eq!(
            report["manual_recovery_records"][0]["entry_id"],
            "recovery-a"
        );
        assert_eq!(report["manual_recovery_records"][0]["session_id"], "pm-a");
        assert_eq!(
            report["manual_recovery_records"][0]["body"],
            "停止窓を再投入した"
        );
        assert_eq!(report["manual_recovery_coverage"], "explicit_records_only");
        assert_eq!(report["ac_10"], "not_evaluated");
        assert_eq!(report["days"][7]["manual_recoveries"], 0);
    }

    #[test]
    fn remote_history_is_unavailable_without_fetching_it_from_status() {
        let repo = tempfile::tempdir().unwrap();
        let _provider = crate::board_provider::test_provider_override::force(
            gwt_config::BoardProviderKind::Slack,
        );
        let report = super::load_report(repo.path(), now());
        assert_eq!(report["available"], false);
        assert_eq!(report["ac_10"], "not_evaluated");
        assert!(report["error"]
            .as_str()
            .unwrap()
            .contains("does not fetch remote history"));
    }
}
