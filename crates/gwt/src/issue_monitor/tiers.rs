use super::*;

/// Issue-scoped tier history survives attempt resets and closed/reopened work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorTierRecord {
    #[serde(default)]
    pub floor: u8,
    pub launch_tier: Option<u8>,
    pub landing_tier: Option<u8>,
    #[serde(default)]
    pub unknown_failures: u64,
}

impl IssueMonitorTierRecord {
    pub(super) fn merge(&mut self, other: &Self) {
        self.floor = self.floor.max(other.floor);
        self.launch_tier = self.launch_tier.max(other.launch_tier);
        self.landing_tier = self.landing_tier.max(other.landing_tier);
        self.unknown_failures = self.unknown_failures.max(other.unknown_failures);
    }
}

/// Actual Session or prospective cached selection profile, never a global guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorTierProfileProjection {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Current eligible automatic selection, subject to launch-time checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorTierSelectionProjection {
    pub tier: u8,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Exact acknowledged launch evidence, not proof of current process liveness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorConfirmedLaunchTier {
    pub tier: u8,
    pub window_id: String,
    pub delivery_id: String,
    /// Filled only by the runtime's exact acknowledged-window Session join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<IssueMonitorTierProfileProjection>,
}

/// Read-only tier facts for presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorTierFacts {
    pub issue_number: u64,
    /// Kind observed in the inbox or current scan, independent of admission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_spec: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_launch: Option<IssueMonitorConfirmedLaunchTier>,
    /// Max-merged history written before spawn, not the latest or running tier
    /// and not proof that an agent successfully started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_launch_tier: Option<u8>,
    /// Recorded exact Work-done observation, not a GitHub merge observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_landing_tier: Option<u8>,
    pub history_floor: u8,
    /// Explicit operator minimum, including zero; never an exact profile lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_tier_override: Option<u8>,
    /// Only classified agent failures, excluding infrastructure and unknown.
    pub agent_failures: u32,
    /// Auto-selection starting floor; availability can select a higher tier.
    /// Absent for manual profiles or when the Issue's spec status is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_minimum_tier: Option<u8>,
    /// Effective automatic tier count; absent for manual profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_count: Option<usize>,
    /// Filled by the runtime using the existing cached launch selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_launch: Option<IssueMonitorTierSelectionProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_launch_unavailable_reason: Option<String>,
}

impl IssueMonitorState {
    pub(super) fn tier_facts(&self, auto_tier_count: Option<usize>) -> Vec<IssueMonitorTierFacts> {
        let spec_by_issue = self
            .dependency_facts
            .iter()
            .map(|(&number, facts)| (number, facts.is_spec))
            .chain(
                self.inbox
                    .iter()
                    .map(|item| (item.issue.number, has_gwt_spec_label(&item.issue.labels))),
            )
            .collect::<BTreeMap<_, _>>();
        self.issue_tiers
            .keys()
            .chain(self.tier_overrides.keys())
            .chain(self.autonomous_records.keys())
            .chain(self.launch_confirmations.keys())
            .chain(self.dependency_facts.keys())
            .chain(self.inbox.iter().map(|item| &item.issue.number))
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|issue_number| {
                let history = self.issue_tiers.get(&issue_number);
                let history_floor = history.map_or(0, |record| record.floor);
                let minimum_tier_override = self.tier_overrides.get(&issue_number).copied();
                let agent_failures = self
                    .autonomous_records
                    .get(&issue_number)
                    .map_or(0, AutonomousIssueRecord::tier_input);
                let is_spec = spec_by_issue.get(&issue_number).copied();
                let next_minimum_tier = auto_tier_count.and_then(|count| {
                    is_spec.map(|is_spec| {
                        tier_for(
                            agent_failures,
                            is_spec,
                            history_floor.max(minimum_tier_override.unwrap_or(0)),
                            count.min(255) as u8,
                        )
                    })
                });
                let next_launch_unavailable_reason = Some(
                    if auto_tier_count.is_none() {
                        "Automatic tier selection is disabled"
                    } else if is_spec.is_none() {
                        "Issue kind is unavailable for automatic tier selection"
                    } else {
                        "Next launch profile awaits cached provider availability"
                    }
                    .to_string(),
                );
                IssueMonitorTierFacts {
                    issue_number,
                    is_spec,
                    confirmed_launch: self.confirmed_launch_tier(issue_number),
                    recorded_launch_tier: history.and_then(|record| record.launch_tier),
                    recorded_landing_tier: history.and_then(|record| record.landing_tier),
                    history_floor,
                    minimum_tier_override,
                    agent_failures,
                    next_minimum_tier,
                    tier_count: auto_tier_count,
                    next_launch: None,
                    next_launch_unavailable_reason,
                }
            })
            .collect()
    }

    fn confirmed_launch_tier(&self, issue_number: u64) -> Option<IssueMonitorConfirmedLaunchTier> {
        if !self.active_launches.contains(&issue_number)
            || self
                .pending_launch_deliveries
                .iter()
                .any(|delivery| delivery.issue_number == issue_number)
        {
            return None;
        }
        let window_id = self.launched_windows.get(&issue_number)?;
        let ack = self.launch_confirmations.get(&issue_number)?;
        if ack.window_id != *window_id
            || self.launch_bindings.get(window_id) != Some(&issue_number)
            || ack.claim_id.is_none()
            || ack.claim_id.as_ref() != self.launched_claims.get(&issue_number)
        {
            return None;
        }
        Some(IssueMonitorConfirmedLaunchTier {
            tier: ack.tier?,
            window_id: window_id.clone(),
            delivery_id: ack.delivery_id.clone()?,
            profile: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct IssueMonitorTierSelection {
    pub tier: u8,
    pub profile: IssueMonitorLaunchProfile,
    pub skipped: Vec<LaunchProfileSkip>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IssueMonitorTierLandingStats {
    pub landed_issues: usize,
    pub lowest_tier_landed_issues: usize,
    /// No observations is distinct from a measured zero-percent success rate.
    pub lowest_tier_landing_rate: Option<f64>,
    pub unknown_failures: u64,
}

fn default_launch_tiers() -> Vec<Vec<IssueMonitorLaunchProfile>> {
    [
        ("gpt-6-luna", "haiku", "low"),
        ("gpt-6-sol", "sonnet", "medium"),
        ("gpt-6-astra", "opus", "high"),
    ]
    .into_iter()
    .map(|(codex, claude, effort)| {
        [("codex", codex), ("claude", claude)]
            .into_iter()
            .map(|(agent_id, model)| IssueMonitorLaunchProfile {
                agent_member_id: None,
                agent_id: agent_id.to_string(),
                model: Some(model.to_string()),
                reasoning: Some(effort.to_string()),
                version: None,
                session_mode: Default::default(),
                skip_permissions: true,
                fast_mode: false,
                runtime_target: Default::default(),
                docker_service: None,
                docker_lifecycle_intent: Default::default(),
                windows_shell: None,
                prefer_for: Vec::new(),
            })
            .collect()
    })
    .collect()
}

impl IssueMonitorPrefs {
    /// Apply one Issue override under the caller's latest-prefs writer lock.
    /// Keep configuration CAS, revocation, and monotonic override semantics
    /// identical across CLI, daemon, and GUI fallback transports.
    pub fn apply_issue_minimum_launch_tier(
        &mut self,
        number: u64,
        tier: u8,
        expected_config_token: Option<&str>,
    ) -> io::Result<()> {
        check_expected_issue_monitor_config_token(self, expected_config_token)?;
        if usize::from(tier) >= self.effective_launch_tiers().len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tier is outside the configured launch tiers",
            ));
        }
        self.advance_effect_authority_epoch()
            .ok_or_else(|| io::Error::other("Issue Monitor authority epoch overflow"))?;
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), self.clone());
        monitor.set_issue_minimum_launch_tier(number, tier);
        *self = monitor.prefs();
        Ok(())
    }

    /// Bind selection to the claimed fresh launch before materialization.
    pub fn record_launch_delivery_tier(
        &mut self,
        issue_number: u64,
        delivery_id: &str,
        materializer_id: &str,
        window_id: &str,
        tier: u8,
    ) -> bool {
        let Some(delivery) = self.pending_launch_deliveries.iter_mut().find(|delivery| {
            delivery.issue_number == issue_number
                && delivery.delivery_id == delivery_id
                && delivery.materializer_id.as_deref() == Some(materializer_id)
                && delivery.materializer_window_id.as_deref() == Some(window_id)
                && delivery.materialized_window_id.is_none()
                && delivery.workspace_durable_window_id.is_none()
        }) else {
            return false;
        };
        delivery.selected_tier = Some(tier);
        true
    }

    pub fn effective_launch_tiers(&self) -> Vec<Vec<IssueMonitorLaunchProfile>> {
        if !self.launch_auto {
            return vec![self.launch_profile_pool()];
        }
        let tiers = if self.launch_tiers.is_empty() {
            default_launch_tiers()
        } else {
            self.launch_tiers.clone()
        };
        let selected = self.launch_profile_pool();
        let Some(primary) = selected
            .first()
            .filter(|profile| profile.agent_member_id.is_some())
        else {
            return tiers;
        };
        let tool = normalize_issue_monitor_provider(&primary.agent_id);
        tiers
            .into_iter()
            .map(|tier| {
                tier.into_iter()
                    .filter(|profile| normalize_issue_monitor_provider(&profile.agent_id) == tool)
                    .flat_map(|profile| {
                        selected
                            .iter()
                            .filter(|member| {
                                normalize_issue_monitor_provider(&member.agent_id) == tool
                            })
                            .map(move |member| {
                                let mut candidate = profile.clone();
                                candidate.agent_member_id = member.agent_member_id.clone();
                                candidate
                            })
                    })
                    .collect()
            })
            .collect()
    }

    /// Wrap the existing candidate selector without changing its policy.
    /// Availability is supplied by the caller's existing detection cache.
    pub fn select_auto_launch_profile(
        &self,
        issue_number: u64,
        is_spec: bool,
        available: Option<&[String]>,
        select: impl Fn(&[IssueMonitorLaunchProfile]) -> LaunchProfileSelection,
    ) -> Option<IssueMonitorTierSelection> {
        let tiers = self.effective_launch_tiers();
        let selected = self.registered_launch_profiles_for_issue(issue_number);
        let bound = self
            .launch_profile_pool()
            .first()
            .is_some_and(|profile| profile.agent_member_id.is_some());
        let tier_input = self
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == issue_number)
            .map_or(0, AutonomousIssueRecord::tier_input);
        let floor = self
            .issue_tiers
            .get(&issue_number)
            .map_or(0, |record| record.floor)
            .max(self.tier_overrides.get(&issue_number).copied().unwrap_or(0));
        let start = tier_for(tier_input, is_spec, floor, tiers.len().min(255) as u8);
        let mut skipped = Vec::new();
        for (index, pool) in tiers.into_iter().enumerate().skip(usize::from(start)) {
            let pool: Vec<_> = pool
                .into_iter()
                .filter(|profile| {
                    (!bound
                        || selected
                            .iter()
                            .any(|selected| selected.agent_member_id == profile.agent_member_id))
                        && available
                            .is_none_or(|agents| agents.iter().any(|id| id == &profile.agent_id))
                })
                .collect();
            let selection = select(&pool);
            skipped.extend(selection.skipped);
            if let Some(profile) = selection.selected.and_then(|index| pool.get(index)) {
                return Some(IssueMonitorTierSelection {
                    tier: index as u8,
                    profile: profile.clone(),
                    skipped,
                });
            }
        }
        None
    }

    pub fn record_tier_launch(&mut self, issue_number: u64, tier: u8) {
        let record = self.issue_tiers.entry(issue_number).or_default();
        record.floor = record.floor.max(tier);
        record.launch_tier = Some(tier);
    }

    pub fn tier_landing_stats(&self) -> IssueMonitorTierLandingStats {
        let landed_issues = self
            .issue_tiers
            .values()
            .filter(|record| record.landing_tier.is_some())
            .count();
        let lowest_tier_landed_issues = self
            .issue_tiers
            .values()
            .filter(|record| record.landing_tier == Some(0))
            .count();
        IssueMonitorTierLandingStats {
            landed_issues,
            lowest_tier_landed_issues,
            lowest_tier_landing_rate: (landed_issues > 0)
                .then(|| lowest_tier_landed_issues as f64 / landed_issues as f64),
            unknown_failures: self.issue_tiers.values().fold(0u64, |sum, record| {
                sum.saturating_add(record.unknown_failures)
            }),
        }
    }
}

/// Called only after the authoritative Work mutation has accepted `done`.
pub(crate) fn record_work_done_tier_landing(
    target: &crate::agent_project_state::SessionWorkMutationTarget,
) -> io::Result<()> {
    use crate::cli::execution_state::ExecutionOwnerKind;
    let Some(execution) = crate::cli::execution_state::load(&target.work_event_root)
        .ok()
        .flatten()
    else {
        return Ok(());
    };
    let owner = match execution.owner_kind {
        ExecutionOwnerKind::Issue => format!("Issue #{}", execution.owner_number),
        ExecutionOwnerKind::Spec => format!("SPEC-{}", execution.owner_number),
    };
    if execution.primary_session_id != target.session_id
        || target.owner.as_deref() != Some(owner.as_str())
    {
        return Ok(());
    }
    let path = issue_monitor_prefs_path_for_repo_path(&target.work_event_root);
    let prefs = load_issue_monitor_prefs(&path)?;
    if !prefs.launch_auto
        || prefs
            .issue_tiers
            .get(&execution.owner_number)
            .and_then(|record| record.launch_tier)
            .is_none()
    {
        return Ok(());
    }
    try_mutate_issue_monitor_prefs(&path, |prefs| {
        if prefs.launch_auto {
            if let Some(record) = prefs.issue_tiers.get_mut(&execution.owner_number) {
                record.landing_tier = record.landing_tier.max(record.launch_tier);
            }
        }
        Ok(())
    })?;
    Ok(())
}
