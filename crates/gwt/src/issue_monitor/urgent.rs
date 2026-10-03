//! Bounded urgent ordering over the existing terminal queue (SPEC #4831).
//! The stored queue remains the normal order; a demotion never has to rebuild it.

use super::*;

fn default_urgent_limit() -> usize {
    // Two urgent items can make progress without displacing the whole backlog.
    // This bounds priority only: launch admission still owns max_active.
    2
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorUrgentGrant {
    pub label_present: bool,
    pub observed_at: String,
    pub issue_updated_at: Option<String>,
    /// Issue revision whose event history established the retained assignment.
    #[serde(default)]
    pub assignment_revision: Option<String>,
    pub assigned_by: Option<String>,
    pub assigned_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorUrgentQueue {
    #[serde(default = "default_urgent_limit")]
    pub limit: usize,
    #[serde(default)]
    pub grants: BTreeMap<u64, IssueMonitorUrgentGrant>,
    #[serde(default)]
    pub demoted: BTreeSet<u64>,
}

impl Default for IssueMonitorUrgentQueue {
    fn default() -> Self {
        Self {
            limit: default_urgent_limit(),
            grants: BTreeMap::new(),
            demoted: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueMonitorUrgentQueueProjection {
    pub entries: Vec<IssueMonitorTerminalQueueEntry>,
    pub last_seen_at: Option<String>,
    pub urgent_limit: usize,
    pub urgent_count: usize,
    pub urgent_overflow: usize,
}

impl IssueMonitorUrgentGrant {
    fn newer_than(&self, other: &Self) -> bool {
        match (
            self.issue_updated_at.as_deref().and_then(parse_rfc3339_utc),
            other
                .issue_updated_at
                .as_deref()
                .and_then(parse_rfc3339_utc),
        ) {
            (Some(left), Some(right)) if left != right => left > right,
            _ => self.observed_at >= other.observed_at,
        }
    }
}

impl IssueMonitorUrgentQueue {
    pub fn projection(
        &self,
        queue: &IssueMonitorTerminalQueue,
    ) -> IssueMonitorUrgentQueueProjection {
        let mut urgent = queue
            .entries
            .iter()
            .filter(|entry| {
                !self.demoted.contains(&entry.number)
                    && self
                        .grants
                        .get(&entry.number)
                        .is_some_and(|grant| grant.label_present)
            })
            .collect::<Vec<_>>();
        urgent.sort_by_key(|entry| {
            let grant = &self.grants[&entry.number];
            parse_rfc3339_utc(grant.assigned_at.as_deref().unwrap_or(&grant.observed_at))
        });
        let urgent_overflow = urgent.len().saturating_sub(self.limit);
        let head = urgent
            .into_iter()
            .take(self.limit)
            .map(|entry| entry.number)
            .collect::<Vec<_>>();
        let mut entries = head
            .iter()
            .filter_map(|number| queue.entries.iter().find(|entry| entry.number == *number))
            .chain(
                queue
                    .entries
                    .iter()
                    .filter(|entry| !head.contains(&entry.number)),
            )
            .cloned()
            .collect::<Vec<_>>();
        for entry in &mut entries {
            let grant = self
                .grants
                .get(&entry.number)
                .filter(|grant| grant.label_present);
            let is_urgent = head.contains(&entry.number);
            entry.priority = Some(if is_urgent { "urgent" } else { "normal" }.to_string());
            entry.priority_reason = Some(
                if self.demoted.contains(&entry.number) {
                    "pm_demoted"
                } else if is_urgent {
                    "urgent_label"
                } else if grant.is_some() {
                    "urgent_limit_reached"
                } else {
                    "normal_order"
                }
                .to_string(),
            );
            entry.assigned_by = grant.and_then(|grant| grant.assigned_by.clone());
            entry.assigned_at = grant.and_then(|grant| grant.assigned_at.clone());
        }
        IssueMonitorUrgentQueueProjection {
            entries,
            last_seen_at: queue.last_seen_at.clone(),
            urgent_limit: self.limit,
            urgent_count: head.len(),
            urgent_overflow,
        }
    }

    /// A newer label observation wins, while operator policy always comes from disk.
    pub(super) fn rebase(&mut self, disk: &Self) {
        self.limit = disk.limit;
        self.demoted = disk.demoted.clone();
        for (number, grant) in &disk.grants {
            match self.grants.get_mut(number) {
                Some(local) if local.newer_than(grant) => {
                    // A failed event read must not erase provenance another
                    // scan already established for this exact Issue revision.
                    if local.issue_updated_at.is_some()
                        && local.issue_updated_at == grant.issue_updated_at
                        && local.label_present == grant.label_present
                        && grant.assigned_at.is_some()
                        && (local.assigned_at.is_none()
                            || (local.assignment_revision != local.issue_updated_at
                                && grant.assignment_revision == grant.issue_updated_at))
                    {
                        local.assigned_by = grant.assigned_by.clone();
                        local.assigned_at = grant.assigned_at.clone();
                        local.assignment_revision = grant.assignment_revision.clone();
                    }
                }
                _ => {
                    self.grants.insert(*number, grant.clone());
                }
            }
        }
    }
}

impl IssueMonitorState {
    pub fn urgent_queue_projection(&self, terminal: &str) -> IssueMonitorUrgentQueueProjection {
        self.urgent_queue.projection(
            self.terminal_queues
                .get(terminal)
                .unwrap_or(&IssueMonitorTerminalQueue::default()),
        )
    }

    pub(crate) fn observe_urgent_issue(&mut self, issue: &IssueMonitorIssue, now: &str) {
        let present = issue.state == IssueMonitorIssueState::Open
            && issue
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case("urgent"));
        if !present && !self.urgent_queue.grants.contains_key(&issue.number) {
            return;
        }
        let mut incoming = IssueMonitorUrgentGrant {
            label_present: present,
            observed_at: now.to_string(),
            issue_updated_at: issue.updated_at.clone(),
            assignment_revision: None,
            assigned_by: None,
            assigned_at: None,
        };
        if let Some(current) = self.urgent_queue.grants.get(&issue.number) {
            if !incoming.newer_than(current) {
                return;
            }
            if current.label_present == present && current.issue_updated_at == issue.updated_at {
                return;
            }
            // Preserve FIFO until the GitHub event readback can identify a re-grant.
            if current.label_present && present {
                incoming.observed_at = current.observed_at.clone();
                incoming.assigned_by = current.assigned_by.clone();
                incoming.assigned_at = current.assigned_at.clone();
                incoming.assignment_revision = current.assignment_revision.clone();
            }
        }
        self.urgent_queue.grants.insert(issue.number, incoming);
    }

    pub(crate) fn record_urgent_assignment(
        &mut self,
        number: u64,
        expected_revision: Option<&str>,
        assignment: &gwt_github::client::LabelAssignment,
    ) {
        let Some(expected_revision) = expected_revision else {
            return;
        };
        if let Some(grant) = self.urgent_queue.grants.get_mut(&number).filter(|grant| {
            grant.label_present && grant.issue_updated_at.as_deref() == Some(expected_revision)
        }) {
            grant.assigned_by = assignment.actor.clone();
            grant.assigned_at = Some(assignment.created_at.clone());
            grant.assignment_revision = Some(expected_revision.to_string());
            self.apply_priority_order_to_queue();
        }
    }

    pub(super) fn admit_urgent_candidates(&mut self, issues: &[IssueMonitorIssue], now: &str) {
        for issue in issues {
            self.observe_urgent_issue(issue, now);
            if issue_monitor_candidate_exclusion(issue).is_none() {
                self.admit_observed_urgent(issue.number, now);
            }
        }
    }

    pub(super) fn admit_observed_urgent(&mut self, number: u64, now: &str) {
        let host = crate::process::current_hostname();
        if !self
            .urgent_queue
            .grants
            .get(&number)
            .is_some_and(|grant| grant.label_present)
            || self.issue_is_closed(number)
            || self.merged_issues.contains(&number)
            || self
                .terminal_queue_exclusions
                .get(&host)
                .is_some_and(|excluded| excluded.contains(&number))
        {
            return;
        }
        let queue = self.terminal_queues.entry(host).or_default();
        if !queue.entries.iter().any(|entry| entry.number == number) {
            queue.entries.push(IssueMonitorTerminalQueueEntry {
                number,
                queued_at: now.to_string(),
                queued_by: "urgent".to_string(),
                ..Default::default()
            });
            queue.last_seen_at = Some(now.to_string());
        }
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;

    fn grant(revision: &str, actor: Option<&str>) -> IssueMonitorUrgentGrant {
        IssueMonitorUrgentGrant {
            label_present: true,
            observed_at: "2026-10-03T02:00:00Z".into(),
            issue_updated_at: Some(revision.into()),
            assignment_revision: actor.map(|_| revision.to_string()),
            assigned_by: actor.map(str::to_string),
            assigned_at: actor.map(|_| "2026-10-03T00:00:00Z".into()),
        }
    }

    #[test]
    fn urgent_assignment_from_stale_scan_cannot_overwrite_newer_revision() {
        let current = grant("2026-10-03T01:00:00Z", Some("new-actor"));
        let mut monitor = IssueMonitorState::new(Default::default());
        monitor.urgent_queue.grants.insert(7, current.clone());
        let stale = gwt_github::client::LabelAssignment {
            actor: Some("old-actor".into()),
            created_at: "2026-10-02T00:00:00Z".into(),
        };
        monitor.record_urgent_assignment(7, Some("2026-10-03T00:00:00Z"), &stale);
        assert_eq!(monitor.urgent_queue.grants.get(&7), Some(&current));
    }

    #[test]
    fn urgent_assignment_rebase_keeps_known_disk_audit_for_the_same_revision() {
        let known = grant("2026-10-03T01:00:00Z", Some("actor"));
        let mut disk = IssueMonitorUrgentQueue::default();
        disk.grants.insert(7, known.clone());
        let mut stale = IssueMonitorUrgentQueue::default();
        stale.grants.insert(7, grant("2026-10-03T01:00:00Z", None));
        stale.rebase(&disk);
        assert_eq!(stale.grants.get(&7), Some(&known));
    }
}
