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
    pub fn effective_launch_tiers(&self) -> Vec<Vec<IssueMonitorLaunchProfile>> {
        if !self.launch_auto {
            vec![self.launch_profile_pool()]
        } else if self.launch_tiers.is_empty() {
            default_launch_tiers()
        } else {
            self.launch_tiers.clone()
        }
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
                    available.is_none_or(|agents| agents.iter().any(|id| id == &profile.agent_id))
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
