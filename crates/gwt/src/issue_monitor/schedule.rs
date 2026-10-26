//! Persistent member plans over the canonical terminal queue (Issue #4777).
use super::*;
use crate::agent_names::AgentMember;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScheduleReason {
    SameSpecContinuation,
    QueueOrder,
    AssignedByYou,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIssueReservation {
    pub host_id: String,
    pub agent_member_id: String,
    pub issue_number: u64,
    pub reason: AgentScheduleReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIssueSchedule {
    pub revision: u64,
    pub reservations: Vec<AgentIssueReservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentScheduleAction {
    Read,
    Reassign {
        issue_number: u64,
        source_member_id: String,
        target_member_id: String,
        expected_roster_revision: u64,
        expected_queue_revision: String,
        expected_schedule_revision: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentScheduledIssue {
    pub issue_number: u64,
    pub reason: AgentScheduleReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemberSchedule {
    pub agent_member_id: String,
    pub issues: Vec<AgentScheduledIssue>,
    pub empty_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentScheduleSnapshot {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_key: String,
    pub roster_revision: u64,
    pub queue_revision: String,
    pub schedule_revision: u64,
    pub members: Vec<AgentMemberSchedule>,
}

impl AgentIssueSchedule {
    pub(super) fn validate(&self) -> Result<(), String> {
        let mut owners = BTreeSet::new();
        for reservation in &self.reservations {
            if reservation.host_id.is_empty()
                || reservation.agent_member_id.is_empty()
                || reservation.issue_number == 0
                || !owners.insert(reservation.issue_number)
            {
                return Err("Agent schedule contains an ambiguous Issue owner; read issue.monitor.schedule.read before retrying".into());
            }
        }
        Ok(())
    }

    pub(crate) fn consume(&mut self, host: &str, number: u64) -> io::Result<()> {
        let before = self.reservations.len();
        self.reservations
            .retain(|entry| entry.host_id != host || entry.issue_number != number);
        if before != self.reservations.len() {
            self.advance()?;
        }
        Ok(())
    }

    fn advance(&mut self) -> io::Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Agent schedule revision exhausted"))?;
        Ok(())
    }
}

impl IssueMonitorPrefs {
    pub fn agent_schedule_queue_revision(&self) -> String {
        let queues: BTreeMap<_, _> = self
            .terminal_queues
            .iter()
            .map(|(host, queue)| {
                (
                    host,
                    self.urgent_queue
                        .projection(queue)
                        .entries
                        .into_iter()
                        .map(|entry| entry.number)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        format!(
            "queue-v1:{:x}",
            Sha256::digest(serde_json::to_vec(&queues).expect("queue order serializes"))
        )
    }

    pub(super) fn scheduled_issue_has_pending_claim(&self, number: u64) -> bool {
        self.pending_effects.iter().any(|effect| {
            matches!(&effect.payload, IssueMonitorEffectPayload::AcquireClaim { issue_number, .. } if *issue_number == number)
        })
    }

    pub(super) fn scheduled_issue_in_flight(&self, number: u64) -> bool {
        self.launching_issues
            .iter()
            .any(|entry| entry.issue_number == number)
            || self
                .pending_launch_deliveries
                .iter()
                .any(|entry| entry.issue_number == number)
            || self
                .launched_issues
                .iter()
                .any(|entry| entry.issue_number == number)
            || self
                .agent_roster
                .iter()
                .flat_map(|roster| &roster.members)
                .any(|member| {
                    member
                        .assignment
                        .as_ref()
                        .is_some_and(|assignment| assignment.issue_number == Some(number))
                })
    }

    fn agent_schedule_snapshot(&self, host: &str) -> io::Result<AgentScheduleSnapshot> {
        let roster = self.agent_roster.as_ref().ok_or_else(|| {
            io::Error::other(
                "Agent roster is not initialized; read issue.monitor.schedule.read before retrying",
            )
        })?;
        self.agent_issue_schedule
            .validate()
            .map_err(io::Error::other)?;
        let members = roster
            .members
            .iter()
            .filter(|member| member.enabled || member.assignment.is_some())
            .map(|member| {
                let issues: Vec<_> = self
                    .agent_issue_schedule
                    .reservations
                    .iter()
                    .filter(|entry| {
                        entry.host_id == host && entry.agent_member_id == member.agent_member_id
                    })
                    .map(|entry| AgentScheduledIssue {
                        issue_number: entry.issue_number,
                        reason: entry.reason,
                    })
                    .collect();
                AgentMemberSchedule {
                    agent_member_id: member.agent_member_id.clone(),
                    empty_reason: issues.is_empty().then(|| "No queued issue".into()),
                    issues,
                }
            })
            .collect();
        Ok(AgentScheduleSnapshot {
            project_key: String::new(),
            roster_revision: roster.revision,
            queue_revision: self.agent_schedule_queue_revision(),
            schedule_revision: self.agent_issue_schedule.revision,
            members,
        })
    }
}

/// Build preferences only; reservations never acquire claims or authorize launches.
fn reconcile(
    prefs: &mut IssueMonitorPrefs,
    specs: &BTreeMap<u64, u64>,
    historical: &BTreeMap<u64, String>,
    host: &str,
) -> io::Result<()> {
    let Some(roster) = prefs.agent_roster.as_ref() else {
        return Ok(());
    };
    prefs
        .agent_issue_schedule
        .validate()
        .map_err(io::Error::other)?;
    let queue = prefs
        .terminal_queues
        .get(host)
        .map(|queue| prefs.urgent_queue.projection(queue).entries)
        .unwrap_or_default();
    let pool = prefs.launch_profile_pool();
    let selected = pool
        .first()
        .filter(|profile| profile.agent_member_id.is_some())
        .map(|primary| {
            pool.iter()
                .filter(|profile| {
                    normalize_issue_monitor_provider(&profile.agent_id)
                        == normalize_issue_monitor_provider(&primary.agent_id)
                })
                .filter_map(|profile| profile.agent_member_id.as_deref())
                .collect::<BTreeSet<_>>()
        });
    let eligible = |member: &&AgentMember| {
        member.enabled
            && selected
                .as_ref()
                .is_none_or(|ids| ids.contains(member.agent_member_id.as_str()))
            && member.assignment.as_ref().is_none_or(|assignment| {
                assignment.host_id.is_empty() || assignment.host_id == host
            })
    };
    let members: Vec<_> = roster.members.iter().filter(eligible).collect();
    let before = prefs.agent_issue_schedule.reservations.clone();
    let mut reservations = before
        .iter()
        .filter(|entry| {
            (entry.host_id != host
                && prefs
                    .terminal_queues
                    .get(&entry.host_id)
                    .is_some_and(|queue| {
                        queue
                            .entries
                            .iter()
                            .any(|queued| queued.number == entry.issue_number)
                    }))
                || (entry.host_id == host
                    && queue
                        .iter()
                        .any(|queued| queued.number == entry.issue_number)
                    && !prefs.scheduled_issue_in_flight(entry.issue_number)
                    && (prefs.scheduled_issue_has_pending_claim(entry.issue_number)
                        || entry.reason == AgentScheduleReason::AssignedByYou
                        || prefs.agent_switch_intents.values().any(|intent| {
                            intent.has_admission_pin()
                                && intent.request.target_issue_number == entry.issue_number
                                && intent.request.member_id == entry.agent_member_id
                                && intent.source.assignment.host_id == host
                        })
                        || members
                            .iter()
                            .any(|member| member.agent_member_id == entry.agent_member_id)))
        })
        .cloned()
        .collect::<Vec<_>>();
    for entry in queue {
        if prefs.scheduled_issue_in_flight(entry.number)
            || prefs.scheduled_issue_has_pending_claim(entry.number)
            || reservations
                .iter()
                .any(|reservation| reservation.issue_number == entry.number)
        {
            continue;
        }
        if let Some(pin) = prefs.agent_switch_intents.values().find(|intent| {
            intent.has_admission_pin()
                && intent.request.target_issue_number == entry.number
                && intent.source.assignment.host_id == host
        }) {
            reservations.push(AgentIssueReservation {
                host_id: host.into(),
                agent_member_id: pin.request.member_id.clone(),
                issue_number: entry.number,
                reason: AgentScheduleReason::QueueOrder,
            });
            continue;
        }
        let spec = specs.get(&entry.number);
        let preferred = spec.and_then(|spec| historical.get(spec));
        let same_spec = members
            .iter()
            .copied()
            .find(|member| {
                member.assignment.is_none()
                    && spec.is_some()
                    && preferred.is_some_and(|id| id == &member.agent_member_id)
            })
            .or_else(|| {
                members.iter().copied().find(|member| {
                    member.assignment.is_none()
                        && spec.is_some()
                        && member.last_spec_number.as_ref() == spec
                })
            });
        let load = |member: &AgentMember| {
            reservations
                .iter()
                .filter(|entry| entry.agent_member_id == member.agent_member_id)
                .count()
        };
        let minimum = members.iter().map(|member| load(member)).min().unwrap_or(0);
        let same_spec = same_spec.filter(|member| load(member) == minimum);
        let member = same_spec.or_else(|| {
            members
                .iter()
                .copied()
                .min_by_key(|member| (load(member), member.assignment.is_some()))
        });
        if let Some(member) = member {
            reservations.push(AgentIssueReservation {
                host_id: host.into(),
                agent_member_id: member.agent_member_id.clone(),
                issue_number: entry.number,
                reason: if same_spec.is_some() {
                    AgentScheduleReason::SameSpecContinuation
                } else {
                    AgentScheduleReason::QueueOrder
                },
            });
        }
    }
    // Automatic plans are preferences until claiming starts. A busy member
    // must not strand them while another member can admit the work now.
    let switch_exclusion = |number, member: &str| {
        prefs.agent_switch_admission_exclusion(Some(number), member, false, host)
    };
    for index in 0..reservations.len() {
        let entry = &reservations[index];
        if entry.host_id != host
            || entry.reason == AgentScheduleReason::AssignedByYou
            || prefs.scheduled_issue_has_pending_claim(entry.issue_number)
            || prefs.agent_switch_intents.values().any(|intent| {
                intent.request.target_issue_number == entry.issue_number
                    && intent.request.member_id == entry.agent_member_id
                    && intent.source.assignment.host_id == host
                    && !matches!(
                        intent.stage,
                        AgentSwitchStage::Completed
                            | AgentSwitchStage::Cancelled
                            | AgentSwitchStage::Failed
                    )
            })
            || !members.iter().any(|member| {
                member.agent_member_id == entry.agent_member_id
                    && (member.assignment.is_some()
                        || switch_exclusion(entry.issue_number, &member.agent_member_id).is_some())
            })
        {
            continue;
        }
        let idle = members
            .iter()
            .copied()
            .filter(|member| {
                member.assignment.is_none()
                    && switch_exclusion(entry.issue_number, &member.agent_member_id).is_none()
                    && !prefs.pending_launch_deliveries.iter().any(|delivery| {
                        delivery.agent_member_id.as_deref() == Some(&member.agent_member_id)
                    })
                    && !reservations.iter().any(|reservation| {
                        reservation.agent_member_id == member.agent_member_id
                            && prefs.scheduled_issue_has_pending_claim(reservation.issue_number)
                    })
            })
            .min_by_key(|member| {
                reservations
                    .iter()
                    .filter(|entry| entry.agent_member_id == member.agent_member_id)
                    .count()
            });
        if let Some(member) = idle {
            reservations[index]
                .agent_member_id
                .clone_from(&member.agent_member_id);
            reservations[index].reason = AgentScheduleReason::QueueOrder;
        }
    }
    // Preserve the canonical queue order within each member's plan.
    let local_order: Vec<_> = prefs
        .terminal_queues
        .get(host)
        .map(|queue| {
            prefs
                .urgent_queue
                .projection(queue)
                .entries
                .into_iter()
                .map(|entry| entry.number)
                .collect()
        })
        .unwrap_or_default();
    reservations.sort_by_key(|entry| {
        (
            entry.host_id.clone(),
            if entry.host_id == host {
                local_order
                    .iter()
                    .position(|number| *number == entry.issue_number)
                    .unwrap_or(usize::MAX)
            } else {
                before
                    .iter()
                    .position(|old| old == entry)
                    .unwrap_or(usize::MAX)
            },
        )
    });
    if reservations != before {
        prefs.agent_issue_schedule.reservations = reservations;
        prefs.agent_issue_schedule.advance()?;
    }
    Ok(())
}

fn context(
    root: &Path,
    prefs: &IssueMonitorPrefs,
    sessions: &Path,
) -> (BTreeMap<u64, u64>, BTreeMap<u64, String>) {
    let specs: BTreeMap<_, _> = prefs
        .terminal_queues
        .values()
        .flat_map(|queue| &queue.entries)
        .filter_map(|entry| {
            crate::issue_monitor_worker::cached_agent_member_spec_owner(root, entry.number)
                .map(|owner| (entry.number, owner))
        })
        .collect();
    // Observe the Session ledger once for the queue, rather than once per SPEC.
    let historical = crate::agent_names::previous_spec_members(
        root,
        sessions,
        &specs.values().copied().collect(),
    );
    (specs, historical)
}

pub fn mutate_project_agent_schedule(
    root: &Path,
    sessions_dir: &Path,
    action: AgentScheduleAction,
) -> Result<AgentScheduleSnapshot, String> {
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    if matches!(action, AgentScheduleAction::Read) {
        crate::agent_names::read_project_roster(root, sessions_dir)?;
    }
    let host = crate::process::current_hostname();
    let (specs, historical) = if matches!(action, AgentScheduleAction::Read) {
        let observed = load_issue_monitor_prefs(&path).map_err(|error| error.to_string())?;
        context(root, &observed, sessions_dir)
    } else {
        (BTreeMap::new(), BTreeMap::new())
    };
    let (saved, _) = crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        if let AgentScheduleAction::Reassign {issue_number, source_member_id, target_member_id, expected_roster_revision, expected_queue_revision, expected_schedule_revision} = &action {
            let snapshot = prefs.agent_schedule_snapshot(&host)?;
            if snapshot.roster_revision != *expected_roster_revision || snapshot.queue_revision != *expected_queue_revision || snapshot.schedule_revision != *expected_schedule_revision {
                return Err(io::Error::other(format!("Agent schedule changed: expected roster={expected_roster_revision}, queue={expected_queue_revision}, schedule={expected_schedule_revision}; current roster={}, queue={}, schedule={}; read issue.monitor.schedule.read and retry",snapshot.roster_revision,snapshot.queue_revision,snapshot.schedule_revision)));
            }
            if prefs.agent_switch_intents.values().any(|intent| intent.has_admission_pin() && intent.request.target_issue_number == *issue_number) {
                return Err(io::Error::other("Switch target is pinned to its selected member; wait for the switch or resume its saved source work"));
            }
            if prefs.scheduled_issue_in_flight(*issue_number) || prefs.scheduled_issue_has_pending_claim(*issue_number) {return Err(io::Error::other("Issue already claiming, launching or running; read issue.monitor.schedule.read and retry"));}
            let target = prefs.agent_roster.as_ref().and_then(|roster| roster.members.iter().find(|member| member.agent_member_id == *target_member_id)).ok_or_else(|| io::Error::other("Target Agent member is absent; read issue.monitor.schedule.read and retry"))?;
            if !target.enabled || target.assignment.as_ref().is_some_and(|assignment| !assignment.host_id.is_empty() && assignment.host_id != host) {
                return Err(io::Error::other("Target Agent must be enabled and belong to this host; read issue.monitor.schedule.read and choose a local member"));
            }
            let queued = prefs.terminal_queues.get(&host).is_some_and(|queue| queue.entries.iter().any(|entry| entry.number == *issue_number));
            let reservation = prefs.agent_issue_schedule.reservations.iter_mut().find(|entry| entry.issue_number == *issue_number && entry.host_id == host && entry.agent_member_id == *source_member_id).filter(|_| queued).ok_or_else(|| io::Error::other("Expected source member does not own this queued Issue on this host; read issue.monitor.schedule.read and retry"))?;
            if source_member_id != target_member_id {
                reservation.agent_member_id.clone_from(target_member_id);
                reservation.reason = AgentScheduleReason::AssignedByYou;
                prefs.agent_issue_schedule.advance()?;
            }
        } else {
            reconcile(prefs, &specs, &historical, &host)?;
        }
        Ok(())
    }).map_err(|error| error.to_string())?;
    let readback = load_issue_monitor_prefs(&path).map_err(|error| error.to_string())?;
    let accepted = saved
        .agent_schedule_snapshot(&host)
        .map_err(|error| error.to_string())?;
    let mut canonical = readback
        .agent_schedule_snapshot(&host)
        .map_err(|error| error.to_string())?;
    if accepted != canonical {
        return Err("Agent schedule changed during readback; read issue.monitor.schedule.read before retrying (the previous move may already be saved)".into());
    }
    canonical.project_key = gwt_core::paths::project_scope_hash(root).to_string();
    Ok(canonical)
}

impl IssueMonitorState {
    pub fn synchronize_agent_issue_schedule(&mut self, root: &Path) -> io::Result<()> {
        if self.agent_roster.is_none() {
            return Ok(());
        }
        let sessions = gwt_core::paths::gwt_sessions_dir();
        // Refresh exact physical occupancy through the existing roster transaction.
        self.agent_roster = Some(
            crate::agent_names::read_project_roster(root, &sessions).map_err(io::Error::other)?,
        );
        let mut prefs = self.prefs();
        self.agent_schedule_source = Some((
            prefs.agent_issue_schedule.clone(),
            prefs.agent_schedule_queue_revision(),
            prefs
                .agent_roster
                .as_ref()
                .expect("roster initialized")
                .revision,
        ));
        let (specs, historical) = context(root, &prefs, &sessions);
        reconcile(
            &mut prefs,
            &specs,
            &historical,
            &crate::process::current_hostname(),
        )?;
        self.agent_issue_schedule = prefs.agent_issue_schedule;
        Ok(())
    }

    pub fn scheduled_agent_member_id(&self, number: u64) -> Option<&str> {
        self.pending_launch_deliveries
            .iter()
            .find(|delivery| delivery.issue_number == number)
            .and_then(|delivery| delivery.agent_member_id.as_deref())
            .or_else(|| {
                self.agent_issue_schedule
                    .reservations
                    .iter()
                    .find(|entry| {
                        entry.issue_number == number
                            && entry.host_id == crate::process::current_hostname()
                    })
                    .map(|entry| entry.agent_member_id.as_str())
            })
    }

    pub(super) fn rebase_agent_issue_schedule(&mut self, disk: &IssueMonitorPrefs) {
        // A scan may propose automatic plans. Retain them only while all their
        // inputs still match; a concurrent operator move always wins on disk.
        let current = self.prefs();
        let keep = self
            .agent_schedule_source
            .as_ref()
            .is_some_and(|(schedule, queue, roster)| {
                *schedule == disk.agent_issue_schedule
                    && *queue == current.agent_schedule_queue_revision()
                    && current
                        .agent_roster
                        .as_ref()
                        .is_some_and(|current| current.revision == *roster)
            });
        if !keep {
            self.agent_issue_schedule = disk.agent_issue_schedule.clone();
            self.agent_schedule_source = None;
        }
    }

    pub fn agent_schedule_launch_exclusion(&self, number: u64) -> Option<String> {
        let host = crate::process::current_hostname();
        if let Some(foreign) = self
            .agent_issue_schedule
            .reservations
            .iter()
            .find(|entry| entry.issue_number == number && entry.host_id != host)
        {
            return Some(format!("scheduled_on_other_host: expected host={}, current host={host}; read issue.monitor.schedule.read on that host before changing its queue", foreign.host_id));
        }
        if let Some(reason) = self.agent_switch_intents.values().find_map(|intent| {
            intent.admission_exclusion(Some(number), self.scheduled_agent_member_id(number), false)
        }) {
            return Some(reason);
        }
        // Legacy direct requests do not belong to the saved queue. A filtered
        // queued row still needs a plan and never becomes a legacy request.
        let legacy = self.agent_roster.is_none()
            || !self
                .stored_local_terminal_queue_numbers()
                .any(|queued| queued == number)
            || self.pending_launch_deliveries.iter().any(|delivery| {
                delivery.issue_number == number && delivery.agent_member_id.is_none()
            });
        let id = match self.scheduled_agent_member_id(number) {
            Some(id) => id,
            None if legacy => return None,
            None => return Some("agent_plan_missing: expected a local member reservation, current plan is absent; read issue.monitor.schedule.read or wait for the next Monitor scan".into()),
        };
        let member = self.agent_roster.as_ref().and_then(|roster| {
            roster
                .members
                .iter()
                .find(|member| member.agent_member_id == id)
        });
        let selected = self
            .launch_profiles
            .first()
            .filter(|profile| profile.agent_member_id.is_some());
        let tool_matches = selected.is_none_or(|primary| {
            member.is_some_and(|member| {
                member
                    .agent_id
                    .as_deref()
                    .and_then(normalize_issue_monitor_provider)
                    == normalize_issue_monitor_provider(&primary.agent_id)
                    && self.launch_profiles.iter().any(|profile| {
                        profile.agent_member_id.as_deref() == Some(id)
                            && normalize_issue_monitor_provider(&profile.agent_id)
                                == normalize_issue_monitor_provider(&primary.agent_id)
                    })
            })
        });
        let available = tool_matches && member.is_some_and(|member| match &member.assignment {
            None => member.enabled,
            Some(assignment) => assignment.issue_number == Some(number)
                && (assignment.host_id.is_empty() || assignment.host_id == host)
                && assignment.delivery_id.as_ref().is_some_and(|id| self.pending_launch_deliveries.iter().any(|delivery| delivery.issue_number == number && delivery.delivery_id == *id && delivery.agent_member_id.as_deref() == Some(member.agent_member_id.as_str()))),
        })
            && !self.pending_launch_deliveries.iter().any(|delivery| delivery.issue_number != number && delivery.agent_member_id.as_deref() == Some(id))
            && !self.pending_effects.iter().any(|effect| matches!(&effect.payload, IssueMonitorEffectPayload::AcquireClaim {issue_number, ..} if *issue_number != number && self.scheduled_agent_member_id(*issue_number) == Some(id)));
        let current = member
            .map(|member| {
                format!(
                    "enabled={}, assignment={:?}",
                    member.enabled,
                    member
                        .assignment
                        .as_ref()
                        .map(|assignment| (&assignment.host_id, assignment.issue_number))
                )
            })
            .unwrap_or_else(|| "absent".into());
        (!available).then(|| format!("scheduled_member_unavailable: expected enabled local idle member {id}; current {current} (including pending claims/deliveries); read issue.monitor.schedule.read and reassign or wait for that member"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_names::AgentRoster;
    fn fixture() -> (
        tempfile::TempDir,
        gwt_core::test_support::ScopedGwtHome,
        std::path::PathBuf,
        std::path::PathBuf,
        AgentRoster,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let roster = crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            crate::agent_names::AgentRosterAction::Resize { size: 2 },
            None,
        )
        .unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.terminal_queues.insert(
                crate::process::current_hostname(),
                IssueMonitorTerminalQueue {
                    entries: [41, 42, 43]
                        .into_iter()
                        .map(|number| IssueMonitorTerminalQueueEntry {
                            number,
                            queued_at: "2026-10-25T00:00:00Z".into(),
                            queued_by: "test".into(),
                            priority: None,
                            priority_reason: None,
                            assigned_by: None,
                            assigned_at: None,
                        })
                        .collect(),
                    last_seen_at: None,
                },
            );
            Ok(())
        })
        .unwrap();
        (temp, home, root, sessions, roster)
    }
    fn move_action(
        snapshot: &AgentScheduleSnapshot,
        number: u64,
        from: &str,
        to: &str,
    ) -> AgentScheduleAction {
        AgentScheduleAction::Reassign {
            issue_number: number,
            source_member_id: from.into(),
            target_member_id: to.into(),
            expected_roster_revision: snapshot.roster_revision,
            expected_queue_revision: snapshot.queue_revision.clone(),
            expected_schedule_revision: snapshot.schedule_revision,
        }
    }

    fn install_switch_pin(root: &Path, member: &str, stage: AgentSwitchStage, exited: bool) {
        crate::try_mutate_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(root), |prefs| {
            let operation = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
            let intent = serde_json::from_value(serde_json::json!({
                "request":{"operation_id":operation,"member_id":member,"expected_roster_revision":1,"target_issue_number":41},
                "source":{"assignment":{"reservation_id":"old-reservation","session_id":"old-session","issue_number":99,"host_id":crate::process::current_hostname(),"host_pid":1,"host_started_at":2},
                    "window_id":"old-window","claim_id":"old-claim","confirmation":{"window_id":"old-window","claim_id":"old-claim","confirmed_at":"2026-10-26T00:00:00Z"}},
                "stage":stage,"runtime_exit_confirmed":exited,
                "checkpoint":{"verified_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","work_id":"saved-work","board_entry_id":"handoff"}
            })).unwrap();
            prefs.agent_switch_intents.insert(operation.into(), intent);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn agent_switch_pin_fences_monitor_admission_and_target_reassignment() {
        let (_temp, _home, root, sessions, roster) = fixture();
        let snapshot =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let member = &roster.members[0].agent_member_id;
        install_switch_pin(&root, member, AgentSwitchStage::StoppingCurrent, false);
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let prefs = load_issue_monitor_prefs(&path).unwrap();
        let mut automatic = prefs.clone();
        let operation = automatic
            .agent_switch_intents
            .keys()
            .next()
            .unwrap()
            .clone();
        automatic
            .agent_switch_intents
            .get_mut(&operation)
            .unwrap()
            .stage = AgentSwitchStage::Checkpointing;
        let mut assignment = automatic.agent_switch_intents[&operation]
            .source
            .assignment
            .clone();
        automatic.agent_roster.as_mut().unwrap().members[0].assignment = Some(assignment.clone());
        let host = crate::process::current_hostname();
        let owner = |prefs: &IssueMonitorPrefs, number| {
            prefs
                .agent_issue_schedule
                .reservations
                .iter()
                .find(|entry| entry.issue_number == number)
                .unwrap()
                .agent_member_id
                .clone()
        };
        reconcile(&mut automatic, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        let selected_target_kept = owner(&automatic, 41) == *member;
        let unrelated_moved = owner(&automatic, 43) == roster.members[1].agent_member_id;

        automatic
            .agent_switch_intents
            .get_mut(&operation)
            .unwrap()
            .stage = AgentSwitchStage::Unknown;
        automatic.agent_roster.as_mut().unwrap().members[0].assignment = None;
        automatic
            .agent_issue_schedule
            .reservations
            .iter_mut()
            .find(|entry| entry.issue_number == 43)
            .unwrap()
            .agent_member_id = member.clone();
        reconcile(&mut automatic, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        let pinned_plan_moved = owner(&automatic, 43) == roster.members[1].agent_member_id;
        assignment.issue_number = Some(100);
        assignment.session_id = None;
        automatic.agent_roster.as_mut().unwrap().members[1].assignment = Some(assignment);
        reconcile(&mut automatic, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        let pinned_destination_skipped = owner(&automatic, 42) == roster.members[1].agent_member_id;
        assert!(selected_target_kept && unrelated_moved && pinned_plan_moved && pinned_destination_skipped,
            "selected target kept={selected_target_kept}, ordinary busy plan moved={unrelated_moved}, unrelated pinned plan moved={pinned_plan_moved}, pinned destination skipped={pinned_destination_skipped}");

        let monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        assert!(
            monitor.agent_schedule_launch_exclusion(43).is_some(),
            "the freed member is pinned against its other queued Issues"
        );
        assert!(
            monitor.agent_schedule_launch_exclusion(41).is_some(),
            "the selected target must wait for physical exit"
        );
        assert!(
            monitor.agent_schedule_launch_exclusion(42).is_none(),
            "another member is unaffected"
        );
        let before = fs::read(&path).unwrap();
        assert!(mutate_project_agent_schedule(
            &root,
            &sessions,
            move_action(&snapshot, 41, member, &roster.members[1].agent_member_id)
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut launching = prefs;
        let pin = launching.agent_switch_intents.values_mut().next().unwrap();
        pin.stage = AgentSwitchStage::LaunchingNext;
        pin.runtime_exit_confirmed = true;
        assert!(
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), launching.clone())
                .agent_schedule_launch_exclusion(41)
                .is_none()
        );
        launching
            .agent_issue_schedule
            .reservations
            .iter_mut()
            .find(|entry| entry.issue_number == 41)
            .unwrap()
            .agent_member_id = roster.members[1].agent_member_id.clone();
        assert!(
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), launching)
                .agent_schedule_launch_exclusion(41)
                .is_some(),
            "moving a reservation cannot bypass the pinned member"
        );
        install_switch_pin(&root, member, AgentSwitchStage::Unknown, true);
        let monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            load_issue_monitor_prefs(&path).unwrap(),
        );
        assert!(monitor.agent_schedule_launch_exclusion(41).is_some());
        assert!(monitor.agent_schedule_launch_exclusion(43).is_some());
        crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            crate::agent_names::AgentRosterAction::Rest {
                member_id: member.clone(),
            },
            None,
        )
        .unwrap();
        mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        assert_eq!(
            load_issue_monitor_prefs(&path)
                .unwrap()
                .agent_issue_schedule
                .reservations
                .iter()
                .find(|entry| entry.issue_number == 41)
                .unwrap()
                .agent_member_id,
            *member,
            "reconciliation must retain the selected target's member even while resting"
        );
    }

    #[test]
    fn agent_switch_pin_reaches_physical_reservation_and_allows_only_explicit_saved_resume() {
        let (_temp, _home, root, sessions, roster) = fixture();
        mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let member = &roster.members[0].agent_member_id;
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        install_switch_pin(&root, member, AgentSwitchStage::StoppingCurrent, false);
        let before = fs::read(&path).unwrap();
        assert!(crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "other",
            Some(43),
            None,
            None,
            Some(member)
        )
        .is_err());
        assert!(crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "target",
            Some(41),
            None,
            None,
            None
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        install_switch_pin(&root, member, AgentSwitchStage::LaunchingNext, true);
        let next = crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "target",
            Some(41),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(next.agent_member_id, *member);
        crate::agent_names::release_project_member(&root, &sessions, member, "target").unwrap();
        install_switch_pin(&root, member, AgentSwitchStage::Unknown, false);
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            let source = prefs
                .agent_switch_intents
                .values()
                .next()
                .unwrap()
                .source
                .assignment
                .clone();
            prefs.agent_roster.as_mut().unwrap().members[0].assignment = Some(source);
            Ok(())
        })
        .unwrap();
        let uncertain = fs::read(&path).unwrap();
        assert!(crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "resume",
            Some(99),
            None,
            None,
            Some(member)
        )
        .is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            uncertain,
            "an unobserved source Session cannot be cleared by the resume exception"
        );
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.agent_roster.as_mut().unwrap().members[0].assignment = None;
            Ok(())
        })
        .unwrap();
        for (stage, reservation) in [
            (AgentSwitchStage::Unknown, "saved-unknown"),
            (AgentSwitchStage::LaunchingNext, "saved-waiting"),
        ] {
            install_switch_pin(&root, member, stage, false);
            let monitor = IssueMonitorState::with_prefs(
                IssueMonitorConfig::default(),
                load_issue_monitor_prefs(&path).unwrap(),
            );
            assert!(monitor
                .agent_schedule_launch_exclusion(41)
                .is_some_and(|reason| reason.contains("agent_switch_pending")));
            assert!(
                crate::agent_names::reserve_project_member(
                    &root,
                    &sessions,
                    "automatic-target",
                    Some(41),
                    None,
                    None,
                    None
                )
                .is_err(),
                "automatic admission still requires physical exit receipt"
            );
            let resumed = crate::agent_names::reserve_project_member(
                &root,
                &sessions,
                reservation,
                Some(99),
                None,
                None,
                Some(member),
            )
            .unwrap();
            assert_eq!(resumed.agent_member_id, *member);
            let saved = load_issue_monitor_prefs(&path).unwrap();
            let pin = saved.agent_switch_intents.values().next().unwrap();
            assert_eq!(pin.stage, AgentSwitchStage::Failed);
            assert!(!pin.runtime_exit_confirmed && pin.checkpoint.is_some());
            crate::agent_names::release_project_member(&root, &sessions, member, reservation)
                .unwrap();
        }
        install_switch_pin(&root, member, AgentSwitchStage::Unknown, true);
        assert!(crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "wrong-member",
            Some(99),
            None,
            None,
            Some(&roster.members[1].agent_member_id)
        )
        .is_err());
        assert!(crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "wrong-issue",
            Some(43),
            None,
            None,
            Some(member)
        )
        .is_err());
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.launching_issues.push(IssueMonitorLaunchingIssue {
                issue_number: 41,
                claimed_at: None,
            });
            Ok(())
        })
        .unwrap();
        assert!(crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "target-in-flight",
            Some(99),
            None,
            None,
            Some(member)
        )
        .is_err());
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.launching_issues.clear();
            prefs
                .agent_issue_schedule
                .reservations
                .push(AgentIssueReservation {
                    host_id: crate::process::current_hostname(),
                    agent_member_id: member.clone(),
                    issue_number: 99,
                    reason: AgentScheduleReason::QueueOrder,
                });
            prefs.agent_issue_schedule.revision = u64::MAX;
            Ok(())
        })
        .unwrap();
        assert!(crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "failed-commit",
            Some(99),
            None,
            None,
            Some(member)
        )
        .is_err());
        assert_eq!(
            load_issue_monitor_prefs(&path)
                .unwrap()
                .agent_switch_intents
                .values()
                .next()
                .unwrap()
                .stage,
            AgentSwitchStage::Unknown
        );
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.agent_issue_schedule.revision = 1;
            Ok(())
        })
        .unwrap();
        let resumed = crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "resume",
            Some(99),
            None,
            None,
            Some(member),
        )
        .unwrap();
        assert_eq!(resumed.name, roster.members[0].name);
        let saved = load_issue_monitor_prefs(&path).unwrap();
        let pin = saved.agent_switch_intents.values().next().unwrap();
        assert_eq!(
            pin.stage,
            AgentSwitchStage::Failed,
            "only a committed explicit source reservation releases the pin"
        );
        assert!(pin.runtime_exit_confirmed && pin.checkpoint.is_some());
    }
    #[test]
    fn agent_schedule_reassignment_is_atomic_and_stale_observations_preserve_bytes() {
        let (_temp, _home, root, sessions, roster) = fixture();
        let before =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        assert_eq!(
            before.project_key,
            gwt_core::paths::project_scope_hash(&root).to_string(),
            "canonical schedules identify their project before the UI can mutate them"
        );
        assert_eq!(
            before.members[0]
                .issues
                .iter()
                .map(|issue| issue.issue_number)
                .collect::<Vec<_>>(),
            vec![41, 43]
        );
        assert_eq!(before.members[1].issues[0].issue_number, 42);
        let action = move_action(
            &before,
            41,
            &roster.members[0].agent_member_id,
            &roster.members[1].agent_member_id,
        );
        let moved = mutate_project_agent_schedule(&root, &sessions, action.clone()).unwrap();
        assert_eq!(
            moved.members[1].issues[0].reason,
            AgentScheduleReason::AssignedByYou
        );
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let bytes = fs::read(&path).unwrap();
        let error = mutate_project_agent_schedule(&root, &sessions, action).unwrap_err();
        assert!(
            error.contains("expected roster=")
                && error.contains("current roster=")
                && error.contains("issue.monitor.schedule.read"),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        // A cold preferences load must retain operator attribution and both owners.
        let cold = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            load_issue_monitor_prefs(&path).unwrap(),
        )
        .prefs();
        let mut legacy_state = serde_json::to_value(IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            cold.clone(),
        ))
        .unwrap();
        assert!(legacy_state.get("agent_schedule_source").is_none());
        legacy_state
            .as_object_mut()
            .unwrap()
            .remove("agent_issue_schedule");
        assert!(serde_json::from_value::<IssueMonitorState>(legacy_state).is_ok());
        let mut cold_snapshot = cold
            .agent_schedule_snapshot(&crate::process::current_hostname())
            .unwrap();
        cold_snapshot.project_key = gwt_core::paths::project_scope_hash(&root).to_string();
        assert_eq!(cold_snapshot, moved);
        let valid = move_action(
            &moved,
            41,
            &roster.members[1].agent_member_id,
            &roster.members[0].agent_member_id,
        );
        assert!(mutate_project_agent_schedule(&root, &sessions, valid).is_ok());
        let current =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let action = move_action(
            &current,
            41,
            &roster.members[0].agent_member_id,
            &roster.members[1].agent_member_id,
        );
        let task_home = gwt_core::test_support::gwt_home_override().expect("isolated user home");
        let outcomes = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                let _home = gwt_core::test_support::ScopedGwtHome::set(&task_home);
                mutate_project_agent_schedule(&root, &sessions, action.clone())
            });
            let second = scope.spawn(|| {
                let _home = gwt_core::test_support::ScopedGwtHome::set(&task_home);
                mutate_project_agent_schedule(&root, &sessions, action.clone())
            });
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(
            outcomes.iter().filter(|result| result.is_ok()).count(),
            1,
            "{outcomes:?}"
        );
        assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);
    }
    #[test]
    fn agent_schedule_same_spec_busy_retention_removal_and_short_queue() {
        let (_temp, _home, root, sessions, roster) = fixture();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let host = crate::process::current_hostname();
        let mut prefs = load_issue_monitor_prefs(&path).unwrap();
        prefs.agent_roster.as_mut().unwrap().members[1].last_spec_number = Some(41);
        reconcile(
            &mut prefs,
            &BTreeMap::from([(41, 41)]),
            &BTreeMap::new(),
            &host,
        )
        .unwrap();
        let reservation = prefs
            .agent_issue_schedule
            .reservations
            .iter()
            .find(|entry| entry.issue_number == 41)
            .unwrap();
        assert_eq!(
            reservation.agent_member_id,
            roster.members[1].agent_member_id
        );
        assert_eq!(
            reservation.reason,
            AgentScheduleReason::SameSpecContinuation
        );
        let busy = crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "busy",
            Some(99),
            None,
            None,
            Some(&roster.members[1].agent_member_id),
        )
        .unwrap();
        prefs.agent_roster.as_mut().unwrap().members[1] = busy;
        let mut manual = prefs.clone();
        manual
            .agent_issue_schedule
            .reservations
            .iter_mut()
            .find(|entry| entry.issue_number == 41)
            .unwrap()
            .reason = AgentScheduleReason::AssignedByYou;
        reconcile(&mut manual, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        let manual = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), manual);
        assert_eq!(
            manual.scheduled_agent_member_id(41),
            Some(roster.members[1].agent_member_id.as_str()),
            "an explicit member choice keeps its busy reservation"
        );
        assert!(manual.agent_schedule_launch_exclusion(41).is_some());

        let revision = prefs.agent_issue_schedule.revision;
        reconcile(&mut prefs, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        let monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        assert!(monitor.agent_schedule_launch_exclusion(41).is_none());
        assert_eq!(
            monitor.scheduled_agent_member_id(41),
            Some(roster.members[0].agent_member_id.as_str()),
            "an automatic reservation uses the idle member even with more future work"
        );
        assert_eq!(prefs.agent_issue_schedule.revision, revision + 1);
        assert_eq!(
            prefs
                .agent_issue_schedule
                .reservations
                .iter()
                .map(|entry| (entry.issue_number, entry.reason))
                .collect::<Vec<_>>(),
            [41, 42, 43].map(|number| (number, AgentScheduleReason::QueueOrder))
        );
        let planned = prefs.agent_issue_schedule.clone();
        prefs = serde_json::from_value(serde_json::to_value(&prefs).unwrap()).unwrap();
        reconcile(&mut prefs, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        assert_eq!(prefs.agent_issue_schedule, planned);

        let busy = crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "busy-other",
            Some(100),
            None,
            None,
            Some(&roster.members[0].agent_member_id),
        )
        .unwrap();
        let mut all_busy = prefs.clone();
        all_busy.agent_roster.as_mut().unwrap().members[0] = busy;
        reconcile(&mut all_busy, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        assert_eq!(all_busy.agent_issue_schedule, planned);
        prefs
            .terminal_queues
            .get_mut(&host)
            .unwrap()
            .entries
            .retain(|entry| entry.number == 42);
        prefs.agent_roster.as_mut().unwrap().members[1].enabled = false;
        reconcile(&mut prefs, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        let snapshot = prefs.agent_schedule_snapshot(&host).unwrap();
        assert_eq!(snapshot.members[0].issues.len(), 1);
        assert_eq!(
            snapshot.members[1].empty_reason.as_deref(),
            Some("No queued issue")
        );
        assert!(prefs
            .agent_issue_schedule
            .reservations
            .iter()
            .all(|entry| entry.issue_number == 42
                && entry.agent_member_id == roster.members[0].agent_member_id));
    }
    #[test]
    fn agent_schedule_queue_revision_and_wrong_source_reject_without_a_write() {
        let (_temp, _home, root, sessions, roster) = fixture();
        let snapshot =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let bad_source = move_action(
            &snapshot,
            41,
            &roster.members[1].agent_member_id,
            &roster.members[0].agent_member_id,
        );
        let original = fs::read(&path).unwrap();
        assert!(mutate_project_agent_schedule(&root, &sessions, bad_source)
            .unwrap_err()
            .contains("source member"));
        assert_eq!(fs::read(&path).unwrap(), original);
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs
                .terminal_queues
                .get_mut(&crate::process::current_hostname())
                .unwrap()
                .entries
                .reverse();
            Ok(())
        })
        .unwrap();
        let changed = fs::read(&path).unwrap();
        let stale = move_action(
            &snapshot,
            41,
            &roster.members[0].agent_member_id,
            &roster.members[1].agent_member_id,
        );
        assert!(mutate_project_agent_schedule(&root, &sessions, stale)
            .unwrap_err()
            .contains("queue="));
        assert_eq!(fs::read(&path).unwrap(), changed);
    }
    #[test]
    fn agent_schedule_pending_claim_fences_reassignment_without_losing_its_member() {
        let (_temp, _home, root, sessions, roster) = fixture();
        let snapshot =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs
                .pending_effects
                .push(PendingIssueMonitorEffect::prepared(
                    "claim-41",
                    0,
                    IssueMonitorEffectPayload::AcquireClaim {
                        issue_number: 41,
                        claim_id: "claim-41".into(),
                        owner: "host/session".into(),
                        heartbeat_at: "2026-10-25T00:00:00Z".into(),
                        expires_at: "2026-10-25T01:00:00Z".into(),
                        launched_work_id: None,
                    },
                ));
            Ok(())
        })
        .unwrap();
        let before = fs::read(&path).unwrap();
        let result = mutate_project_agent_schedule(
            &root,
            &sessions,
            move_action(
                &snapshot,
                41,
                &roster.members[0].agent_member_id,
                &roster.members[1].agent_member_id,
            ),
        );
        assert!(
            result.is_err(),
            "a prepared claim has already selected its member"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        let read =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        assert!(
            read.members[0]
                .issues
                .iter()
                .any(|issue| issue.issue_number == 41),
            "claim's member survives read/reconcile until delivery pins it"
        );
        let monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            load_issue_monitor_prefs(&path).unwrap(),
        );
        assert!(
            monitor.agent_schedule_launch_exclusion(41).is_none(),
            "the exact pending claim can finish"
        );
        assert!(
            monitor.agent_schedule_launch_exclusion(43).is_some(),
            "another Issue cannot use that member's pending claim slot"
        );
        assert!(
            monitor.agent_schedule_launch_exclusion(42).is_none(),
            "a different free member remains available"
        );

        crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "busy-42-member",
            Some(99),
            None,
            None,
            Some(&roster.members[1].agent_member_id),
        )
        .unwrap();
        let read =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        assert!(
            read.members[1]
                .issues
                .iter()
                .any(|issue| issue.issue_number == 42),
            "a member pinned by another pending claim is not an idle destination"
        );

        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            let key = prefs.pending_effects[0].attempt_key();
            assert!(prefs.mark_pending_effect_attempting(&key));
            Ok(())
        })
        .unwrap();
        crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            crate::agent_names::AgentRosterAction::Rest {
                member_id: roster.members[0].agent_member_id.clone(),
            },
            None,
        )
        .unwrap();
        mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let prefs = load_issue_monitor_prefs(&path).unwrap();
        assert_eq!(
            prefs.pending_effects[0].state,
            IssueMonitorEffectState::Attempting
        );
        let monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        assert_eq!(
            monitor.scheduled_agent_member_id(41),
            Some(roster.members[0].agent_member_id.as_str()),
            "rest must not retarget an in-flight claim to another member"
        );
        assert!(monitor.agent_schedule_launch_exclusion(41).is_some());

        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.pending_effects.clear();
            Ok(())
        })
        .unwrap();
        mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            load_issue_monitor_prefs(&path).unwrap(),
        );
        assert_eq!(
            monitor.scheduled_agent_member_id(41),
            Some(roster.members[1].agent_member_id.as_str()),
            "a settled claim permits normal replanning away from a resting member"
        );
    }

    #[test]
    fn agent_schedule_rest_keeps_exact_materialized_delivery_replay() {
        let (_temp, _home, root, sessions, roster) = fixture();
        mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "reserve-41",
            Some(41),
            None,
            Some("delivery-41"),
            Some(&roster.members[0].agent_member_id),
        )
        .unwrap();
        crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            crate::agent_names::AgentRosterAction::Rest {
                member_id: roster.members[0].agent_member_id.clone(),
            },
            None,
        )
        .unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let mut prefs = load_issue_monitor_prefs(&path).unwrap();
        let delivery: PendingIssueMonitorLaunchDelivery = serde_json::from_value(serde_json::json!({
            "agent_member_id":roster.members[0].agent_member_id, "delivery_id":"delivery-41", "issue_number":41,
            "branch_name":"work/issue-41", "linked_issue_kind":crate::LinkedIssueKind::Issue,
            "claim_id":"claim-41", "created_at":"2026-10-25T00:00:00Z", "materialized_window_id":"tab::pane"
        })).unwrap();
        prefs.pending_launch_deliveries.push(delivery.clone());
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        monitor.set_source_repository(Some(("fixture", "queue")));
        assert!(
            monitor.can_materialize_delivery(&delivery, Some("tab::pane")),
            "rest only changes future admission; exact installed delivery still acknowledges"
        );
        assert!(
            monitor.agent_schedule_launch_exclusion(41).is_none(),
            "the exact current delivery remains usable"
        );
        assert!(
            monitor.agent_schedule_launch_exclusion(43).is_some(),
            "resting member cannot take a different Issue"
        );
    }

    #[test]
    fn agent_schedule_adopted_queue_releases_only_its_orphaned_foreign_reservations() {
        let (_temp, _home, root, sessions, _) = fixture();
        mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        let host = crate::process::current_hostname();
        let mut prefs =
            load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(&root))
                .unwrap();
        let queue = prefs.terminal_queues.remove(&host).unwrap();
        prefs.terminal_queues.insert("old-host".into(), queue);
        for entry in &mut prefs.agent_issue_schedule.reservations {
            entry.host_id = "old-host".into();
        }
        reconcile(&mut prefs, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        assert!(
            monitor
                .agent_schedule_launch_exclusion(41)
                .unwrap()
                .contains("scheduled_on_other_host"),
            "an intact foreign queue keeps ownership"
        );
        assert_eq!(
            monitor.adopt_terminal_queue("old-host", "2026-10-25T00:00:00Z"),
            3
        );
        let mut adopted = monitor.prefs();
        reconcile(&mut adopted, &BTreeMap::new(), &BTreeMap::new(), &host).unwrap();
        assert!(
            adopted
                .agent_issue_schedule
                .reservations
                .iter()
                .all(|entry| entry.host_id == host),
            "canonical queue adoption leaves no foreign reservation behind"
        );
        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), adopted);
        assert!(restored.agent_schedule_launch_exclusion(41).is_none());
    }
    #[test]
    fn agent_schedule_new_plan_uses_idle_member_when_future_load_is_equal() {
        let (_temp, _home, root, sessions, roster) = fixture();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs
                .terminal_queues
                .get_mut(&crate::process::current_hostname())
                .unwrap()
                .entries
                .retain(|entry| entry.number == 41);
            Ok(())
        })
        .unwrap();
        crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "busy-first",
            Some(99),
            None,
            None,
            Some(&roster.members[0].agent_member_id),
        )
        .unwrap();
        let planned =
            mutate_project_agent_schedule(&root, &sessions, AgentScheduleAction::Read).unwrap();
        assert!(planned.members[0].issues.is_empty(), "a new plan must not wait on the busy first member while an equally loaded member is idle");
        assert_eq!(
            planned.members[1].issues,
            vec![AgentScheduledIssue {
                issue_number: 41,
                reason: AgentScheduleReason::QueueOrder
            }]
        );
    }
}
