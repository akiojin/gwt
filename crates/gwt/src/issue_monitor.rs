use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::{self, Write},
    path::Path,
};

use crate::autonomous_handoff::{
    parse_protected_autonomous_handoff_answer_prompt, protected_autonomous_handoff_answer_prompt,
    AutonomousHandoffDeliveryState, AutonomousHandoffDeliveryTarget,
    AutonomousHandoffReceiptIdentity, AutonomousHandoffState, AutonomousQuestionHandoff,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use gwt_github::{
    issue_auto_claim::{acquire_claim, ClaimAcquireOutcome, ClaimComment, ClaimStatus},
    IssueClient, IssueNumber,
};

use crate::{
    has_gwt_spec_label, knowledge_launch_target_branch_name, LaunchWizardPreviousProfile,
    LinkedIssueKind,
};

const GITHUB_AUTH_SETUP_MESSAGE: &str = concat!(
    "GitHub authentication is required before automatic Issue Monitor launches can claim Issues. ",
    "Configure it on the host terminal with: ",
    "gh auth login --hostname github.com --git-protocol https --scopes repo,read:org; ",
    "gh auth setup-git. ",
    "Then verify: gh auth status --hostname github.com; git ls-remote origin HEAD. ",
    "gwt does not store GitHub credentials; it uses the host gh/Git credential setup."
);

const GIT_HTTPS_AUTH_SETUP_PREFIX: &str = concat!(
    "Git HTTPS credentials are required before Issue Monitor can create work branches. ",
    "Configure the host terminal with: ",
    "gh auth login --hostname github.com --git-protocol https --scopes repo,read:org; ",
    "gh auth setup-git. ",
    "Then verify: git ls-remote origin HEAD."
);

/// Project-scoped schema marker for the one-shot recovery of the exact launch
/// failure persisted before Issue #3272 was fixed. Missing serde fields remain
/// version 0; fresh projects start at this current version and never replay a
/// historical migration.
pub const LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION: u32 = 1;
pub const ISSUE_COMPLETION_MIGRATION_VERSION: u32 = 1;
const LEGACY_ISSUE_MONITOR_AUTHORITY_FENCE_VERSION: u32 = 1;
const ISSUE_MONITOR_AUTHORITY_FENCE_VERSION: u32 = 2;
const LEGACY_SHUTDOWN_REVOKE_FENCE: &[u8] = b"gwt issue-monitor shutdown revoke v1\n";

const LEGACY_GIT_LAUNCH_FAILURE_PREFIX: &str =
    "Current branch is unavailable: Git error: Not a git repository: ";

fn normalize_issue_monitor_provider(raw: &str) -> Option<String> {
    gwt_agent::resolve_agent_id(raw).map(|id| id.command().to_ascii_lowercase())
}

fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn format_rfc3339_utc(value: chrono::DateTime<chrono::Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn merge_provider_quota_hold(
    holds: &mut BTreeMap<String, String>,
    provider: &str,
    reset_at: &str,
) -> Option<String> {
    let provider = normalize_issue_monitor_provider(provider)?;
    let incoming = parse_rfc3339_utc(reset_at)?;
    let joined = holds
        .get(&provider)
        .and_then(|current| parse_rfc3339_utc(current))
        .map_or(incoming, |current| current.max(incoming));
    let reset_at = format_rfc3339_utc(joined);
    holds.insert(provider, reset_at.clone());
    Some(reset_at)
}

fn normalize_provider_quota_holds(holds: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut normalized = BTreeMap::new();
    for (provider, reset_at) in holds {
        merge_provider_quota_hold(&mut normalized, provider, reset_at);
    }
    normalized
}

fn normalize_provider_keyed<T: Clone>(map: &BTreeMap<String, T>) -> BTreeMap<String, T> {
    map.iter()
        .filter_map(|(provider, value)| {
            normalize_issue_monitor_provider(provider).map(|provider| (provider, value.clone()))
        })
        .collect()
}

fn concrete_provider_quota_deadline(resets_at: Option<&str>, now: &str) -> String {
    let now = parse_rfc3339_utc(now).unwrap_or_else(chrono::Utc::now);
    let deadline = resets_at
        .and_then(parse_rfc3339_utc)
        .filter(|reset| *reset > now)
        .unwrap_or_else(|| now + chrono::Duration::seconds(60));
    format_rfc3339_utc(deadline)
}

pub fn github_auth_setup_message() -> &'static str {
    GITHUB_AUTH_SETUP_MESSAGE
}

pub fn is_git_https_auth_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("could not read username for 'https://github.com'")
        || lower.contains("could not read username for \"https://github.com")
        || (lower.contains("terminal prompts disabled") && lower.contains("github.com"))
}

pub fn git_https_auth_setup_message(original_error: &str) -> String {
    format!(
        "{GIT_HTTPS_AUTH_SETUP_PREFIX} Original error: {}",
        original_error.trim()
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorConfig {
    pub enabled: bool,
    pub poll_interval_secs: u64,
    pub claim_heartbeat_secs: u64,
    pub claim_ttl_secs: u64,
    pub max_active: usize,
    pub queue_when_gui_absent: bool,
}

/// SPEC #3200 FR-030: tunable bounds for autonomous (unattended) operation.
/// Every field has a documented default so older prefs deserialize cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousTuning {
    /// Max failed attempts per issue before escalating to `NeedsHuman`
    /// (FR-021). Bounds the auto-relaunch / Deliver-fix loop.
    pub max_attempts: u32,
    /// An active agent with no liveness progress for this long is considered
    /// stuck; its active slot is recovered (FR-025).
    pub stuck_timeout_secs: u64,
    /// Heartbeat freshness window used by stuck/idle detection (FR-025).
    pub heartbeat_interval_secs: u64,
    /// Max time to watch a PR toward merge before treating it as stuck
    /// (FR-018 merge-watch).
    pub merge_watch_timeout_secs: u64,
    /// Max Deliver Fix-loop iterations within one attempt before the attempt
    /// counts as a failure.
    pub deliver_fix_loop_cap: u32,
    /// Base backoff seconds for transient-failure retry (FR-022/FR-024).
    pub retry_backoff_base_secs: u64,
    /// Upper bound for the (exponential) retry backoff.
    pub retry_backoff_cap_secs: u64,
    /// SPEC #3200 FR-015: the model the INDEPENDENT review agent runs on. When
    /// set (and different from the implementer's model) the review is forced onto
    /// it so the verdict is not a self-grade. `None` falls back to the saved
    /// launch profile's model (still a fresh, adversarial session).
    #[serde(default)]
    pub review_model: Option<String>,
}

impl Default for AutonomousTuning {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            stuck_timeout_secs: 1800,
            heartbeat_interval_secs: 120,
            merge_watch_timeout_secs: 3600,
            deliver_fix_loop_cap: 5,
            retry_backoff_base_secs: 60,
            retry_backoff_cap_secs: 1800,
            review_model: None,
        }
    }
}

/// SPEC #3200 FR-015: pick the model an independent review should run on, given
/// the implementer's model and the configured `review_model`. Returns the
/// configured model only when it is set AND genuinely different from the
/// implementer's (avoids a self-grade); otherwise `None` (caller keeps the saved
/// profile model — still a fresh adversarial session).
pub fn resolve_review_model(
    implementer_model: Option<&str>,
    configured_review_model: Option<&str>,
) -> Option<String> {
    let configured = configured_review_model?.trim();
    if configured.is_empty() {
        return None;
    }
    match implementer_model {
        Some(impl_model) if impl_model.eq_ignore_ascii_case(configured) => None,
        _ => Some(configured.to_string()),
    }
}

/// Durable delivery state for an Issue Monitor side effect. `Prepared` is
/// guaranteed not to have been submitted remotely; `Attempting` crosses the
/// submission boundary and therefore requires read-back or compensation after
/// an ambiguous result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorEffectState {
    Prepared,
    Attempting,
}

/// The remote mutation described by one durable Issue Monitor journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueMonitorEffectPayload {
    AcquireClaim {
        issue_number: u64,
        claim_id: String,
        owner: String,
        heartbeat_at: String,
        expires_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launched_work_id: Option<String>,
    },
    ReleaseClaim {
        issue_number: u64,
        claim_id: String,
        #[serde(default)]
        owner: String,
    },
    ArmAutoMerge {
        issue_number: u64,
        pr_number: u64,
        reviewed_sha: String,
    },
    DisarmAutoMerge {
        issue_number: u64,
        pr_number: u64,
        compensates_effect_id: String,
    },
}

/// Stable identity of one delivery attempt. Results are accepted only when all
/// three fields still match the journal entry that crossed the execution fence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorEffectAttemptKey {
    pub effect_id: String,
    pub authority_epoch: u64,
    pub attempt: u32,
}

/// One durable side-effect proposal or in-flight delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingIssueMonitorEffect {
    pub effect_id: String,
    pub authority_epoch: u64,
    pub attempt: u32,
    pub state: IssueMonitorEffectState,
    pub payload: IssueMonitorEffectPayload,
}

impl PendingIssueMonitorEffect {
    pub fn prepared(
        effect_id: impl Into<String>,
        authority_epoch: u64,
        payload: IssueMonitorEffectPayload,
    ) -> Self {
        Self {
            effect_id: effect_id.into(),
            authority_epoch,
            attempt: 0,
            state: IssueMonitorEffectState::Prepared,
            payload,
        }
    }

    pub fn attempt_key(&self) -> IssueMonitorEffectAttemptKey {
        IssueMonitorEffectAttemptKey {
            effect_id: self.effect_id.clone(),
            authority_epoch: self.authority_epoch,
            attempt: self.attempt,
        }
    }
}

fn effect_matches_key(
    effect: &PendingIssueMonitorEffect,
    key: &IssueMonitorEffectAttemptKey,
) -> bool {
    effect.effect_id == key.effect_id
        && effect.authority_epoch == key.authority_epoch
        && effect.attempt == key.attempt
}

fn mark_effect_attempting(
    pending_effects: &mut [PendingIssueMonitorEffect],
    key: &IssueMonitorEffectAttemptKey,
) -> bool {
    let Some(effect) = pending_effects
        .iter_mut()
        .find(|effect| effect_matches_key(effect, key))
    else {
        return false;
    };
    if effect.state != IssueMonitorEffectState::Prepared {
        return false;
    }
    effect.state = IssueMonitorEffectState::Attempting;
    true
}

fn complete_attempting_effect(
    pending_effects: &mut Vec<PendingIssueMonitorEffect>,
    key: &IssueMonitorEffectAttemptKey,
) -> Option<PendingIssueMonitorEffect> {
    let index = pending_effects.iter().position(|effect| {
        effect.state == IssueMonitorEffectState::Attempting && effect_matches_key(effect, key)
    })?;
    Some(pending_effects.remove(index))
}

fn ensure_claim_release_effect(
    pending_effects: &mut Vec<PendingIssueMonitorEffect>,
    authority_epoch: u64,
    source_effect_id: &str,
    issue_number: u64,
    claim_id: &str,
    owner: &str,
) {
    if pending_effects.iter().any(|effect| {
        matches!(
            &effect.payload,
            IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: pending_issue,
                claim_id: pending_claim,
                owner: pending_owner,
            } if *pending_issue == issue_number
                && pending_claim == claim_id
                && pending_owner == owner
        )
    }) {
        return;
    }
    pending_effects.push(PendingIssueMonitorEffect::prepared(
        format!("release:{source_effect_id}:ineligible:{authority_epoch}"),
        authority_epoch,
        IssueMonitorEffectPayload::ReleaseClaim {
            issue_number,
            claim_id: claim_id.to_string(),
            owner: owner.to_string(),
        },
    ));
}

fn ensure_auto_merge_disarm_effect(
    pending_effects: &mut Vec<PendingIssueMonitorEffect>,
    authority_epoch: u64,
    source_effect_id: &str,
    issue_number: u64,
    pr_number: u64,
) {
    if pending_effects.iter().any(|effect| {
        matches!(
            &effect.payload,
            IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: pending_issue,
                pr_number: pending_pr,
                compensates_effect_id,
            } if (*pending_issue == issue_number && *pending_pr == pr_number)
                || compensates_effect_id == source_effect_id
        )
    }) {
        return;
    }
    pending_effects.push(PendingIssueMonitorEffect::prepared(
        format!("disarm:{source_effect_id}:{authority_epoch}"),
        authority_epoch,
        IssueMonitorEffectPayload::DisarmAutoMerge {
            issue_number,
            pr_number,
            compensates_effect_id: source_effect_id.to_string(),
        },
    ));
}

fn revoke_uncommitted_effects_for_closed_issue(
    pending_effects: &mut Vec<PendingIssueMonitorEffect>,
    authority_epoch: u64,
    issue_number: u64,
) {
    let attempting = pending_effects
        .iter()
        .filter(|effect| {
            effect.state == IssueMonitorEffectState::Attempting
                && matches!(
                    effect.payload,
                    IssueMonitorEffectPayload::AcquireClaim {
                        issue_number: pending_issue,
                        ..
                    } | IssueMonitorEffectPayload::ArmAutoMerge {
                        issue_number: pending_issue,
                        ..
                    } if pending_issue == issue_number
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    pending_effects.retain(|effect| {
        !matches!(
            effect.payload,
            IssueMonitorEffectPayload::AcquireClaim {
                issue_number: pending_issue,
                ..
            } | IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: pending_issue,
                ..
            } if pending_issue == issue_number
        )
    });
    for effect in attempting {
        match &effect.payload {
            IssueMonitorEffectPayload::AcquireClaim {
                claim_id, owner, ..
            } => ensure_claim_release_effect(
                pending_effects,
                authority_epoch,
                &effect.effect_id,
                issue_number,
                claim_id,
                owner,
            ),
            IssueMonitorEffectPayload::ArmAutoMerge { pr_number, .. } => {
                ensure_auto_merge_disarm_effect(
                    pending_effects,
                    authority_epoch,
                    &effect.effect_id,
                    issue_number,
                    *pr_number,
                );
            }
            IssueMonitorEffectPayload::ReleaseClaim { .. }
            | IssueMonitorEffectPayload::DisarmAutoMerge { .. } => {}
        }
    }
}

fn revoke_uncommitted_claims_for_issue(
    pending_effects: &mut Vec<PendingIssueMonitorEffect>,
    authority_epoch: u64,
    issue_number: u64,
) {
    let attempting = pending_effects
        .iter()
        .filter(|effect| {
            effect.state == IssueMonitorEffectState::Attempting
                && matches!(
                    effect.payload,
                    IssueMonitorEffectPayload::AcquireClaim {
                        issue_number: pending_issue,
                        ..
                    } if pending_issue == issue_number
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    pending_effects.retain(|effect| {
        effect.state != IssueMonitorEffectState::Prepared
            || !matches!(
                effect.payload,
                IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: pending_issue,
                    ..
                } if pending_issue == issue_number
            )
    });
    for effect in attempting {
        if let IssueMonitorEffectPayload::AcquireClaim {
            claim_id, owner, ..
        } = &effect.payload
        {
            ensure_claim_release_effect(
                pending_effects,
                authority_epoch,
                &effect.effect_id,
                issue_number,
                claim_id,
                owner,
            );
        }
    }
}

fn advance_autonomous_effect_authority(
    autonomous_mode: &mut bool,
    effect_authority_epoch: &mut u64,
    pending_effects: &mut Vec<PendingIssueMonitorEffect>,
    enabled: bool,
) -> Option<u64> {
    if *autonomous_mode == enabled {
        return Some(*effect_authority_epoch);
    }
    let next_epoch = advance_effect_authority(effect_authority_epoch, pending_effects)?;
    *autonomous_mode = enabled;
    Some(next_epoch)
}

fn advance_effect_authority(
    effect_authority_epoch: &mut u64,
    pending_effects: &mut Vec<PendingIssueMonitorEffect>,
) -> Option<u64> {
    let next_epoch = effect_authority_epoch.checked_add(1)?;
    let mut next_effects = pending_effects.clone();
    let attempting = next_effects
        .iter()
        .filter(|effect| {
            effect.state == IssueMonitorEffectState::Attempting
                && matches!(
                    effect.payload,
                    IssueMonitorEffectPayload::AcquireClaim { .. }
                        | IssueMonitorEffectPayload::ArmAutoMerge { .. }
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    next_effects.retain(|effect| {
        effect.state != IssueMonitorEffectState::Prepared
            || !matches!(
                effect.payload,
                IssueMonitorEffectPayload::AcquireClaim { .. }
                    | IssueMonitorEffectPayload::ArmAutoMerge { .. }
            )
    });

    for effect in attempting {
        let (effect_id, payload, already_compensated) = match &effect.payload {
            IssueMonitorEffectPayload::AcquireClaim {
                issue_number,
                claim_id,
                owner,
                ..
            } => {
                let already = next_effects.iter().any(|pending| {
                    matches!(
                        &pending.payload,
                        IssueMonitorEffectPayload::ReleaseClaim {
                            issue_number: pending_issue,
                            claim_id: pending_claim,
                            owner: pending_owner,
                        } if pending_issue == issue_number
                            && pending_claim == claim_id
                            && pending_owner == owner
                    )
                });
                (
                    format!("release:{}:{next_epoch}", effect.effect_id),
                    IssueMonitorEffectPayload::ReleaseClaim {
                        issue_number: *issue_number,
                        claim_id: claim_id.clone(),
                        owner: owner.clone(),
                    },
                    already,
                )
            }
            IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number,
                pr_number,
                ..
            } => {
                let already = next_effects.iter().any(|pending| {
                    matches!(
                        &pending.payload,
                        IssueMonitorEffectPayload::DisarmAutoMerge {
                            compensates_effect_id,
                            ..
                        } if compensates_effect_id == &effect.effect_id
                    )
                });
                (
                    format!("disarm:{}:{next_epoch}", effect.effect_id),
                    IssueMonitorEffectPayload::DisarmAutoMerge {
                        issue_number: *issue_number,
                        pr_number: *pr_number,
                        compensates_effect_id: effect.effect_id.clone(),
                    },
                    already,
                )
            }
            IssueMonitorEffectPayload::ReleaseClaim { .. }
            | IssueMonitorEffectPayload::DisarmAutoMerge { .. } => continue,
        };
        if !already_compensated {
            next_effects.push(PendingIssueMonitorEffect::prepared(
                effect_id, next_epoch, payload,
            ));
        }
    }

    *effect_authority_epoch = next_epoch;
    *pending_effects = next_effects;
    Some(next_epoch)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorControlReceipt {
    pub control_id: String,
    pub should_scan: bool,
    #[serde(default)]
    pub authority_changed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCompletionState {
    #[default]
    Completed,
    Reopened,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCompletionEvidence {
    #[default]
    Legacy,
    LinkedPr,
    WorkBranch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueCompletionDecision {
    Complete,
    Incomplete,
    Unknown,
}

/// Durable Issue-wide completion state. `generation` is local to one Issue;
/// higher generations win cross-process rebase, and a same-generation
/// `Reopened` record wins fail-closed over `Completed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCompletionRecord {
    pub issue_number: u64,
    pub generation: u64,
    pub state: IssueCompletionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_updated_at: Option<String>,
    #[serde(default)]
    pub evidence: IssueCompletionEvidence,
}

impl IssueCompletionRecord {
    fn legacy(issue_number: u64) -> Self {
        Self {
            issue_number,
            generation: 0,
            state: IssueCompletionState::Completed,
            issue_updated_at: None,
            evidence: IssueCompletionEvidence::Legacy,
        }
    }
}

/// GitHub lifecycle fact used to converge current-action monitor state across
/// restarts and stale writers. Explicit facts compare exact GitHub revisions;
/// a non-explicit Closed fact may carry only a lower-bound revision floor.
/// Ambiguous cross-process ordering falls back to generation and a `Closed`
/// tie-break, preventing cross-clock comparisons and stale resurrection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueClosureState {
    #[default]
    Closed,
    Reopened,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueClosureEvidence {
    /// An explicit GitHub Issue row whose `updated_at` is a comparable GitHub
    /// revision. This is the serde default for pre-field closure records.
    #[default]
    ExplicitRevision,
    /// Absence from a complete Live open-Issue snapshot. Its observation clock
    /// is local and must never be compared with GitHub `updated_at`.
    CompleteLiveAbsence,
    /// A trusted local release transition with no GitHub revision attached.
    DirectRelease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueClosureRecord {
    pub issue_number: u64,
    pub generation: u64,
    pub state: IssueClosureState,
    #[serde(default)]
    pub evidence: IssueClosureEvidence,
    /// Highest valid GitHub `updated_at` revision known for this lifecycle
    /// lineage. Absence/direct facts carry the floor forward without treating
    /// their local observation clock as a GitHub revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorPrefs {
    pub enabled: bool,
    pub max_active_agents: usize,
    pub priority_order: Vec<u64>,
    /// One-shot, project-scoped migration marker. The serde default is
    /// intentionally the numeric default (0) for pre-migration JSON, while
    /// [`Default`] uses the current version for genuinely fresh projects.
    #[serde(default)]
    pub legacy_git_launch_failure_migration_version: u32,
    #[serde(default)]
    pub launch_profile: Option<IssueMonitorLaunchProfile>,
    /// Issue #3785 / SPEC #3165: provider-wide launch admission holds keyed by
    /// the canonical agent command. Values are concrete RFC3339 reset
    /// deadlines; per-provider rebases join by the later instant.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_quota_holds: BTreeMap<String, String>,
    /// Issue #3923 AC-2: what each hold in `provider_quota_holds` was formed
    /// from, keyed the same way.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_quota_hold_evidence: BTreeMap<String, IssueMonitorProviderQuotaHoldEvidence>,
    /// Issue #3923 AC-1: operator releases that fence stale holds across every
    /// process. See [`IssueMonitorProviderQuotaHoldRelease`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_quota_hold_releases: BTreeMap<String, IssueMonitorProviderQuotaHoldRelease>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launched_issues: Vec<IssueMonitorLaunchedIssue>,
    /// Durable launch generation keyed by Issue number. Kept separate from the
    /// compatibility `launched_issues` rows so older struct consumers remain
    /// source-compatible while new readers can reject delayed window closes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub launched_claims: BTreeMap<u64, String>,
    /// Issue #3222: claims whose agent window is not bound yet (`Launching`).
    /// Persisted so an in-flight claim survives the per-handler prefs
    /// roundtrip — otherwise a rescan re-claims the same issue (same-owner
    /// renewal) and spawns a duplicate window past `max_active`.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_launching_issues"
    )]
    pub launching_issues: Vec<IssueMonitorLaunchingIssue>,
    /// SPEC #3200 FR-052: launch deliveries remain durable until the GUI
    /// returns the matching delivery id in a Launched/LaunchFailed ACK.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_launch_deliveries: Vec<PendingIssueMonitorLaunchDelivery>,
    /// SPEC #3165 FR-100/FR-101: one-shot strategy for an issue's next launch.
    /// The marker is removed only after it has been copied into a durable
    /// delivery (or the legacy direct request path), making failover/retry
    /// intent survive reload without leaking into later launches.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub queued_launch_session_strategies: BTreeMap<u64, IssueMonitorLaunchSessionStrategy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_issues: Vec<IssueMonitorFailedIssue>,
    /// Issue #3645 / #3628: failure holds an operator released.
    ///
    /// Removing an entry from `failed_issues` says nothing to a process that
    /// still holds the same failure in memory: the disk merge only ever *adds*
    /// disk failures, so the stale holder re-stamps the row on its next commit
    /// and the recovery silently un-happens. A release therefore has to be a
    /// published fact rather than the absence of one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub released_failures: Vec<IssueMonitorReleasedFailure>,
    /// Issue #3734: bounded durable history of explicit operator requeues.
    ///
    /// `released_failures` is active convergence state and disappears when an
    /// issue fails again. This separate history keeps the reset evidence while
    /// remaining bounded for long-running repositories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requeue_audit: Vec<IssueMonitorReleasedFailure>,
    /// Monotonic counter for the releases above. A process adopts only releases
    /// issued after the snapshot it already observed, so a release cannot erase
    /// a failure that was recorded later.
    #[serde(default)]
    pub failure_release_version: u64,
    /// Compatibility projection of Issue-wide completed records. Kept for old
    /// readers; `completion_records` is the current source of truth.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_issues: Vec<u64>,
    /// Versioned migration away from bare `merged_issues`. Fresh projects use
    /// the current version; pre-Phase-12 JSON deserializes as zero.
    #[serde(default)]
    pub issue_completion_migration_version: u32,
    /// Issue-generation-bound completion/reopen records. `merged_issues`
    /// remains a compatibility projection for older readers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion_records: Vec<IssueCompletionRecord>,
    /// Issue #3602: durable close/reopen facts fence current-action companions
    /// against resurrection by an older process. Additive + serde-defaulted so
    /// pre-#3602 preference files remain readable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closure_records: Vec<IssueClosureRecord>,
    /// SPEC #3200: opt-in autonomous (unattended) resolution mode. Default
    /// `false` preserves SPEC #3165 human-gated behavior exactly (FR-001).
    #[serde(default)]
    pub autonomous_mode: bool,
    /// SPEC #3200 FR-030: tunable bounds for unattended operation.
    #[serde(default)]
    pub autonomous_tuning: AutonomousTuning,
    /// SPEC #3200 T-016/T-022: per-issue autonomous state (attempt counter,
    /// phase, in-flight launch id, acceptance snapshot). Persisted so an
    /// in-flight attempt survives a daemon restart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub autonomous_records: Vec<AutonomousIssueRecord>,
    /// Monotonic authority generation for every remotely mutating effect.
    /// Missing values in pre-journal prefs intentionally deserialize as zero.
    #[serde(default)]
    pub effect_authority_epoch: u64,
    /// Durable side-effect journal. Empty is omitted to preserve the compact
    /// shape of existing prefs files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_effects: Vec<PendingIssueMonitorEffect>,
    /// Receipt for the most recently admitted daemon control. The control ID
    /// is generated per admission and reused only by that control's retry, so
    /// one slot is sufficient while the worker enforces a FIFO retry barrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_control_receipt: Option<IssueMonitorControlReceipt>,
    /// Issue #3478 (FR-025): durable question handoffs. Appended by the
    /// intercepting hook (any process) and driven through their lifecycle by
    /// the Issue Monitor driver. Empty is omitted so existing prefs files keep
    /// their compact shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub autonomous_handoffs: Vec<AutonomousQuestionHandoff>,
    /// Issue #3633 (AC-5): when a driver last completed a scan for this
    /// project.
    ///
    /// Persisted because the question "is anything scanning this project?"
    /// is asked precisely when no driver is running, and an in-memory
    /// timestamp cannot answer it. A reader with only the prefs file used to
    /// have nothing to compare against, which is how a monitor that had never
    /// scanned kept reporting a completely healthy snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan_at: Option<String>,
}

impl Default for IssueMonitorPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            max_active_agents: 1,
            priority_order: Vec::new(),
            legacy_git_launch_failure_migration_version:
                LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
            launch_profile: None,
            provider_quota_holds: BTreeMap::new(),
            provider_quota_hold_evidence: BTreeMap::new(),
            provider_quota_hold_releases: BTreeMap::new(),
            launched_issues: Vec::new(),
            launched_claims: BTreeMap::new(),
            launching_issues: Vec::new(),
            pending_launch_deliveries: Vec::new(),
            queued_launch_session_strategies: BTreeMap::new(),
            failed_issues: Vec::new(),
            released_failures: Vec::new(),
            requeue_audit: Vec::new(),
            failure_release_version: 0,
            merged_issues: Vec::new(),
            issue_completion_migration_version: ISSUE_COMPLETION_MIGRATION_VERSION,
            completion_records: Vec::new(),
            closure_records: Vec::new(),
            autonomous_mode: false,
            autonomous_tuning: AutonomousTuning::default(),
            autonomous_records: Vec::new(),
            effect_authority_epoch: 0,
            pending_effects: Vec::new(),
            last_control_receipt: None,
            autonomous_handoffs: Vec::new(),
            last_scan_at: None,
        }
    }
}

impl IssueMonitorPrefs {
    /// Fallback for an existing prefs file that could not be decoded and for
    /// which the caller has no valid in-memory snapshot. Unlike [`Default`],
    /// the compatibility migration remains unapplied until a complete live
    /// scan and the Launch Agent resolver both succeed.
    pub fn recovery_default() -> Self {
        Self {
            legacy_git_launch_failure_migration_version: 0,
            ..Self::default()
        }
    }

    /// Adopt another process's completed migration only when its marker is
    /// strictly newer. A launch already owned by this process remains
    /// authoritative over a stale disk failure, while an adopted failure
    /// cancels any claimed-but-unbound launch for the same issue. Equal/older
    /// snapshots cannot erase failures recorded later.
    pub fn adopt_newer_legacy_git_launch_failure_migration(
        &mut self,
        disk: &IssueMonitorPrefs,
    ) -> bool {
        if disk.legacy_git_launch_failure_migration_version
            <= self.legacy_git_launch_failure_migration_version
        {
            return false;
        }
        self.legacy_git_launch_failure_migration_version =
            disk.legacy_git_launch_failure_migration_version;
        let launched_issue_numbers = self
            .launched_issues
            .iter()
            .map(|launched| launched.issue_number)
            .collect::<BTreeSet<_>>();
        self.failed_issues = disk
            .failed_issues
            .iter()
            .filter(|failed| !launched_issue_numbers.contains(&failed.issue_number))
            .cloned()
            .collect();
        let adopted_failure_numbers = self
            .failed_issues
            .iter()
            .map(|failed| failed.issue_number)
            .collect::<BTreeSet<_>>();
        self.launching_issues
            .retain(|launching| !adopted_failure_numbers.contains(&launching.issue_number));
        self.pending_launch_deliveries
            .retain(|delivery| !adopted_failure_numbers.contains(&delivery.issue_number));
        self.queued_launch_session_strategies
            .retain(|issue_number, _| !adopted_failure_numbers.contains(issue_number));
        true
    }

    /// Commit the execution fence for an exact Prepared delivery tuple.
    pub fn mark_pending_effect_attempting(&mut self, key: &IssueMonitorEffectAttemptKey) -> bool {
        mark_effect_attempting(&mut self.pending_effects, key)
    }

    /// Remove an Attempting journal entry only when the completed delivery
    /// tuple is still exact.
    pub fn complete_pending_effect(
        &mut self,
        key: &IssueMonitorEffectAttemptKey,
    ) -> Option<PendingIssueMonitorEffect> {
        complete_attempting_effect(&mut self.pending_effects, key)
    }

    /// Return an exact Attempting tuple to Prepared under a fresh attempt
    /// number after a failure known to have happened before remote submission.
    pub fn retry_pending_effect(&mut self, key: &IssueMonitorEffectAttemptKey) -> bool {
        let Some(effect) = self.pending_effects.iter_mut().find(|effect| {
            effect.state == IssueMonitorEffectState::Attempting && effect_matches_key(effect, key)
        }) else {
            return false;
        };
        let Some(next_attempt) = effect.attempt.checked_add(1) else {
            return false;
        };
        effect.attempt = next_attempt;
        effect.state = IssueMonitorEffectState::Prepared;
        true
    }

    pub fn advance_effect_authority_epoch(&mut self) -> Option<u64> {
        advance_effect_authority(&mut self.effect_authority_epoch, &mut self.pending_effects)
    }

    /// Advance autonomous authority atomically. Turning autonomous mode off
    /// cancels unsubmitted arm proposals and durably compensates every
    /// outcome-ambiguous arm attempt. Overflow rejects the whole transition.
    pub fn set_autonomous_mode_with_effect_revocation(&mut self, enabled: bool) -> Option<u64> {
        advance_autonomous_effect_authority(
            &mut self.autonomous_mode,
            &mut self.effect_authority_epoch,
            &mut self.pending_effects,
            enabled,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchedIssue {
    pub issue_number: u64,
    pub window_id: String,
}

/// #3223 follow-up: one claimed-but-unbound launch with its claim anchor.
/// `claimed_at` lets a restored claim EXPIRE after `claim_ttl_secs` instead of
/// holding a max-active slot forever when the process died before the ACK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchingIssue {
    pub issue_number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorLaunchSessionStrategy {
    #[default]
    ResumeIfSafe,
    FreshRequired,
}

/// Typed failure metadata carried across the AppRuntime/daemon/Monitor
/// boundary. Message-only legacy failures omit this value and retain their
/// existing retry/terminal behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueMonitorFailure {
    ResumeWriterConflict {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        holder_window_id: Option<String>,
    },
    /// Issue #3616: the provider backing the agent ran out of quota.
    ///
    /// Distinct from every other agent exit because the cause is stated by the
    /// provider itself and is about the account, not the work: the retry budget
    /// must not be spent and the Issue must not become terminal.
    ProviderUsageLimit {
        /// Agent id the launch used (`codex` / `claude`), so a reader can tell
        /// which account is out rather than pausing the whole fleet.
        provider: String,
        /// RFC3339 instant the provider said access returns, when it printed a
        /// parseable one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets_at: Option<String>,
        /// Issue #3923 AC-2: what the block was read from — the matched screen
        /// text and the usage poller's reading at that moment — so a false
        /// hold can be diagnosed from the record instead of guessed at.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<IssueMonitorProviderQuotaHoldEvidence>,
    },
}

/// Result of applying a typed late-resume writer-conflict recovery. The
/// authority-exhausted case is distinct from a stale source so callers can
/// fail closed instead of acknowledging a transition that never committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMonitorResumeWriterConflictOutcome {
    Requeued,
    Rejected,
    AuthorityExhausted,
}

/// Issue #3616: result of applying a provider quota hold. `Rejected` means the
/// reporting window is no longer the bound launch, or the issue is already
/// terminal — in both cases nothing was mutated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMonitorProviderUsageLimitOutcome {
    Held,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingIssueMonitorLaunchDelivery {
    pub delivery_id: String,
    pub issue_number: u64,
    pub branch_name: String,
    pub linked_issue_kind: LinkedIssueKind,
    pub claim_id: String,
    #[serde(default)]
    pub claim_owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materializer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materializer_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materializer_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialized_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_durable_window_id: Option<String>,
    #[serde(default)]
    pub launch_session_strategy: IssueMonitorLaunchSessionStrategy,
    pub created_at: String,
}

/// Backward-compat: the first shipped shape was a bare id array. A parse
/// failure here would `unwrap_or_default()` into a full prefs wipe, so both
/// shapes must deserialize.
fn deserialize_launching_issues<'de, D>(
    deserializer: D,
) -> Result<Vec<IssueMonitorLaunchingIssue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Compat {
        Bare(u64),
        Full(IssueMonitorLaunchingIssue),
    }
    let entries = Vec::<Compat>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            Compat::Bare(issue_number) => IssueMonitorLaunchingIssue {
                issue_number,
                claimed_at: None,
            },
            Compat::Full(full) => full,
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorFailedIssue {
    pub issue_number: u64,
    pub message: String,
    /// #3165 error-window lifecycle: the agent window that was on the canvas
    /// when this issue failed. Persisted so an explicit Launch Now (even after a
    /// daemon/GUI restart) can close the stale window before relaunching. `None`
    /// for failures that never opened a window (e.g. pre-launch errors).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

/// Issue #3645 / #3628: one operator recovery, published so every process
/// converges on it. Kept until the same issue fails again, at which point the
/// newer failure supersedes the release and the entry is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorReleasedFailure {
    pub issue_number: u64,
    /// Value of [`IssueMonitorPrefs::failure_release_version`] when this
    /// release was issued. A reader adopts it only when it is newer than the
    /// snapshot the reader already observed.
    pub release_version: u64,
    pub released_at: String,
    pub reason: String,
    /// Persisted autonomous attempt count observed before the release.
    #[serde(default)]
    pub attempts_before: u32,
    /// Persisted autonomous attempt count after the release. New releases set
    /// this to zero; the default keeps pre-Issue-3734 preferences readable.
    #[serde(default)]
    pub attempts_after: u32,
}

/// Issue #3734 FR-113: operator reset evidence is durable but cannot grow
/// without bound while the Monitor runs unattended.
const REQUEUE_AUDIT_CAP: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchProfile {
    pub agent_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub session_mode: gwt_agent::SessionMode,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub codex_fast_mode: bool,
    #[serde(default)]
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    #[serde(default)]
    pub docker_service: Option<String>,
    #[serde(default)]
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
    #[serde(default)]
    pub windows_shell: Option<gwt_agent::WindowsShellKind>,
}

impl From<&gwt_agent::LaunchConfig> for IssueMonitorLaunchProfile {
    fn from(config: &gwt_agent::LaunchConfig) -> Self {
        Self {
            agent_id: config.agent_id.command().to_string(),
            model: config.model.clone(),
            reasoning: config.reasoning_level.clone(),
            version: config.tool_version.clone(),
            session_mode: config.session_mode,
            skip_permissions: config.skip_permissions,
            codex_fast_mode: config.fast_mode || config.codex_fast_mode,
            runtime_target: config.runtime_target,
            docker_service: config.docker_service.clone(),
            docker_lifecycle_intent: config.docker_lifecycle_intent,
            windows_shell: config.windows_shell,
        }
    }
}

impl From<IssueMonitorLaunchProfile> for LaunchWizardPreviousProfile {
    fn from(profile: IssueMonitorLaunchProfile) -> Self {
        Self {
            agent_id: profile.agent_id,
            model: profile.model,
            reasoning: profile.reasoning,
            version: profile.version,
            session_mode: profile.session_mode,
            skip_permissions: profile.skip_permissions,
            codex_fast_mode: profile.codex_fast_mode,
            runtime_target: profile.runtime_target,
            docker_service: profile.docker_service,
            docker_lifecycle_intent: profile.docker_lifecycle_intent,
            windows_shell: profile.windows_shell,
            hermes: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorLaunchProfileSource {
    Saved,
    LastSettings,
    Default,
}

impl IssueMonitorLaunchProfileSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "Saved",
            Self::LastSettings => "Last settings",
            Self::Default => "Default",
        }
    }
}

pub fn issue_monitor_launch_profile_summary(profile: &LaunchWizardPreviousProfile) -> String {
    let model = profile.model.as_deref().unwrap_or("default");
    let reasoning = profile.reasoning.as_deref().unwrap_or("auto");
    format!(
        "{} / {} / {} / {}",
        profile.agent_id,
        model,
        reasoning,
        issue_monitor_runtime_label(profile.runtime_target)
    )
}

fn issue_monitor_runtime_label(target: gwt_agent::LaunchRuntimeTarget) -> &'static str {
    match target {
        gwt_agent::LaunchRuntimeTarget::Host => "host",
        gwt_agent::LaunchRuntimeTarget::Docker => "docker",
    }
}

impl Default for IssueMonitorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: 300,
            claim_heartbeat_secs: 300,
            claim_ttl_secs: 1800,
            max_active: 1,
            queue_when_gui_absent: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorIssueState {
    #[default]
    Open,
    Closed,
}

/// Provenance of one Issue Monitor candidate snapshot. Only a complete live
/// GitHub result is safe to authorize a destructive persisted-state migration;
/// cache snapshots may be partial and are scan-only inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorCandidateSource {
    Live,
    /// Live GitHub data that reached the configured list cap. It is fresh but
    /// cannot prove that an absent Issue is closed or belongs elsewhere.
    LiveIncomplete,
    Cache,
}

/// Match only the exact pre-#3272 launch failure for the current project.
/// Windows provider/verbatim prefixes are normalized on both sides, but the
/// remaining path must otherwise be equal — substrings, suffixes and nearby
/// errors are deliberately rejected.
pub fn is_legacy_git_launch_failure_for_project(message: &str, project_root: &Path) -> bool {
    let Some(failed_path) = message.strip_prefix(LEGACY_GIT_LAUNCH_FAILURE_PREFIX) else {
        return false;
    };
    gwt_core::paths::normalize_windows_child_process_path(Path::new(failed_path))
        == gwt_core::paths::normalize_windows_child_process_path(project_root)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorReadiness {
    #[default]
    NotApplicable,
    Ready,
    /// Structured SPEC artifacts are launch-ready but at least one task is
    /// still unchecked, so a work merge is not Issue-wide completion.
    ReadyWithOpenTasks,
    /// Structured SPEC artifacts are launch-ready and every task checkbox is
    /// checked. This is the only SPEC readiness that permits completion.
    ReadyWithCompletedTasks,
    NotReady,
}

impl IssueMonitorReadiness {
    fn is_launch_ready(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::ReadyWithOpenTasks | Self::ReadyWithCompletedTasks
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorIssue {
    pub number: u64,
    pub title: String,
    pub labels: Vec<String>,
    pub state: IssueMonitorIssueState,
    /// The acceptance-criteria source text the autonomous gate classifies.
    /// The Issue body; for a gwt-spec Issue whose `spec` section is routed to
    /// a comment, the assembled section is appended so the block is seen
    /// regardless of storage (Issue #3930 AC-4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub readiness: IssueMonitorReadiness,
    /// GitHub's Issue revision used to bind completion evidence. Missing data
    /// is fail-closed for ordinary linked-PR completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorInboxState {
    Queued,
    /// A design-required Issue is missing a usable plan or tasks artifact.
    /// This is re-evaluated on every scan and is therefore non-terminal.
    NotReady,
    /// An operator excluded the Issue from automatic execution with a hold
    /// label. This is re-evaluated on every scan and is therefore non-terminal.
    HoldExcluded,
    Launching,
    Launched,
    /// Current Issue generation is complete. The GitHub Issue may still be
    /// open until release, so this is distinct from `Released`.
    Merged,
    /// The GitHub Issue was closed (e.g. at release). Final terminal state.
    Released,
    LaunchFailed,
    AgentFailed,
    BlockedByClaim,
    Skipped,
    /// SPEC #3200 FR-027: autonomous resolution exhausted its bounded retries,
    /// hit a terminal review failure, or could not verify its safety gates, and
    /// has been handed back to a human. Terminal: scan / requeue / window-close
    /// must never revive it; only an explicit human reset exits it.
    NeedsHuman,
}

impl MonitorInboxState {
    /// A terminal state whose meaning must not be overwritten by a later
    /// window/project close (which only re-queues still-active work) or by a
    /// scan re-queue.
    fn is_terminal(self) -> bool {
        matches!(
            self,
            MonitorInboxState::Merged
                | MonitorInboxState::Released
                | MonitorInboxState::LaunchFailed
                | MonitorInboxState::AgentFailed
                | MonitorInboxState::NeedsHuman
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorInboxItem {
    pub issue: IssueMonitorIssue,
    pub state: MonitorInboxState,
    pub claim_id: Option<String>,
    pub blocked_by_owner: Option<String>,
    pub claim_expires_at: Option<String>,
    pub launched_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_plan: Option<IssueMonitorLaunchPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<String>,
}

/// SPEC-3431 FR-033: the exact identity a PM-requested stop must match.
///
/// Naming an issue is not enough. A stale notification, a concurrent scan, or
/// a PM working from an old snapshot all name a real issue number; what tells
/// them apart is whether they also name the claim, delivery, and window that
/// are live *right now*. Every component is compared, including its absence —
/// omitting the window id of a launched agent is a mismatch, not a wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMonitorStopTarget {
    pub issue_number: u64,
    pub claim_id: Option<String>,
    pub delivery_id: Option<String>,
    pub window_id: Option<String>,
}

/// Issue #3927 (SPEC #3340 FR-044): the durable Monitor facts the runtime's
/// canonical terminal predicate reads for one Issue-linked window. Read-only;
/// the runtime decides, the Monitor only reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IssueMonitorTerminalWindowFacts {
    /// The Monitor durably binds this exact window to the Issue.
    pub binds_this_window: bool,
    /// The Monitor durably binds the Issue to a different window: this
    /// launch was replaced (failover / relaunch).
    pub binds_other_window: bool,
    /// A durable closed-Issue record or validated Issue-wide completion
    /// exists.
    pub issue_closed: bool,
    /// The row is parked for a human.
    pub needs_human: bool,
    /// A failure record (launch / agent failure or operator stop) holds the
    /// Issue.
    pub failure_hold: bool,
}

/// SPEC-3431 FR-033: why a stop request was refused. Every variant leaves the
/// monitor untouched, so the caller can surface the reason without having to
/// undo anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMonitorStopMismatch {
    /// No inbox row for that issue in this project.
    UnknownIssue,
    /// The issue is queued, blocked, skipped, or already terminal — there is
    /// no live launch to revoke.
    NotRunning,
    ClaimMismatch,
    DeliveryMismatch,
    WindowMismatch,
}

/// SPEC-3431 FR-033: the result of a stop_only request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorStopOutcome {
    /// Authority and slot revoked. `window_id` is the pane the caller must now
    /// reap through the Monitor-owned lifecycle path.
    Stopped {
        window_id: String,
    },
    /// This exact launch was already stopped. No second revocation.
    AlreadyStopped,
    Mismatch(IssueMonitorStopMismatch),
}

/// SPEC-3431 FR-029〜031: the result of a failover_restart request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorFailoverOutcome {
    /// The launch was revoked and the issue is queued at the head for the
    /// currently saved profile. `stopped_window_id` is the pane the caller must
    /// still reap; it is `None` when the launch had not materialized one yet.
    Restarting {
        stopped_window_id: Option<String>,
    },
    /// The exact source was current, but the durable effect authority could
    /// not advance. No field was mutated and automation must remain denied.
    AuthorityExhausted,
    Mismatch(IssueMonitorStopMismatch),
}

/// Issue #3645 / #3628: the result of an operator recovery for a row that a
/// failure is holding out of the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorRequeueOutcome {
    /// The hold was released and the issue is queued again. `stale_window_id`
    /// is the error window the failure retained, if any, so the caller can reap
    /// it — the release already unbound it from the issue.
    Requeued {
        stale_window_id: Option<String>,
        attempts_before: u32,
        attempts_after: u32,
    },
    /// A launch still owns this issue. Recovering it here would mint a second
    /// claim for one issue, so the exact-identity operations `stop_only` and
    /// `failover_restart` own that case instead.
    LaunchLive,
    /// No failure is holding this issue. Nothing was changed, and reporting a
    /// recovery would tell the operator the state moved when it did not.
    NotHeld,
}

/// SPEC-3431 FR-033: marks an inbox error as a deliberate stop rather than a
/// failure, so FR-031 diagnostics can tell "the PM stopped this" apart from
/// "this ran out of retries".
const STOP_ONLY_REASON_PREFIX: &str = "stopped: ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchRequest {
    pub issue_number: u64,
    pub branch_name: String,
    pub linked_issue_kind: LinkedIssueKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    #[serde(default)]
    pub launch_session_strategy: IssueMonitorLaunchSessionStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchPlan {
    pub branch_name: String,
    pub linked_issue_kind: LinkedIssueKind,
    pub prompt: String,
}

/// Provider-wide launch admission hold projected to both agent and GUI
/// readers. `provider` is the canonical agent command and `reset_at` is a
/// concrete RFC3339 UTC deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorProviderQuotaHold {
    pub provider: String,
    pub reset_at: String,
    /// Issue #3923 AC-2: the evidence the hold was formed from, when the
    /// process that formed it recorded any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<IssueMonitorProviderQuotaHoldEvidence>,
}

/// Issue #3923 AC-2: one usage-poller window as it read when a hold formed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorProviderQuotaPollerWindow {
    pub kind: String,
    /// Whole percent, `0..=100`. Rounded so the state stays `Eq`.
    pub used_percent: u8,
}

/// Issue #3923 AC-2: what a provider quota hold was formed from.
///
/// The incident this records against was a hold whose only basis was a stale
/// notice at the bottom of a pane while the poller read the account at 26%.
/// Keeping both readings on the hold makes that contradiction visible in
/// `issue.monitor.status` and in gwt.log instead of requiring a guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorProviderQuotaHoldEvidence {
    /// RFC3339 instant the hold was recorded. Compared against a release's
    /// `released_at` to decide whether that release fences this hold.
    pub recorded_at: String,
    /// `screen_notice` (a provider notice on the pane), `usage_poller` (the
    /// legacy poller-driven release), or `unrecorded` for a path that gave no
    /// detail.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    /// The screen tail the notice was matched in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_text: Option<String>,
    /// The poller's `UsageState` for the agent's provider (`ok`, `stale`, ...),
    /// `None` when the poller had no reading for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poller_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poller_limit_reached: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub poller_windows: Vec<IssueMonitorProviderQuotaPollerWindow>,
}

impl IssueMonitorProviderQuotaHoldEvidence {
    /// Evidence for a hold read from a provider notice on `window_id`'s screen.
    pub fn screen_notice(recorded_at: &str, window_id: &str, screen_text: &str) -> Self {
        Self {
            recorded_at: recorded_at.to_string(),
            source: "screen_notice".to_string(),
            issue_number: None,
            window_id: Some(window_id.to_string()),
            screen_text: Some(screen_text.trim().to_string()).filter(|text| !text.is_empty()),
            poller_state: None,
            poller_limit_reached: None,
            poller_windows: Vec::new(),
        }
    }

    /// Attach the usage poller's current reading for `agent_id`'s provider.
    pub fn with_poller(
        mut self,
        agent_id: Option<&str>,
        accounts: &[gwt_core::usage::ProviderUsage],
    ) -> Self {
        let Some(account) = agent_id.and_then(|agent_id| account_for_agent(agent_id, accounts))
        else {
            return self;
        };
        self.poller_state = Some(usage_state_label(&account.state).to_string());
        self.poller_limit_reached = Some(account.limit_reached);
        self.poller_windows = account
            .windows
            .iter()
            .map(|window| IssueMonitorProviderQuotaPollerWindow {
                kind: window.kind.as_str().to_string(),
                used_percent: window.used_percent.round().clamp(0.0, 100.0) as u8,
            })
            .collect();
        self
    }
}

fn usage_state_label(state: &gwt_core::usage::UsageState) -> &'static str {
    use gwt_core::usage::UsageState;
    match state {
        UsageState::Ok => "ok",
        UsageState::Disabled => "disabled",
        UsageState::NoData => "no_data",
        UsageState::Unavailable { .. } => "unavailable",
        UsageState::Stale { .. } => "stale",
    }
}

/// Issue #3923 AC-1: an operator's explicit release of a provider quota hold.
///
/// Holds join by the later reset across every process, so removing one from
/// disk says nothing to a process that still holds it in memory — it would
/// re-stamp the hold on its next commit. A release is therefore a published
/// fact: it voids every hold recorded at or before `released_at` and leaves a
/// hold recorded after it untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorProviderQuotaHoldRelease {
    pub released_at: String,
    pub reason: String,
    /// The reset deadline the hold carried when it was released, for audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_reset_at: Option<String>,
}

/// Issue #3923 AC-5: why a CLI launch-profile switch was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorLaunchProfileSwitchError {
    /// The wizard has never saved a profile: the runtime / Docker / session
    /// choices it carries cannot be invented from an agent name alone.
    NoSavedProfile,
    /// The agent name is blank.
    InvalidAgent,
}

impl std::fmt::Display for IssueMonitorLaunchProfileSwitchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSavedProfile => f.write_str(
                "no saved Issue Monitor launch profile; configure one in the GUI before switching its agent",
            ),
            Self::InvalidAgent => f.write_str("launch_agent must name an agent (for example codex or claude)"),
        }
    }
}

/// Result of [`IssueMonitorState::clear_provider_quota_hold`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorProviderQuotaHoldClearOutcome {
    /// `provider` is blank. Any non-blank name resolves to a builtin or
    /// custom agent id, matching how holds themselves are keyed.
    UnknownProvider,
    /// The release is recorded. `released_reset_at` is `None` when nothing was
    /// held in this snapshot; the release still fences a hold another process
    /// has not committed yet.
    Cleared {
        released_reset_at: Option<String>,
        released_issues: Vec<u64>,
    },
}

/// Whether `release` voids a hold recorded with `evidence`. A hold with no
/// recorded instant predates evidence recording and is voided by any release.
fn provider_quota_hold_is_released(
    evidence: Option<&IssueMonitorProviderQuotaHoldEvidence>,
    release: Option<&IssueMonitorProviderQuotaHoldRelease>,
) -> bool {
    let Some(released_at) = release.and_then(|release| parse_rfc3339_utc(&release.released_at))
    else {
        return false;
    };
    match evidence.and_then(|evidence| parse_rfc3339_utc(&evidence.recorded_at)) {
        Some(recorded_at) => recorded_at <= released_at,
        None => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorStatusView {
    pub enabled: bool,
    pub state: String,
    pub queue_len: usize,
    pub active_count: usize,
    pub max_active_agents: usize,
    pub total_candidates: usize,
    pub active_issue_number: Option<u64>,
    pub last_scan_at: Option<String>,
    pub last_error: Option<String>,
    pub launch_profile_source: IssueMonitorLaunchProfileSource,
    pub launch_profile_summary: String,
    /// SPEC #3200 T-048/FR-001: whether unattended autonomous mode is enabled.
    #[serde(default)]
    pub autonomous_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_hold: Option<IssueMonitorProviderQuotaHold>,
    /// SPEC #3200 T-048/FR-033: per-issue autonomous lifecycle summary, so every
    /// decision boundary (phase, attempts, needs-human) is observable.
    #[serde(default)]
    pub autonomous_issues: Vec<AutonomousIssueSummary>,
}

/// Atomic agent-facing projection of the live Issue Monitor driver state.
/// Unlike [`IssueMonitorStatusView`], this includes the ordered queue itself so
/// callers never have to reconstruct transient claim outcomes from cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorAgentStatus {
    pub queue: Vec<u64>,
    pub active_launches: Vec<u64>,
    pub max_active: usize,
    pub enabled: bool,
    pub autonomous_mode: bool,
    pub has_launch_profile: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_hold: Option<IssueMonitorProviderQuotaHold>,
    /// Issue #3923 AC-1: every provider hold in force, with its evidence, so
    /// the PM can see a hold on a provider the profile is not using and
    /// release it by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_quota_holds: Vec<IssueMonitorProviderQuotaHold>,
    /// SPEC-3431 FR-024: issues handed back to a human. Previously reachable
    /// only through the daemon's lossy broadcast ring, which made a missed
    /// escalation unrecoverable for an unattended reader.
    #[serde(default)]
    pub needs_human: Vec<u64>,
    /// Per-issue lifecycle rows, so a caller can tell "not launched yet" from
    /// "claimed by someone else" from "failed", and can map an issue to the
    /// pane that is working on it.
    #[serde(default)]
    pub inbox: Vec<IssueMonitorInboxSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan_at: Option<String>,
    /// Issue #3633 (AC-5): why the scan cadence looks stopped, when it does.
    ///
    /// Deliberately its own field rather than a projection onto `last_error`.
    /// In the production incident `last_error` was already occupied by an
    /// unrelated per-issue launch failure, so a stall written there would have
    /// stayed invisible — which is the exact failure mode this Issue is about:
    /// a monitor that has stopped while every field still reads healthy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_stall: Option<String>,
}

/// SPEC-3431 FR-069: when the provider backing `agent_id` is out of quota,
/// the instant it recovers.
///
/// `None` when that provider has quota left, is unknown to the usage poller,
/// or reports no reset instant. Scoped to the agent's own provider on purpose:
/// one exhausted account must not stall launches running on a different one.
///
/// Pure so the decision can be tested without a usage poller, and so the
/// caller (which has the launch profile) supplies the agent rather than this
/// module guessing it.
pub fn rate_limit_reset_for_agent(
    agent_id: &str,
    accounts: &[gwt_core::usage::ProviderUsage],
) -> Option<String> {
    account_for_agent(agent_id, accounts)
        .filter(|account| account.limit_reached)?
        .windows
        .iter()
        .filter_map(|window| window.resets_at)
        .min()
        .map(|reset| reset.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// Issue #3616: whether the poller independently reports `agent_id`'s account
/// out of quota.
///
/// Distinct from [`rate_limit_reset_for_agent`], which returns `None` both for
/// "has quota left" and for "out of quota but printed no reset instant". A
/// caller corroborating a screen notice needs the first fact on its own.
pub fn provider_limit_reached_for_agent(
    agent_id: &str,
    accounts: &[gwt_core::usage::ProviderUsage],
) -> bool {
    account_for_agent(agent_id, accounts).is_some_and(|account| account.limit_reached)
}

/// Issue #3923 AC-3: whether the poller independently reports `agent_id`'s
/// account as usable — a fresh `Ok` reading, `limit_reached` false, and no
/// window at 100%.
///
/// This is the reading that contradicts a stale notice at the bottom of a
/// pane. Anything short of a fresh reading (stale, unavailable, no data) is
/// not a contradiction and must not suppress a hold.
pub fn provider_reports_healthy_for_agent(
    agent_id: &str,
    accounts: &[gwt_core::usage::ProviderUsage],
) -> bool {
    account_for_agent(agent_id, accounts).is_some_and(|account| {
        account.state == gwt_core::usage::UsageState::Ok
            && !account.limit_reached
            && account
                .windows
                .iter()
                .all(|window| window.used_percent < 100.0)
    })
}

/// The poller account backing `agent_id`, scoped to that agent's own provider
/// so one drained account never stalls a fleet on another.
fn account_for_agent<'accounts>(
    agent_id: &str,
    accounts: &'accounts [gwt_core::usage::ProviderUsage],
) -> Option<&'accounts gwt_core::usage::ProviderUsage> {
    use gwt_core::usage::UsageProvider;

    let provider = match agent_id.trim().to_ascii_lowercase().as_str() {
        "codex" => UsageProvider::Codex,
        "claude" | "claude-code" => UsageProvider::ClaudeCode,
        _ => return None,
    };
    accounts.iter().find(|account| account.provider == provider)
}

/// Issue #3676 AC-2: whether the CLI backing an agent holds usable
/// credentials, judged only from durable on-disk / environment facts.
///
/// `Unknown` is deliberate fail-open: providers that keep credentials where
/// this process cannot look (for example Claude Code's macOS Keychain entry)
/// must never be reported as unauthenticated. Only a definitive "this CLI
/// would boot into its login screen" observation may block a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAuthState {
    Authenticated,
    Unauthenticated,
    Unknown,
}

/// Ambient credential facts for [`provider_auth_state`], captured at the edge
/// so the decision itself stays pure and testable without environment reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderAuthProbe<'facts> {
    pub codex_home: Option<&'facts Path>,
    pub claude_home: Option<&'facts Path>,
    pub openai_api_key: Option<&'facts str>,
    pub anthropic_api_key: Option<&'facts str>,
}

/// Credential verdict for `agent_id` from the supplied ambient facts.
pub fn provider_auth_state(agent_id: &str, probe: &ProviderAuthProbe) -> ProviderAuthState {
    match agent_id.trim().to_ascii_lowercase().as_str() {
        "codex" => {
            if probe
                .openai_api_key
                .is_some_and(|key| !key.trim().is_empty())
            {
                return ProviderAuthState::Authenticated;
            }
            let Some(home) = probe.codex_home else {
                return ProviderAuthState::Unknown;
            };
            match fs::read_to_string(home.join("auth.json")) {
                Ok(body) => codex_auth_state_from_body(&body),
                // A Codex CLI without auth.json boots into its login screen.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    ProviderAuthState::Unauthenticated
                }
                Err(_) => ProviderAuthState::Unknown,
            }
        }
        "claude" | "claude-code" => {
            if probe
                .anthropic_api_key
                .is_some_and(|key| !key.trim().is_empty())
            {
                return ProviderAuthState::Authenticated;
            }
            let Some(home) = probe.claude_home else {
                return ProviderAuthState::Unknown;
            };
            if home.join(".credentials.json").exists() {
                ProviderAuthState::Authenticated
            } else {
                ProviderAuthState::Unknown
            }
        }
        _ => ProviderAuthState::Unknown,
    }
}

/// Credential verdict from a Codex `auth.json` body: either an API key or a
/// non-empty OAuth token must be present. An unparseable body is treated as
/// unauthenticated because the Codex CLI itself cannot use it either.
fn codex_auth_state_from_body(body: &str) -> ProviderAuthState {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return ProviderAuthState::Unauthenticated;
    };
    let api_key_present = value
        .get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|key| !key.trim().is_empty());
    let tokens_present = value.get("tokens").is_some_and(|tokens| {
        ["access_token", "refresh_token"].iter().any(|field| {
            tokens
                .get(*field)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|token| !token.trim().is_empty())
        })
    });
    if api_key_present || tokens_present {
        ProviderAuthState::Authenticated
    } else {
        ProviderAuthState::Unauthenticated
    }
}

/// Edge wrapper reading the ambient credential facts from the environment.
/// Production launch paths install this as the Monitor's auth probe; tests
/// inject their own probe instead of mutating process state.
pub fn provider_auth_state_from_env(agent_id: &str) -> ProviderAuthState {
    let codex_home = gwt_core::usage::codex::codex_home();
    let claude_home = gwt_core::usage::claude::claude_home();
    let openai_api_key = std::env::var("OPENAI_API_KEY").ok();
    let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    provider_auth_state(
        agent_id,
        &ProviderAuthProbe {
            codex_home: codex_home.as_deref(),
            claude_home: claude_home.as_deref(),
            openai_api_key: openai_api_key.as_deref(),
            anthropic_api_key: anthropic_api_key.as_deref(),
        },
    )
}

/// Identifiable launch-refusal message for an unauthenticated provider CLI
/// (Issue #3676 AC-2). The `provider_unauthenticated:` prefix is the machine
/// marker; the rest tells the operator how to restore the provider.
pub fn provider_unauthenticated_message(agent_id: &str) -> String {
    format!(
        "provider_unauthenticated: the '{agent_id}' CLI has no usable credentials, so the \
         Issue Monitor refused this launch before starting a terminal. Log the CLI in \
         (codex: run `codex login`) or point the Monitor launch profile at an \
         authenticated provider, then retry."
    )
}

/// One inbox row, reduced to the facts an agent acts on. The full
/// [`IssueMonitorInboxItem`] carries the whole GitHub issue payload, which
/// would dwarf the rest of the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorInboxSummary {
    pub issue_number: u64,
    pub state: MonitorInboxState,
    #[serde(default)]
    pub github_state: IssueMonitorIssueState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_updated_at: Option<String>,
    #[serde(default)]
    pub readiness: IssueMonitorReadiness,
    #[serde(default)]
    pub recoverable_merged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// SPEC-3431 FR-068: when this launch last showed signs of life (RFC3339).
    ///
    /// Seeded at launch and advanced by hook arrivals, which is the only clock
    /// that tracks real progress: a live agent blocked on an approval prompt,
    /// a provider rate limit, or a genuine hang produces no PTY status change
    /// at all, so without this the four cases are indistinguishable from
    /// healthy work. Reported rather than judged — the reader decides what
    /// "too old" means for the work at hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// Issue #3616 AC-3/AC-4: when this issue is held out of the queue, until
    /// when and why.
    ///
    /// A queued row with a reset days away is not the same situation as a
    /// queued row waiting its turn, and a reader with only `state` cannot tell
    /// them apart. Both fields project the autonomous record so there is one
    /// clock rather than a parallel one that can disagree with the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_not_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_hold_reason: Option<String>,
    /// SPEC-3431 FR-024/FR-033: the claim and delivery backing this launch.
    ///
    /// `stop_only` requires an exact identity match, so the PM has to be able
    /// to read the identity it is asked to send. Without these two the
    /// requirement is unsatisfiable from the snapshot alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    /// Issue #3844 AC-2: what the launched agent declared it is waiting for,
    /// so a silent row can be told apart from a stalled one by its reader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<IssueMonitorWaitSummary>,
    /// Issue #3944 AC-2: the Monitor's open request that the PM steer this
    /// launch (stuck with a live window, attempts exhausted, or a held gate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering: Option<AutonomousSteeringRequest>,
}

/// SPEC #3200 T-048: status-view summary of one issue's autonomous lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousIssueSummary {
    pub issue_number: u64,
    pub phase: AutonomousPhase,
    pub attempts: u32,
    pub needs_human: bool,
    /// Issue #3478 (AC-9): why the issue is parked, in operator-facing English.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_human_reason: Option<String>,
    /// Issue #3944 AC-1: which human-answerable kind parked the issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_human_kind: Option<NeedsHumanKind>,
    /// Issue #3944 AC-2: the open steering request for the PM, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering: Option<AutonomousSteeringRequest>,
    /// Issue #3478 (AC-9): the question waiting for a human, when the park was
    /// caused by a confirmation question rather than a failed gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<AutonomousPendingQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorState {
    pub config: IssueMonitorConfig,
    pub gui_connected: bool,
    pub inbox: Vec<IssueMonitorInboxItem>,
    legacy_git_launch_failure_migration_version: u32,
    /// Exact failure fingerprints removed by an in-memory migration that has
    /// not necessarily committed yet. An older concurrent snapshot may merge
    /// unrelated failures, but never these exact rows.
    #[serde(default, skip)]
    legacy_git_launch_failure_migration_tombstones: BTreeMap<u64, String>,
    last_scan_at: Option<String>,
    last_error: Option<String>,
    launch_auth_required: bool,
    active_launches: Vec<u64>,
    priority_order: Vec<u64>,
    launch_profile: Option<IssueMonitorLaunchProfile>,
    /// Durable provider-wide launch admission source of truth. Keys and values
    /// are canonicalized on restore and joined monotonically across rebases.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    provider_quota_holds: BTreeMap<String, String>,
    /// Issue #3923 AC-2: evidence per held provider.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    provider_quota_hold_evidence: BTreeMap<String, IssueMonitorProviderQuotaHoldEvidence>,
    /// Issue #3923 AC-1: release fences per provider.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    provider_quota_hold_releases: BTreeMap<String, IssueMonitorProviderQuotaHoldRelease>,
    launched_windows: BTreeMap<u64, String>,
    /// Durable generation for each launched window binding. A successor launch
    /// receives a new claim even when its issue and window ids are reused.
    launched_claims: BTreeMap<u64, String>,
    /// issue → work branch for currently launched Issues, used to look up the
    /// PR when checking whether the work has merged.
    launched_branches: BTreeMap<u64, String>,
    /// Compatibility projection of completed records (state `Merged`).
    merged_issues: BTreeSet<u64>,
    issue_completion_migration_version: u32,
    completion_records: BTreeMap<u64, IssueCompletionRecord>,
    /// Issue #3602: source of truth for whether current-action companions may
    /// exist. Completion records remain separate historical delivery facts.
    #[serde(default)]
    closure_records: BTreeMap<u64, IssueClosureRecord>,
    /// Process-local fence for a Closed→Reopened transition that has not yet
    /// necessarily reached disk. Baseline Open observations do not enter it.
    #[serde(default, skip)]
    closure_reopen_tombstones: BTreeSet<u64>,
    /// SPEC #3200 FR-001: opt-in autonomous (unattended) resolution mode.
    autonomous_mode: bool,
    /// SPEC #3200 Phase 7: authority generation and durable remote-effect
    /// journal, mirrored losslessly by [`IssueMonitorPrefs`].
    #[serde(default)]
    effect_authority_epoch: u64,
    #[serde(default)]
    pending_effects: Vec<PendingIssueMonitorEffect>,
    #[serde(default)]
    last_control_receipt: Option<IssueMonitorControlReceipt>,
    /// SPEC #3200 FR-030: tunable bounds for unattended operation.
    autonomous_tuning: AutonomousTuning,
    /// SPEC #3200 T-016/T-022: per-issue autonomous lifecycle records keyed by
    /// issue number (attempt counter, phase, in-flight launch id, snapshot).
    autonomous_records: BTreeMap<u64, AutonomousIssueRecord>,
    failed_issues: BTreeMap<u64, String>,
    /// #3165 error-window lifecycle: the stale agent window id retained per
    /// failed issue, so an explicit Launch Now can close it before relaunching.
    failed_windows: BTreeMap<u64, String>,
    /// Issue #3645 / #3628: operator recoveries published for other processes.
    /// Mutually exclusive with `failed_issues` per issue — a newer failure
    /// always supersedes the release that preceded it.
    #[serde(default)]
    released_failures: BTreeMap<u64, IssueMonitorReleasedFailure>,
    /// Issue #3734: durable operator-reset evidence, ordered by release
    /// version and capped by [`REQUEUE_AUDIT_CAP`].
    #[serde(default)]
    requeue_audit: Vec<IssueMonitorReleasedFailure>,
    /// Highest release version this process has issued or observed.
    #[serde(default)]
    failure_release_version: u64,
    queue: VecDeque<u64>,
    pending_launches: VecDeque<IssueMonitorLaunchRequest>,
    /// Durable ACK-driven deliveries. Unlike `pending_launches`, this queue is
    /// projected non-destructively and mirrored in prefs across restart.
    pending_launch_deliveries: VecDeque<PendingIssueMonitorLaunchDelivery>,
    /// Durable one-shot policy for an issue's next launch. Once materialized,
    /// the delivery/request carries the policy and this marker is consumed.
    queued_launch_session_strategies: BTreeMap<u64, IssueMonitorLaunchSessionStrategy>,
    /// SPEC #3200 Option A: review-agent spawn requests produced by the
    /// orchestration loop, drained by the daemon→GUI payload builder.
    pending_review_dispatches: VecDeque<AutonomousReviewDispatch>,
    /// SPEC #3200 FR-034 (T-111): operator notices produced by autonomous
    /// lifecycle transitions (merged / needs-human / retry / auto-merge armed).
    /// Drained into `toast` payloads by the daemon→GUI payload builder; retained
    /// (bounded) while no GUI is connected so unattended events surface on the
    /// next connect.
    pending_autonomous_notices: VecDeque<AutonomousNotice>,
    /// #3223 follow-up: claim anchors for unbound launches (issue → RFC3339).
    launching_claimed_at: BTreeMap<u64, String>,
    /// Issue #3478 (FR-025): structured question handoffs written by the
    /// intercepting hook and driven to `AwaitingHuman` / `Resumed` here.
    #[serde(default)]
    autonomous_handoffs: Vec<AutonomousQuestionHandoff>,
}

/// Issue #3478 (AC-5): one answered handoff ready to be delivered back to the
/// exact session that asked, so the parked work resumes without a duplicate
/// launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousHandoffResumption {
    pub handoff_id: String,
    pub issue_number: u64,
    /// gwt Session id of the agent that asked. The resume path must target
    /// this exact session.
    pub session_id: String,
    /// Answer prompt delivered to the resumed session (question + answer).
    pub prompt: String,
}

/// One write-ahead answer-delivery attempt ready for a physical submit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousHandoffDeliveryAttempt {
    pub handoff_id: String,
    pub issue_number: u64,
    pub session_id: String,
    pub attempt: u32,
    pub prompt_sha256: String,
    /// Protected prompt whose trailing marker authenticates every field above.
    pub prompt: String,
}

/// Result of atomically preparing an answered handoff for delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutonomousHandoffDeliveryPreparation {
    /// The `Attempting` write-ahead fence is durable; physical submit may begin.
    Ready(AutonomousHandoffDeliveryAttempt),
    /// A definitely-not-submitted retry remains inside its backoff window.
    Backoff {
        handoff_id: String,
        session_id: String,
        attempt: u32,
        retry_not_before: String,
    },
    /// The exact target materializer is still alive. Callers wait for its
    /// semantic receipt instead of treating a concurrent scan as a crash.
    InFlight {
        handoff_id: String,
        session_id: String,
        attempt: u32,
        target_session_id: String,
    },
    /// An unresolved write-ahead attempt was observed again. No prompt is
    /// returned, so an automatic replay is impossible.
    Ambiguous {
        handoff_id: String,
        session_id: String,
        attempt: u32,
        reason: String,
    },
}

/// Result of a failure proved to have happened before physical submit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutonomousHandoffDeliveryFailureOutcome {
    Retry {
        attempt: u32,
        retry_not_before: String,
    },
    Escalated {
        attempt: u32,
        reason: String,
    },
    /// The reported identity no longer owns the current Attempting fence.
    Rejected,
}

/// Issue #3478 (AC-9): status-view projection of one waiting question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousPendingQuestion {
    pub handoff_id: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Machine-readable
    /// [`AutonomousHandoffReason`](crate::autonomous_handoff::AutonomousHandoffReason) code.
    pub reason_code: String,
    pub session_id: String,
    pub provider: String,
    pub created_at: String,
    /// Whether registering an answer can resume the stored session.
    pub resumable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutonomousRecordRebasePolicy {
    DiskAuthoritative,
    LocalSameKeyAuthoritative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingLaunchDeliveryMatch {
    Matched(usize),
    Missing,
    Mismatched,
}

/// Monotonic progress of one immutable human-answer identity. A new answer is
/// ordered separately by `answer_revision`; within one answer, disk writers
/// may only advance this tuple and can never make a submitted prompt replayable.
fn autonomous_handoff_delivery_progress(delivery: &AutonomousHandoffDeliveryState) -> (u32, u8) {
    match delivery {
        AutonomousHandoffDeliveryState::Pending => (0, 0),
        AutonomousHandoffDeliveryState::Attempting {
            attempt, target, ..
        } => (*attempt, if target.is_some() { 2 } else { 1 }),
        AutonomousHandoffDeliveryState::RetryBackoff { attempt, .. } => (*attempt, 3),
        AutonomousHandoffDeliveryState::Ambiguous { attempt, .. } => (*attempt, 4),
        AutonomousHandoffDeliveryState::Exhausted { attempt, .. } => (*attempt, 5),
        // An authenticated provider receipt is definitive and must dominate a
        // concurrent restart/exit observer that can only infer ambiguity.
        AutonomousHandoffDeliveryState::Delivered { attempt, .. } => (*attempt, 6),
    }
}

pub fn is_auto_improve_candidate(issue: &IssueMonitorIssue, config: &IssueMonitorConfig) -> bool {
    let _ = config;
    issue.state == IssueMonitorIssueState::Open
}

const ISSUE_MONITOR_NOT_READY_REASON: &str = "plan/tasks の整備が必要（gwt-plan-spec）";

fn issue_monitor_candidate_exclusion(
    issue: &IssueMonitorIssue,
) -> Option<(MonitorInboxState, String)> {
    if let Some(label) = issue
        .labels
        .iter()
        .find(|label| label.eq_ignore_ascii_case("hold"))
    {
        return Some((
            MonitorInboxState::HoldExcluded,
            format!("hold label: {label}"),
        ));
    }
    if issue
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("gwt-spec"))
        && !issue.readiness.is_launch_ready()
    {
        return Some((
            MonitorInboxState::NotReady,
            ISSUE_MONITOR_NOT_READY_REASON.to_string(),
        ));
    }
    None
}

/// SPEC #3200 FR-003/004/005: routing decision for whether an open Issue may be
/// resolved by the autonomous (unattended) path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityDecision {
    /// Two-stage opt-in satisfied and every safety precondition holds — the
    /// Issue may be resolved autonomously.
    Eligible,
    /// The two-stage opt-in is NOT satisfied (autonomous_mode off OR no
    /// `auto-merge` label) — fall back to the existing SPEC #3165 human-gated
    /// flow unchanged.
    HumanGate(String),
    /// Issue #3944 AC-4: two-stage opt-in IS satisfied but a readiness
    /// precondition failed (no machine-checkable criteria, or branch protection
    /// not verified) — the row is `not_ready` with the named reason and is
    /// re-evaluated on the next scan; never `needs_human`.
    NotReady(String),
    /// The row is already parked for a human (`NeedsHuman` phase); the
    /// predicate never revives it. Issue #3944 AC-1: this is the only
    /// `NeedsHuman` the predicate reports, and it creates no new park.
    NeedsHuman(String),
}

/// Pure autonomous-eligibility predicate (SPEC #3200 FR-003).
///
/// Routing (the negatives matter as much as the positive):
/// - missing (i) `autonomous_mode` or (ii) the `auto-merge` label ⇒ `HumanGate`
///   — these two-stage-opt-in negatives use the existing #3165 gate, NOT
///   `NeedsHuman`.
/// - already needs-human ⇒ `NeedsHuman(reason)` (stays parked).
/// - missing (iii) machine-checkable criteria or (iv) verified branch
///   protection ⇒ `NotReady(reason)` (Issue #3944 AC-4).
/// - all satisfied ⇒ `Eligible`. Issue #3944 AC-2: an exhausted attempt
///   counter no longer gates here — the capped retry backoff is the throttle.
pub fn autonomous_eligibility(
    autonomous_mode: bool,
    has_auto_merge_label: bool,
    criteria: &crate::issue_monitor_gate::AcceptanceCriteria,
    protection: &gwt_git::branch_protection::BranchProtectionStatus,
    is_needs_human: bool,
) -> EligibilityDecision {
    // Stage 1 — two-stage opt-in. Either negative falls back to the existing
    // human-gated #3165 behavior, NOT to needs-human.
    if !autonomous_mode {
        return EligibilityDecision::HumanGate("autonomous_mode is off".to_string());
    }
    if !has_auto_merge_label {
        return EligibilityDecision::HumanGate("issue lacks the auto-merge label".to_string());
    }
    if is_needs_human {
        return EligibilityDecision::NeedsHuman("already escalated to needs-human".to_string());
    }
    // Stage 2 — readiness preconditions. Opt-in is satisfied, but the Issue or
    // the repository is not ready for an unattended run: name the cause and
    // re-check on the next scan (Issue #3944 AC-4).
    if !criteria.machine_checkable {
        // Issue #3930 AC-2: name the element that is actually missing. A
        // `not_ready` row is re-evaluated on every scan, so fixing the body is
        // the whole remedy.
        let missing = criteria
            .rejection_reason()
            .unwrap_or_else(|| "no machine-checkable acceptance criteria".to_string());
        return EligibilityDecision::NotReady(format!(
            "{missing}; the row is re-evaluated on the next scan"
        ));
    }
    if !protection.is_verified() {
        let reason = match protection {
            gwt_git::branch_protection::BranchProtectionStatus::Unreadable(detail) => {
                format!("branch protection could not be verified (permissions): {detail}")
            }
            _ => "branch protection absent or structurally insufficient".to_string(),
        };
        return EligibilityDecision::NotReady(reason);
    }
    EligibilityDecision::Eligible
}

/// SPEC #3200 T-022: lifecycle phase of one issue's current autonomous attempt.
/// Observable via the status view so every decision boundary is testable
/// (FR-033). `Idle` is the resting state between attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousPhase {
    /// No attempt in flight (never launched, or reset after merge/escalation).
    #[default]
    Idle,
    /// Implementation agent launched and producing changes.
    Implementing,
    /// Implementation complete; independent review / strong gate in flight.
    Reviewing,
    /// Gate passed; Deliver is driving the PR to merge.
    Delivering,
    /// Work merged — terminal success for the autonomous path.
    Merged,
    /// Parked for a human decision. Issue #3944: only a destructive-change
    /// approval or a user choice reaches this phase (see [`NeedsHumanKind`]).
    NeedsHuman,
}

/// Issue #3944 AC-1: the only two reasons an autonomous Issue may be parked in
/// `needs_human`. Every other stop cause (stuck/idle, attempts exhausted,
/// launch failure, readiness, CI, review, branch protection) is an automatic
/// recovery: a steering request to the PM, a requeue, or `not_ready`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsHumanKind {
    /// AC-5: a destructive or irreversible change needs the user's approval.
    DestructiveChangeApproval,
    /// A spec decision only the user can make.
    UserChoiceRequired,
}

impl NeedsHumanKind {
    /// Operator-facing English label used as the reason-line prefix.
    pub fn label(self) -> &'static str {
        match self {
            Self::DestructiveChangeApproval => "destructive change approval required",
            Self::UserChoiceRequired => "user choice required",
        }
    }

    /// Stable machine-readable code (matches the serialized form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DestructiveChangeApproval => "destructive_change_approval",
            Self::UserChoiceRequired => "user_choice_required",
        }
    }
}

/// Issue #3944 AC-2: the Issue Monitor's request that the PM steer a launch it
/// will not tear down (a live but silent window) or has just requeued past
/// its attempt cap. Read from `issue.monitor.status`; cleared by the agent's
/// next liveness signal or by the next launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousSteeringRequest {
    pub reason: String,
    /// RFC3339 of the latest request. Also anchors stuck detection so a live
    /// window is re-flagged once per `stuck_timeout_secs`, not every scan.
    pub requested_at: String,
    /// How many times this launch has been flagged without progress.
    pub count: u32,
}

/// SPEC #3200 T-022/T-016/T-018: the typed container for one issue's autonomous
/// state. Single source of truth for the attempt counter (FR-026), the current
/// lifecycle phase, the launch id binding the in-flight attempt (TOCTOU /
/// stuck-detection anchor, FR-013), and the launch-time acceptance snapshot
/// (FR-014). Persisted via [`IssueMonitorPrefs`] so a daemon restart never
/// resets an in-flight attempt's counter or loses its snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousIssueRecord {
    pub issue_number: u64,
    #[serde(default)]
    pub phase: AutonomousPhase,
    /// Launch / window id binding the CURRENT attempt. `None` between attempts.
    #[serde(default)]
    pub active_launch_id: Option<String>,
    /// Failed/started attempts so far (the persisted attempt counter).
    #[serde(default)]
    pub attempts: u32,
    /// Acceptance-criteria snapshot captured at launch; compared at gate time.
    #[serde(default)]
    pub acceptance_snapshot: Option<crate::issue_monitor_gate::AcceptanceSnapshot>,
    /// SPEC #3200 T-043/FR-029: earliest RFC3339 time the issue may relaunch
    /// after a transient retry was scheduled (bounded backoff). `None` ⇒ ready.
    #[serde(default)]
    pub retry_not_before: Option<String>,
    /// Issue #3616: why `retry_not_before` is set, when the cause is something
    /// a reader must act on differently than an ordinary backoff — today, a
    /// provider quota block. `None` for the plain retry ladder, whose reason is
    /// already implied by the attempt counter.
    #[serde(default)]
    pub retry_hold_reason: Option<String>,
    /// Canonical provider that owns this retry floor when it came from an
    /// account-wide quota hold. Missing legacy values stay untyped and never
    /// authorize a healthy-provider bypass by deadline coincidence alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_hold_provider: Option<String>,
    /// SPEC #3200 T-044/T-045/FR-013: RFC3339 of the last observed liveness
    /// signal from the launched agent — the anchor for stuck/idle detection.
    #[serde(default)]
    pub last_heartbeat: Option<String>,
    /// SPEC #3200: the open PR number produced by the implementation agent, set
    /// when the loop transitions Implementing→Reviewing. `None` until a PR exists.
    #[serde(default)]
    pub pr_number: Option<u64>,
    /// SPEC #3200 FR-016: the SHA the independent review evaluated and the gate is
    /// bound to (TOCTOU anchor). Set at Reviewing; checked against the merged SHA.
    #[serde(default)]
    pub reviewed_sha: Option<String>,
    /// SPEC #3200 FR-015: the independent-review verdict for `reviewed_sha`.
    /// `None` while review is in flight; `Some(true/false)` once it returns.
    #[serde(default)]
    pub review_passed: Option<bool>,
    /// Issue #3844: the agent's own statement that its silence is a wait, not
    /// a stall. While in force (see [`AUTONOMOUS_WAIT_MAX_SECS`]) stuck/idle
    /// detection skips this record. Cleared when the launch ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<AutonomousWaitDeclaration>,
    /// Issue #3944 AC-1: why the row is parked, when `phase` is `NeedsHuman`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_human_kind: Option<NeedsHumanKind>,
    /// Issue #3944 AC-2: the open steering request for this launch, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering: Option<AutonomousSteeringRequest>,
}

/// Issue #3844: what a launched agent declared it is waiting for.
///
/// Waiting is not progress, but it is also not a stall: an agent parked on a
/// host-exclusivity turn, a PM serialization ruling, or a long verify run it
/// must not touch produces no heartbeat and is exactly as healthy as a busy
/// one. The declaration carries the reason and the resume condition so the
/// PM can read *what* is being waited for from `issue.monitor.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousWaitDeclaration {
    pub reason: String,
    pub resume_condition: String,
    /// RFC3339 of the first declaration of this continuous wait. Re-declaring
    /// updates the text but never this anchor, so renewals cannot chain past
    /// [`AUTONOMOUS_WAIT_MAX_SECS`].
    pub since: String,
    /// RFC3339 of the latest (re)declaration.
    pub declared_at: String,
}

/// Issue #3844 AC-3: how long one continuous wait declaration suspends stuck
/// detection. Sized for a serialized verification queue of a few predecessors
/// at 30–40 minutes each; past it the ordinary `stuck_timeout_secs` rule
/// applies to the record's heartbeat again, so a declaration can never keep a
/// genuinely dead agent alive indefinitely.
pub const AUTONOMOUS_WAIT_MAX_SECS: u64 = 3 * 60 * 60;

/// Outcome of a wait declaration or clearing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutonomousWaitOutcome {
    /// The wait is recorded; `expires_at` is when it stops protecting the record.
    Declared {
        since: String,
        expires_at: String,
    },
    Cleared,
    /// Clearing found no declaration to clear.
    NotWaiting,
    /// Declaring is refused: only a launch that is running can be waiting.
    NotLaunched,
}

/// Issue #3844 AC-2: the wait declaration as the PM reads it from an inbox row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorWaitSummary {
    pub reason: String,
    pub resume_condition: String,
    pub since: String,
    /// RFC3339 after which the declaration no longer suspends stuck detection.
    pub expires_at: String,
}

fn autonomous_wait_expires_at(since: &str) -> String {
    rfc3339_plus_secs(since, AUTONOMOUS_WAIT_MAX_SECS).unwrap_or_else(|| since.to_string())
}

/// Whether `record`'s wait declaration still suspends stuck detection at `now`.
/// An unparseable anchor fails closed (not in force).
fn autonomous_wait_in_force(record: &AutonomousIssueRecord, now: &str) -> bool {
    record
        .wait
        .as_ref()
        .and_then(|wait| rfc3339_elapsed_secs(&wait.since, now))
        .is_some_and(|elapsed| elapsed < AUTONOMOUS_WAIT_MAX_SECS as i64)
}

/// Issue #3944 AC-2: seconds since the record's latest liveness anchor — the
/// agent's last heartbeat or the monitor's last steering request, whichever
/// is later. `None` with no anchor at all (never judged stuck).
fn autonomous_liveness_anchor_elapsed_secs(
    record: &AutonomousIssueRecord,
    now: &str,
) -> Option<i64> {
    let heartbeat = record
        .last_heartbeat
        .as_deref()
        .and_then(|heartbeat| rfc3339_elapsed_secs(heartbeat, now));
    let steering = record
        .steering
        .as_ref()
        .and_then(|steering| rfc3339_elapsed_secs(&steering.requested_at, now));
    match (heartbeat, steering) {
        (Some(heartbeat), Some(steering)) => Some(heartbeat.min(steering)),
        (Some(elapsed), None) | (None, Some(elapsed)) => Some(elapsed),
        (None, None) => None,
    }
}

/// SPEC #3200 T-043/FR-029: bounded exponential backoff (seconds) for the
/// `attempt`-th transient retry. attempt 1 ⇒ `base_secs`, doubling each
/// subsequent attempt, clamped to `cap_secs`. Saturating arithmetic so large
/// attempt counts never overflow or panic on shift.
pub fn autonomous_retry_backoff_secs(attempt: u32, base_secs: u64, cap_secs: u64) -> u64 {
    let exponent = attempt.saturating_sub(1).min(32);
    let scaled = base_secs.saturating_mul(1u64 << exponent);
    scaled.min(cap_secs)
}

/// Add `secs` to an RFC3339 instant, returning the new RFC3339 string. `None`
/// when `now` is not parseable as RFC3339.
fn rfc3339_plus_secs(now: &str, secs: u64) -> Option<String> {
    // Guard the u64→i64 cast: an absurd magnitude (only possible via corrupted
    // tuning) fails closed to None rather than wrapping negative.
    let secs = i64::try_from(secs).ok()?;
    let parsed = chrono::DateTime::parse_from_rfc3339(now).ok()?;
    let later = parsed + chrono::Duration::seconds(secs);
    Some(later.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// Whole seconds elapsed from `earlier` to `now` (both RFC3339). `None` when
/// either is unparseable. Negative when `now` precedes `earlier`.
fn rfc3339_elapsed_secs(earlier: &str, now: &str) -> Option<i64> {
    let a = chrono::DateTime::parse_from_rfc3339(earlier).ok()?;
    let b = chrono::DateTime::parse_from_rfc3339(now).ok()?;
    Some((b - a).num_seconds())
}

/// SPEC #3200 Option A: a request for the GUI to spawn an independent review
/// agent. The GUI launches a fresh-session, different-model agent with a prompt
/// built from `required_criteria` + `diff`, bound to `reviewed_sha`; that agent
/// returns its verdict via the `ReviewVerdict` control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousReviewDispatch {
    pub issue_number: u64,
    pub pr_number: u64,
    pub reviewed_sha: String,
    pub required_criteria: Vec<String>,
    pub diff: String,
    /// SPEC #3200 Option A: the work branch kind, so the GUI spawns the review
    /// agent in the implementation agent's existing work-branch worktree.
    #[serde(default)]
    pub linked_issue_kind: LinkedIssueKind,
}

/// SPEC #3200 FR-034 (T-111): one operator notice for an unattended autonomous
/// lifecycle transition. Surfaced to the GUI as an `issue_monitor_toast`
/// (transient surface toast + persistent scrollable notification stack).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousNotice {
    /// Toast level: `info` | `warn` | `error` | `done`.
    pub level: String,
    pub issue_number: u64,
    pub message: String,
}

/// Bound for [`IssueMonitorState::pending_autonomous_notices`]: unattended
/// operation with no GUI connected must not grow the queue without limit.
const AUTONOMOUS_NOTICE_CAP: usize = 100;

/// SPEC #3200 T-042 / Issue #3944 AC-2: the routing outcome of dispatching an
/// autonomous failure or stall. Neither outcome parks the Issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutonomousFailureOutcome {
    /// Re-queued for another attempt; carries the new attempt count. At or
    /// past `max_attempts` the ladder continues at its capped backoff and the
    /// record carries a steering request for the PM.
    Retry { attempt: u32 },
    /// The launch is alive and stays in place: no slot reclaimed, no attempt
    /// consumed; the PM is asked to steer it (`count` requests so far).
    SteeringRequested { count: u32 },
}

impl AutonomousIssueRecord {
    /// A fresh record for `issue_number` with no attempts, holds, or snapshot.
    pub fn new(issue_number: u64) -> Self {
        Self {
            issue_number,
            phase: AutonomousPhase::Idle,
            active_launch_id: None,
            attempts: 0,
            acceptance_snapshot: None,
            retry_not_before: None,
            retry_hold_reason: None,
            retry_hold_provider: None,
            last_heartbeat: None,
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
            wait: None,
            needs_human_kind: None,
            steering: None,
        }
    }
}

/// SPEC #3200 FR-004: the GitHub label that, together with project-level
/// `autonomous_mode`, opts an issue into unattended autonomous resolution.
pub const AUTO_MERGE_LABEL: &str = "auto-merge";

/// Whether `issue` carries the [`AUTO_MERGE_LABEL`] (case-insensitive).
pub fn issue_has_auto_merge_label(issue: &IssueMonitorIssue) -> bool {
    issue
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(AUTO_MERGE_LABEL))
}

pub fn issue_monitor_linked_issue_kind(issue: &IssueMonitorIssue) -> LinkedIssueKind {
    if has_gwt_spec_label(&issue.labels) {
        LinkedIssueKind::Spec
    } else {
        LinkedIssueKind::Issue
    }
}

pub fn issue_monitor_launch_prompt(kind: LinkedIssueKind, number: u64) -> String {
    match kind {
        LinkedIssueKind::Spec => format!("$gwt-execute #{number}"),
        LinkedIssueKind::Issue => format!(
            "$gwt-execute #{number}\n\nBefore changing code, evaluate every remaining acceptance criterion. If all criteria are already satisfied, close Issue #{number} and finish without creating another implementation PR. Otherwise, implement only the remaining criteria."
        ),
    }
}

pub fn issue_monitor_launch_plan(issue: &IssueMonitorIssue) -> IssueMonitorLaunchPlan {
    let linked_issue_kind = issue_monitor_linked_issue_kind(issue);
    IssueMonitorLaunchPlan {
        branch_name: knowledge_launch_target_branch_name(linked_issue_kind, issue.number),
        linked_issue_kind,
        prompt: issue_monitor_launch_prompt(linked_issue_kind, issue.number),
    }
}

fn load_issue_monitor_prefs_unlocked(path: &Path) -> io::Result<IssueMonitorPrefs> {
    if !path.exists() {
        return Ok(IssueMonitorPrefs::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        let kind = match error.classify() {
            serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
                io::ErrorKind::InvalidData
            }
            serde_json::error::Category::Data => io::ErrorKind::InvalidInput,
            serde_json::error::Category::Io => io::ErrorKind::Other,
        };
        io::Error::new(kind, error)
    })
}

/// Per-process-unique scratch path for the atomic prefs write, placed in the
/// same directory as `path` (so the final `rename` stays on one filesystem and
/// is atomic). The daemon (`gwtd`) and GUI (`gwt`) processes both write this same
/// prefs file; a fixed `*.json.tmp` name let their concurrent writes open and
/// truncate the SAME scratch file and interleave into torn JSON, which
/// `load_issue_monitor_prefs` then silently reset to default (adversarial
/// review). Scoping the scratch name to `{pid}-{uuid}` gives every writer its own
/// file. Mirrors the gwt-core atomic-write convention.
fn unique_prefs_tmp_path(path: &Path) -> std::path::PathBuf {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("issue-monitor.json");
    parent.join(format!(
        ".{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

fn unique_corrupt_prefs_path(path: &Path) -> std::path::PathBuf {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("issue-monitor.json");
    parent.join(format!(
        "{file_name}.corrupt-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    // Windows does not support opening a directory through std::fs::File.
    // The scratch file itself is still sync_all'd before the atomic rename;
    // match the repository's other durable writers by treating directory
    // metadata sync as an explicit compatibility no-op off Unix.
    Ok(())
}

fn durable_atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let tmp = unique_prefs_tmp_path(path);
    let result = (|| {
        let mut scratch = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        scratch.write_all(content)?;
        scratch.sync_all()?;
        // Deadline-aware transactions may spend their remaining budget on
        // serialization or scratch fsync. Recheck immediately before the
        // canonical rename so an expired proposal cannot become visible after
        // its acceptance boundary.
        gwt_core::operation_deadline::ensure_remaining("Issue Monitor durable rename")?;
        fs::rename(&tmp, path)?;
        #[cfg(test)]
        if std::env::var_os("GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE")
            .is_some_and(|target| Path::new(&target) == path)
        {
            let fail_once = path.with_extension("parent-sync-fail-once");
            if fail_once.exists() {
                fs::remove_file(fail_once)?;
                return Err(io::Error::other(
                    "injected Issue Monitor prefs parent sync failure after rename",
                ));
            }
        }
        sync_parent_directory(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn save_issue_monitor_prefs_unlocked(path: &Path, prefs: &IssueMonitorPrefs) -> io::Result<()> {
    let content = serde_json::to_string_pretty(prefs).map_err(io::Error::other)?;
    durable_atomic_write(path, content.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorAuthorityFence {
    pub version: u32,
    pub pid: u32,
    pub instance_id: String,
}

impl IssueMonitorAuthorityFence {
    pub fn current_process() -> Self {
        Self {
            version: ISSUE_MONITOR_AUTHORITY_FENCE_VERSION,
            pid: std::process::id(),
            instance_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}

/// Process-lifetime ownership of the Issue Monitor authority lock.
///
/// The stable sibling file stays on disk; ownership is the kernel lock held by
/// this handle and is released automatically on process exit or lease drop.
#[must_use = "dropping the lease releases Issue Monitor effect authority"]
#[derive(Debug)]
pub struct IssueMonitorAuthorityLease {
    lock: fs::File,
}

impl Drop for IssueMonitorAuthorityLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorAuthorityFenceState {
    Missing,
    LegacyShutdownRevoke,
    Active(IssueMonitorAuthorityFence),
}

pub fn issue_monitor_authority_fence_path(prefs_path: &Path) -> std::path::PathBuf {
    prefs_path.with_extension("shutdown-revoke")
}

fn issue_monitor_authority_lock_path(prefs_path: &Path) -> std::path::PathBuf {
    prefs_path.with_extension("authority.lock")
}

pub fn load_issue_monitor_authority_fence(
    prefs_path: &Path,
) -> io::Result<IssueMonitorAuthorityFenceState> {
    let path = issue_monitor_authority_fence_path(prefs_path);
    let content = match fs::read(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(IssueMonitorAuthorityFenceState::Missing);
        }
        Err(error) => return Err(error),
    };
    if content == LEGACY_SHUTDOWN_REVOKE_FENCE {
        return Ok(IssueMonitorAuthorityFenceState::LegacyShutdownRevoke);
    }
    let fence: IssueMonitorAuthorityFence = serde_json::from_slice(&content).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("parse Issue Monitor authority fence failed: {error}"),
        )
    })?;
    if ![
        LEGACY_ISSUE_MONITOR_AUTHORITY_FENCE_VERSION,
        ISSUE_MONITOR_AUTHORITY_FENCE_VERSION,
    ]
    .contains(&fence.version)
        || fence.pid == 0
        || fence.instance_id.trim().is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Issue Monitor authority fence has invalid identity fields",
        ));
    }
    Ok(IssueMonitorAuthorityFenceState::Active(fence))
}

pub fn persist_issue_monitor_authority_fence(
    prefs_path: &Path,
    fence: &IssueMonitorAuthorityFence,
) -> io::Result<()> {
    let content = serde_json::to_vec_pretty(fence).map_err(io::Error::other)?;
    durable_atomic_write(&issue_monitor_authority_fence_path(prefs_path), &content)
}

pub fn persist_legacy_issue_monitor_shutdown_revoke_fence(prefs_path: &Path) -> io::Result<()> {
    durable_atomic_write(
        &issue_monitor_authority_fence_path(prefs_path),
        LEGACY_SHUTDOWN_REVOKE_FENCE,
    )
}

/// Establish durable effect authority and hold its process-lifetime lease.
///
/// Current v2 fences use the kernel lock as their liveness identity, so a free
/// lock makes even a same-PID fence stale and recoverable. Legacy v1 owners did
/// not hold this lock; their PID probe therefore remains fail-closed until the
/// old process exits.
pub fn establish_issue_monitor_authority_fence(
    prefs_path: &Path,
    current: &IssueMonitorAuthorityFence,
    is_process_alive: impl Fn(u32) -> bool,
) -> io::Result<(IssueMonitorPrefs, IssueMonitorAuthorityLease)> {
    if current.version != ISSUE_MONITOR_AUTHORITY_FENCE_VERSION
        || current.pid == 0
        || current.instance_id.trim().is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "current Issue Monitor authority fence must use the current schema and a valid identity",
        ));
    }
    with_issue_monitor_prefs_lock(prefs_path, || {
        let authority_lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(issue_monitor_authority_lock_path(prefs_path))?;
        if let Err(error) = FileExt::try_lock_exclusive(&authority_lock) {
            if gwt_core::operation_deadline::is_lock_contended(&error) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Issue Monitor authority lifetime lease is already held by another daemon",
                ));
            }
            return Err(error);
        }
        let lease = IssueMonitorAuthorityLease {
            lock: authority_lock,
        };
        let mut prefs = load_issue_monitor_prefs_unlocked(prefs_path)?;
        match load_issue_monitor_authority_fence(prefs_path)? {
            IssueMonitorAuthorityFenceState::Missing => {
                persist_issue_monitor_authority_fence(prefs_path, current)?;
            }
            IssueMonitorAuthorityFenceState::Active(existing)
                if existing.version == LEGACY_ISSUE_MONITOR_AUTHORITY_FENCE_VERSION
                    && is_process_alive(existing.pid) =>
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "legacy Issue Monitor authority fence is owned by live daemon pid {}",
                        existing.pid
                    ),
                ));
            }
            IssueMonitorAuthorityFenceState::LegacyShutdownRevoke
            | IssueMonitorAuthorityFenceState::Active(_) => {
                prefs.advance_effect_authority_epoch().ok_or_else(|| {
                    io::Error::other("Issue Monitor authority epoch exhausted during fence replay")
                })?;
                save_issue_monitor_prefs_unlocked(prefs_path, &prefs)?;
                persist_issue_monitor_authority_fence(prefs_path, current)?;
            }
        }
        Ok((prefs, lease))
    })
}

/// Acquire short-lived Issue Monitor effect authority for the GUI fallback.
///
/// The local driver deliberately does not publish a durable daemon fence: it
/// owns only one bounded scan/effect pass. Holding the same lifetime lock as a
/// daemon closes the gap between the fence pre-check and remote submission,
/// while the prefs lock preserves the daemon's `prefs.lock -> authority.lock`
/// ordering. A free lifetime lock proves a v2 fence is stale, so the fallback
/// revokes its epoch and removes it before proceeding. Legacy fences remain
/// fail-closed because they did not carry lifetime-lock liveness.
pub fn try_acquire_issue_monitor_local_fallback_lease(
    prefs_path: &Path,
) -> io::Result<IssueMonitorAuthorityLease> {
    with_issue_monitor_prefs_lock(prefs_path, || {
        let authority_lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(issue_monitor_authority_lock_path(prefs_path))?;
        if let Err(error) = FileExt::try_lock_exclusive(&authority_lock) {
            if gwt_core::operation_deadline::is_lock_contended(&error) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Issue Monitor daemon authority lease is already held",
                ));
            }
            return Err(error);
        }
        let lease = IssueMonitorAuthorityLease {
            lock: authority_lock,
        };
        match load_issue_monitor_authority_fence(prefs_path)? {
            IssueMonitorAuthorityFenceState::Missing => Ok(lease),
            IssueMonitorAuthorityFenceState::Active(existing)
                if existing.version == ISSUE_MONITOR_AUTHORITY_FENCE_VERSION =>
            {
                let mut prefs = load_issue_monitor_prefs_unlocked(prefs_path)?;
                prefs.advance_effect_authority_epoch().ok_or_else(|| {
                    io::Error::other(
                        "Issue Monitor authority epoch exhausted during local fence recovery",
                    )
                })?;
                save_issue_monitor_prefs_unlocked(prefs_path, &prefs)?;
                let fence_path = issue_monitor_authority_fence_path(prefs_path);
                fs::remove_file(&fence_path)?;
                sync_parent_directory(&fence_path)?;
                Ok(lease)
            }
            IssueMonitorAuthorityFenceState::LegacyShutdownRevoke
            | IssueMonitorAuthorityFenceState::Active(_) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Issue Monitor daemon authority fence excludes the local fallback",
            )),
        }
    })
}

pub fn clear_issue_monitor_authority_fence(
    prefs_path: &Path,
    expected: &IssueMonitorAuthorityFence,
) -> io::Result<()> {
    with_issue_monitor_prefs_lock(prefs_path, || {
        match load_issue_monitor_authority_fence(prefs_path)? {
            IssueMonitorAuthorityFenceState::Active(current) if current == *expected => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Issue Monitor authority fence identity changed before clear",
                ));
            }
        }
        let path = issue_monitor_authority_fence_path(prefs_path);
        fs::remove_file(&path)?;
        #[cfg(test)]
        if let Some(fail_once) =
            std::env::var_os("GWT_TEST_FAIL_ISSUE_MONITOR_FENCE_PARENT_SYNC_ONCE")
        {
            let fail_once = std::path::PathBuf::from(fail_once);
            if fail_once.exists() {
                fs::remove_file(fail_once)?;
                return Err(io::Error::other(
                    "injected Issue Monitor authority fence parent sync failure",
                ));
            }
        }
        sync_parent_directory(&path)
    })
}

fn with_issue_monitor_prefs_lock<T>(
    path: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    with_issue_monitor_prefs_lock_observed(path, || {}, operation)
}

fn with_issue_monitor_prefs_lock_observed<T>(
    path: &Path,
    on_first_contention: impl FnMut(),
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    // Lock a stable sibling inode: locking `path` itself would stop protecting
    // future writers as soon as the atomic rename replaces that inode. A
    // compare/retry loop still has a check-to-rename TOCTOU, while making the
    // daemon the sole writer is beyond this compatibility migration's scope.
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    gwt_core::operation_deadline::lock_exclusive_with_observer(&lock, on_first_contention)?;
    let result = operation();
    let unlock_result = FileExt::unlock(&lock);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

pub fn load_issue_monitor_prefs(path: &Path) -> io::Result<IssueMonitorPrefs> {
    load_issue_monitor_prefs_unlocked(path)
}

/// Replace a complete prefs snapshot while holding the stable writer lock.
/// Production read-modify-write callers should use
/// [`mutate_issue_monitor_prefs_recovering`] so their mutation is based on the
/// latest committed snapshot under the same lock and malformed JSON cannot
/// permanently block future writes. [`mutate_issue_monitor_prefs`] remains
/// available for deliberately fail-closed callers and tests.
pub fn save_issue_monitor_prefs(path: &Path, prefs: &IssueMonitorPrefs) -> io::Result<()> {
    with_issue_monitor_prefs_lock(path, || save_issue_monitor_prefs_unlocked(path, prefs))
}

/// Serialize one cross-process prefs transaction from latest load through the
/// unique-scratch atomic save, returning both the committed snapshot and the
/// mutation's result. The closure must not call [`save_issue_monitor_prefs`]
/// recursively because this transaction already owns the sibling lock.
pub fn mutate_issue_monitor_prefs<T>(
    path: &Path,
    mutation: impl FnOnce(&mut IssueMonitorPrefs) -> T,
) -> io::Result<(IssueMonitorPrefs, T)> {
    with_issue_monitor_prefs_lock(path, || {
        let mut prefs = load_issue_monitor_prefs_unlocked(path)?;
        let result = mutation(&mut prefs);
        save_issue_monitor_prefs_unlocked(path, &prefs)?;
        Ok((prefs, result))
    })
}

/// Fail-closed prefs transaction whose mutation may reject before any save.
/// The stable sibling lock is held across latest-load, validation, and the
/// durable atomic writer. Returning `Err` from `mutation`, or returning the
/// same typed preferences unchanged, leaves the canonical bytes untouched and
/// creates no scratch write.
pub fn try_mutate_issue_monitor_prefs<T>(
    path: &Path,
    mutation: impl FnOnce(&mut IssueMonitorPrefs) -> io::Result<T>,
) -> io::Result<(IssueMonitorPrefs, T)> {
    with_issue_monitor_prefs_lock(path, || {
        let mut prefs = load_issue_monitor_prefs_unlocked(path)?;
        let before = prefs.clone();
        let result = mutation(&mut prefs)?;
        if prefs != before {
            save_issue_monitor_prefs_unlocked(path, &prefs)?;
        }
        Ok((prefs, result))
    })
}

/// Local fallback transaction ordered against daemon startup by the same
/// stable prefs lock. A daemon that establishes its lifetime authority fence
/// before this closure acquires the lock wins; the fallback then returns an
/// error before mutation or save. If the fallback wins, startup waits and sees
/// its committed prefs after the lock is released.
pub fn try_mutate_issue_monitor_prefs_without_authority_fence<T>(
    path: &Path,
    mutation: impl FnOnce(&mut IssueMonitorPrefs) -> io::Result<T>,
) -> io::Result<(IssueMonitorPrefs, T)> {
    with_issue_monitor_prefs_lock(path, || {
        match load_issue_monitor_authority_fence(path)? {
            IssueMonitorAuthorityFenceState::Missing => {}
            IssueMonitorAuthorityFenceState::LegacyShutdownRevoke
            | IssueMonitorAuthorityFenceState::Active(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Issue Monitor daemon authority fence appeared before local fallback commit",
                ));
            }
        }
        let mut prefs = load_issue_monitor_prefs_unlocked(path)?;
        let result = mutation(&mut prefs)?;
        save_issue_monitor_prefs_unlocked(path, &prefs)?;
        Ok((prefs, result))
    })
}

/// Serialize one prefs transaction while recovering a malformed JSON snapshot.
///
/// Only JSON syntax/EOF failures are recoverable. The malformed bytes are
/// copied to a unique sibling quarantine file while the stable writer lock is
/// held, then `recovery_baseline` is mutated and atomically committed. Schema
/// data errors and other I/O errors remain fail-closed. Callers should pass
/// their latest valid in-memory prefs; callers without one should explicitly
/// pass [`IssueMonitorPrefs::recovery_default`].
pub fn mutate_issue_monitor_prefs_recovering<T>(
    path: &Path,
    recovery_baseline: &IssueMonitorPrefs,
    mutation: impl FnOnce(&mut IssueMonitorPrefs) -> T,
) -> io::Result<(IssueMonitorPrefs, T)> {
    mutate_issue_monitor_prefs_recovering_observed(path, recovery_baseline, || {}, mutation)
}

pub(crate) fn mutate_issue_monitor_prefs_recovering_observed<T>(
    path: &Path,
    recovery_baseline: &IssueMonitorPrefs,
    on_first_contention: impl FnMut(),
    mutation: impl FnOnce(&mut IssueMonitorPrefs) -> T,
) -> io::Result<(IssueMonitorPrefs, T)> {
    with_issue_monitor_prefs_lock_observed(path, on_first_contention, || {
        let (mut prefs, recovery) = match load_issue_monitor_prefs_unlocked(path) {
            Ok(prefs) => (prefs, None),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                let quarantine = unique_corrupt_prefs_path(path);
                fs::copy(path, &quarantine)?;
                (recovery_baseline.clone(), Some((quarantine, error)))
            }
            Err(error) => return Err(error),
        };
        let result = mutation(&mut prefs);
        save_issue_monitor_prefs_unlocked(path, &prefs)?;
        if let Some((quarantine, error)) = recovery {
            tracing::warn!(
                path = %path.display(),
                quarantine = %quarantine.display(),
                error = %error,
                "recovered malformed issue monitor prefs"
            );
        }
        Ok((prefs, result))
    })
}

pub fn issue_monitor_prefs_path_for_repo_path(repo_path: &Path) -> std::path::PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("project-state/issue-monitor.json")
}

/// Issue #3478 (AC-5): the prompt delivered to the resumed session. It restates
/// the exact question the agent asked and the human's answer, so the resumed
/// conversation regains the decision context it was parked on.
pub fn autonomous_handoff_answer_prompt(handoff: &AutonomousQuestionHandoff) -> String {
    let mut prompt = format!(
        "Your autonomous execution for Issue #{issue} was parked on a question that needed human judgment.\n\n\
Question you asked:\n{question}\n",
        issue = handoff.issue_number,
        question = handoff.question,
    );
    if !handoff.options.is_empty() {
        prompt.push_str("\nOptions you offered:\n");
        for option in &handoff.options {
            if option.description.is_empty() {
                prompt.push_str(&format!("- {}\n", option.label));
            } else {
                prompt.push_str(&format!("- {}: {}\n", option.label, option.description));
            }
        }
    }
    prompt.push_str(&format!(
        "\nHuman answer:\n{answer}\n\nDecision recorded at:\n{answered_at}\n\nContinue the work with this answer. Do not re-ask it.",
        answer = handoff.answer.as_deref().unwrap_or(""),
        answered_at = handoff.answered_at.as_deref().unwrap_or("unknown"),
    ));
    prompt
}

/// Issue #3478 (AC-5): cross-process one-shot take of the answer prompt owed to
/// `issue_number`'s resumed launch.
///
/// The daemon un-parks the Issue but the GUI owns the launch, so the delivery
/// marker has to be committed to the shared control plane under its stable
/// lock — otherwise two launch attempts could both believe they own the answer.
pub fn take_autonomous_resume_prompt_from_prefs(
    prefs_path: &Path,
    issue_number: u64,
    now: &str,
) -> Option<String> {
    prepare_autonomous_handoff_delivery_from_prefs(prefs_path, issue_number, now)
        .map_err(|error| {
            tracing::warn!(
                error = %error,
                path = %prefs_path.display(),
                "failed to prepare autonomous resume prompt"
            );
        })
        .ok()
        .flatten()
        .and_then(|prepared| match prepared {
            AutonomousHandoffDeliveryPreparation::Ready(attempt) => Some(attempt.prompt),
            AutonomousHandoffDeliveryPreparation::Backoff { .. }
            | AutonomousHandoffDeliveryPreparation::InFlight { .. }
            | AutonomousHandoffDeliveryPreparation::Ambiguous { .. } => None,
        })
}

/// Issue #3716 (AC-2): inspect the unanswered resumption for an Issue without
/// consuming it. The asking gwt Session is part of the durable handoff and is
/// the routing authority; branch recency must never replace it.
pub fn pending_autonomous_handoff_resumption_from_prefs(
    prefs_path: &Path,
    issue_number: u64,
) -> io::Result<Option<AutonomousHandoffResumption>> {
    let prefs = load_issue_monitor_prefs(prefs_path)?;
    Ok(prefs
        .autonomous_handoffs
        .iter()
        .find(|handoff| {
            handoff.issue_number == issue_number
                && handoff.state == AutonomousHandoffState::Resumed
                && handoff.delivered_at.is_none()
                && matches!(
                    handoff.delivery,
                    AutonomousHandoffDeliveryState::Pending
                        | AutonomousHandoffDeliveryState::Attempting { .. }
                        | AutonomousHandoffDeliveryState::RetryBackoff { .. }
                )
        })
        .map(|handoff| AutonomousHandoffResumption {
            handoff_id: handoff.handoff_id.clone(),
            issue_number: handoff.issue_number,
            session_id: handoff.session_id.clone(),
            prompt: autonomous_handoff_answer_prompt(handoff),
        }))
}

/// Atomically write the `Attempting` fence before a caller may submit an
/// answered handoff. Re-reading an unresolved fence performs crash recovery:
/// it returns no prompt, transitions to Ambiguous/NeedsHuman, and removes all
/// slot/delivery ownership in the same prefs transaction.
pub fn prepare_autonomous_handoff_delivery_from_prefs(
    prefs_path: &Path,
    issue_number: u64,
    now: &str,
) -> io::Result<Option<AutonomousHandoffDeliveryPreparation>> {
    let (_, prepared) = try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let prepared = monitor.prepare_autonomous_handoff_delivery(issue_number, now);
        *prefs = monitor.prefs();
        Ok(prepared)
    })?;
    Ok(prepared)
}

/// Bind a prepared prompt to the exact gwt/native Session and materializer
/// that may submit it. This must commit after target Session creation and
/// before a process spawn or live-pane write crosses the provider boundary.
pub fn bind_autonomous_handoff_delivery_target_from_prefs(
    prefs_path: &Path,
    handoff_id: &str,
    source_session_id: &str,
    attempt: u32,
    target_identity: &AutonomousHandoffDeliveryTarget,
) -> io::Result<bool> {
    if target_identity.gwt_session_id.is_empty()
        || target_identity.native_session_id.is_empty()
        || target_identity.provider.is_empty()
        || target_identity.repo_hash.is_empty()
        || target_identity.project_state_root.is_empty()
        || target_identity.window_id.is_empty()
        || target_identity.materializer_id.is_empty()
        || target_identity.materializer_pid == 0
        || target_identity.materializer_started_at == 0
    {
        return Ok(false);
    }
    let (_, bound) = try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let Some(handoff) = prefs.autonomous_handoffs.iter_mut().find(|handoff| {
            handoff.handoff_id == handoff_id
                && handoff.session_id == source_session_id
                && handoff.issue_number == target_identity.issue_number
                && handoff.provider == target_identity.provider
                && handoff.state == AutonomousHandoffState::Resumed
        }) else {
            return Ok(false);
        };
        let AutonomousHandoffDeliveryState::Attempting {
            attempt: current_attempt,
            materializer_pid,
            materializer_started_at,
            target,
            ..
        } = &mut handoff.delivery
        else {
            return Ok(false);
        };
        if *current_attempt != attempt
            || *materializer_pid != target_identity.materializer_pid
            || *materializer_started_at != target_identity.materializer_started_at
        {
            return Ok(false);
        }
        match target {
            Some(current) => Ok(current.as_ref() == target_identity),
            None => {
                *target = Some(Box::new(target_identity.clone()));
                Ok(true)
            }
        }
    })?;
    Ok(bound)
}

/// Authenticated `UserPromptSubmit` receipt for an answered handoff.
///
/// Parsing happens before the lock only as an inexpensive rejection. The
/// current canonical prompt is rebuilt and hashed again while holding the
/// prefs lock, so a delayed receipt cannot acknowledge an updated answer.
pub fn acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
    prefs_path: &Path,
    source_session_id: &str,
    observed_target: &AutonomousHandoffReceiptIdentity,
    prompt: &str,
    now: &str,
) -> io::Result<bool> {
    let Some(marker) = parse_protected_autonomous_handoff_answer_prompt(prompt) else {
        return Ok(false);
    };
    if marker.session_id != source_session_id {
        return Ok(false);
    }
    let submitted_prompt = prompt.strip_suffix('\r').unwrap_or(prompt);
    let (_, acknowledged) = try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let Some(index) = monitor.autonomous_handoffs.iter().position(|handoff| {
            handoff.handoff_id == marker.handoff_id
                && handoff.session_id == marker.session_id
                && handoff.issue_number == observed_target.issue_number
                && handoff.provider == observed_target.provider
                && handoff.state == AutonomousHandoffState::Resumed
        }) else {
            return Ok(false);
        };
        let handoff = &monitor.autonomous_handoffs[index];
        let body = autonomous_handoff_answer_prompt(handoff);
        let Some(canonical_prompt) = protected_autonomous_handoff_answer_prompt(
            &body,
            &handoff.handoff_id,
            &handoff.session_id,
            marker.attempt,
        ) else {
            return Ok(false);
        };
        let Some(canonical_marker) =
            parse_protected_autonomous_handoff_answer_prompt(&canonical_prompt)
        else {
            return Ok(false);
        };
        if submitted_prompt != canonical_prompt
            || canonical_marker != marker
            || !matches!(
                &handoff.delivery,
                AutonomousHandoffDeliveryState::Attempting {
                    attempt,
                    prompt_sha256,
                    target: Some(target),
                    ..
                } if *attempt == marker.attempt
                    && prompt_sha256 == &marker.prompt_sha256
                    && target.gwt_session_id == observed_target.gwt_session_id
                    && target.native_session_id == observed_target.native_session_id
                    && target.provider == observed_target.provider
                    && target.issue_number == observed_target.issue_number
                    && target.repo_hash == observed_target.repo_hash
                    && target.project_state_root == observed_target.project_state_root
            )
        {
            return Ok(false);
        }
        let target = match &monitor.autonomous_handoffs[index].delivery {
            AutonomousHandoffDeliveryState::Attempting {
                target: Some(target),
                ..
            } => target.clone(),
            _ => return Ok(false),
        };
        let launch_already_durable = monitor
            .active_launches
            .contains(&observed_target.issue_number)
            && monitor
                .launched_windows
                .get(&observed_target.issue_number)
                .is_some_and(|window_id| {
                    issue_monitor_window_ids_match(window_id, &target.window_id)
                });
        let mut complete_launch = false;
        if let Some(delivery_id) = target.delivery_id.as_deref() {
            match monitor.match_pending_launch_delivery(observed_target.issue_number, delivery_id) {
                PendingLaunchDeliveryMatch::Matched(delivery_index) => {
                    let delivery = &monitor.pending_launch_deliveries[delivery_index];
                    if delivery.materializer_window_id.as_deref() != Some(target.window_id.as_str())
                    {
                        return Ok(false);
                    }
                    if delivery.materialized_window_id.as_deref() == Some(target.window_id.as_str())
                        && delivery.workspace_durable_window_id.as_deref()
                            == Some(target.window_id.as_str())
                    {
                        monitor.pending_launch_deliveries.remove(delivery_index);
                        complete_launch = true;
                    }
                }
                PendingLaunchDeliveryMatch::Mismatched => return Ok(false),
                PendingLaunchDeliveryMatch::Missing if !launch_already_durable => return Ok(false),
                PendingLaunchDeliveryMatch::Missing => {}
            }
        } else if !launch_already_durable {
            return Ok(false);
        }
        if complete_launch {
            monitor.complete_active_launch(observed_target.issue_number, target.window_id);
        }
        monitor.autonomous_handoffs[index].delivered_at = Some(now.to_string());
        monitor.autonomous_handoffs[index].delivery = AutonomousHandoffDeliveryState::Delivered {
            attempt: marker.attempt,
            prompt_sha256: marker.prompt_sha256.clone(),
            delivered_at: now.to_string(),
        };
        *prefs = monitor.prefs();
        Ok(true)
    })?;
    Ok(acknowledged)
}

/// Persist a definitely-not-submitted delivery failure and apply its bounded
/// exact-session backoff or terminal NeedsHuman transition atomically.
pub fn record_autonomous_handoff_delivery_failure_from_prefs(
    prefs_path: &Path,
    handoff_id: &str,
    session_id: &str,
    attempt: u32,
    message: &str,
    now: &str,
) -> io::Result<AutonomousHandoffDeliveryFailureOutcome> {
    let (_, outcome) = try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let outcome = monitor.record_autonomous_handoff_delivery_failure(
            handoff_id, session_id, attempt, message, now,
        );
        *prefs = monitor.prefs();
        Ok(outcome)
    })?;
    Ok(outcome)
}

/// Mark a worker-started or partially-written delivery as outcome-ambiguous.
/// This is the only valid failure path after the pre-submit scheduling fence.
pub fn mark_autonomous_handoff_delivery_ambiguous_from_prefs(
    prefs_path: &Path,
    handoff_id: &str,
    session_id: &str,
    attempt: u32,
    reason: &str,
    now: &str,
) -> io::Result<bool> {
    let (_, marked) = try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let marked = monitor.mark_autonomous_handoff_delivery_ambiguous(
            handoff_id, session_id, attempt, reason, now,
        );
        *prefs = monitor.prefs();
        Ok(marked)
    })?;
    Ok(marked)
}

/// Preserve exact-session routing for a durable answered handoff, including
/// legacy or in-flight deliveries that still carry a stale FreshRequired bit.
pub fn preserve_answered_handoff_resume_strategy_from_prefs(
    prefs_path: &Path,
    issue_number: u64,
) -> io::Result<bool> {
    let (_, answered_handoff_pending) = try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let answered_handoff_pending = prefs.autonomous_handoffs.iter().any(|handoff| {
            handoff.issue_number == issue_number
                && handoff.state == AutonomousHandoffState::Resumed
                && handoff.delivered_at.is_none()
        });
        if !answered_handoff_pending {
            return Ok(false);
        }
        prefs.queued_launch_session_strategies.insert(
            issue_number,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        for delivery in prefs
            .pending_launch_deliveries
            .iter_mut()
            .filter(|delivery| delivery.issue_number == issue_number)
        {
            delivery.launch_session_strategy = IssueMonitorLaunchSessionStrategy::ResumeIfSafe;
        }
        Ok(true)
    })?;
    Ok(answered_handoff_pending)
}

/// Commit the one-shot marker for the exact handoff and asking Session after
/// the runtime has proved that the answer reached its physical delivery
/// boundary. A mismatched or already-consumed handoff is a no-op.
pub fn mark_autonomous_handoff_delivered_from_prefs(
    prefs_path: &Path,
    handoff_id: &str,
    session_id: &str,
    now: &str,
) -> io::Result<bool> {
    let (_, marked) = try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let Some(handoff) = prefs.autonomous_handoffs.iter().find(|handoff| {
            handoff.handoff_id == handoff_id
                && handoff.session_id == session_id
                && handoff.state == AutonomousHandoffState::Resumed
        }) else {
            return Ok(false);
        };
        // Compatibility query only. A physical-boundary callback without the
        // protected UserPromptSubmit prompt cannot transition delivery state.
        let _ = now;
        Ok(matches!(
            handoff.delivery,
            AutonomousHandoffDeliveryState::Delivered { .. }
        ))
    })?;
    Ok(marked)
}

/// Issue #3478 (FR-025): append one handoff to the project's Issue Monitor
/// control plane under the stable cross-process prefs lock.
///
/// Called from the intercepting hook, which runs in the agent's process rather
/// than the daemon's. Appending is idempotent on `handoff_id` so a retried
/// hook invocation cannot park the same question twice. This writes only the
/// disk-owned inbound queue — every lifecycle transition stays with the
/// driver — so it deliberately does not require the effect authority fence.
pub fn record_autonomous_question_handoff(
    prefs_path: &Path,
    handoff: &AutonomousQuestionHandoff,
) -> io::Result<()> {
    mutate_issue_monitor_prefs_recovering(
        prefs_path,
        &IssueMonitorPrefs::recovery_default(),
        |prefs| {
            if !prefs
                .autonomous_handoffs
                .iter()
                .any(|known| known.handoff_id == handoff.handoff_id)
            {
                prefs.autonomous_handoffs.push(handoff.clone());
            }
        },
    )
    .map(|_| ())
}

impl IssueMonitorState {
    fn autonomous_handoff_delivery_target_is_live(
        &self,
        target: &crate::autonomous_handoff::AutonomousHandoffDeliveryTarget,
    ) -> bool {
        if !Self::autonomous_handoff_materializer_is_live(
            target.materializer_pid,
            target.materializer_started_at,
        ) {
            return false;
        }
        let launched = self.active_launches.contains(&target.issue_number)
            && self
                .launched_windows
                .get(&target.issue_number)
                .is_some_and(|window_id| {
                    issue_monitor_window_ids_match(window_id, &target.window_id)
                });
        if launched {
            return true;
        }
        let Some(delivery_id) = target.delivery_id.as_deref() else {
            return false;
        };
        self.pending_launch_deliveries.iter().any(|delivery| {
            delivery.issue_number == target.issue_number
                && delivery.delivery_id == delivery_id
                && delivery.materializer_id.as_deref() == Some(target.materializer_id.as_str())
                && delivery.materializer_window_id.as_deref() == Some(target.window_id.as_str())
                && delivery.materializer_pid == Some(target.materializer_pid)
        })
    }

    fn autonomous_handoff_materializer_is_live(pid: u32, started_at: u64) -> bool {
        pid > 0
            && started_at > 0
            && crate::process::host_process_start_time(pid) == Some(started_at)
    }

    pub fn new(config: IssueMonitorConfig) -> Self {
        Self {
            config,
            gui_connected: false,
            inbox: Vec::new(),
            legacy_git_launch_failure_migration_version:
                LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
            legacy_git_launch_failure_migration_tombstones: BTreeMap::new(),
            last_scan_at: None,
            last_error: None,
            launch_auth_required: false,
            active_launches: Vec::new(),
            priority_order: Vec::new(),
            launch_profile: None,
            provider_quota_holds: BTreeMap::new(),
            provider_quota_hold_evidence: BTreeMap::new(),
            provider_quota_hold_releases: BTreeMap::new(),
            launched_windows: BTreeMap::new(),
            launched_claims: BTreeMap::new(),
            launched_branches: BTreeMap::new(),
            merged_issues: BTreeSet::new(),
            issue_completion_migration_version: ISSUE_COMPLETION_MIGRATION_VERSION,
            completion_records: BTreeMap::new(),
            closure_records: BTreeMap::new(),
            closure_reopen_tombstones: BTreeSet::new(),
            autonomous_mode: false,
            effect_authority_epoch: 0,
            pending_effects: Vec::new(),
            last_control_receipt: None,
            autonomous_tuning: AutonomousTuning::default(),
            autonomous_records: BTreeMap::new(),
            failed_issues: BTreeMap::new(),
            failed_windows: BTreeMap::new(),
            released_failures: BTreeMap::new(),
            requeue_audit: Vec::new(),
            failure_release_version: 0,
            queue: VecDeque::new(),
            pending_launches: VecDeque::new(),
            pending_launch_deliveries: VecDeque::new(),
            queued_launch_session_strategies: BTreeMap::new(),
            pending_review_dispatches: VecDeque::new(),
            pending_autonomous_notices: VecDeque::new(),
            launching_claimed_at: BTreeMap::new(),
            autonomous_handoffs: Vec::new(),
        }
    }

    pub fn with_prefs(mut config: IssueMonitorConfig, prefs: IssueMonitorPrefs) -> Self {
        config.enabled = prefs.enabled;
        config.max_active = prefs.max_active_agents.max(1);
        let mut state = Self::new(config);
        state.legacy_git_launch_failure_migration_version =
            prefs.legacy_git_launch_failure_migration_version;
        state.priority_order = prefs.priority_order;
        state.last_scan_at = prefs.last_scan_at;
        state.launch_profile = prefs.launch_profile;
        state.provider_quota_holds = normalize_provider_quota_holds(&prefs.provider_quota_holds);
        state.provider_quota_hold_evidence =
            normalize_provider_keyed(&prefs.provider_quota_hold_evidence);
        state.provider_quota_hold_releases =
            normalize_provider_keyed(&prefs.provider_quota_hold_releases);
        state.enforce_provider_quota_hold_releases();
        state.queued_launch_session_strategies = prefs.queued_launch_session_strategies;
        state.launched_claims = prefs.launched_claims;
        // Issue #3627: restore used to re-inject every persisted launch into
        // `active_launches` without consulting `config.max_active`, so a disk
        // snapshot holding more launches than the cap (11 slots against a cap
        // of 5, in the reported wedge) reproduced that over-cap accounting on
        // every reload and every restart — the queue could never recover by
        // restarting.
        //
        // The excess is dropped whole, window binding included, so
        // `launched_windows` stays a subset of `active_launches`. Every
        // reconciler in this file walks one of the two and assumes the other
        // agrees; admitting a bound launch that holds no slot would make those
        // passes skip it silently.
        //
        // Claimed-but-unbound launches (Issue #3222) reserve their slots ahead
        // of the bound ones rather than being capped away. Their claim is
        // serialized back out of `active_launches`, so dropping one here would
        // erase an in-flight claim from disk and let the next rescan re-claim
        // the same Issue into a duplicate window — the exact regression #3222
        // fixed. They lapse on their own TTL through
        // [`Self::expire_stale_unbound_launches`] instead.
        let max_active = state.config.max_active.max(1);
        let bound_issue_numbers = prefs
            .launched_issues
            .iter()
            .filter(|launched| !launched.window_id.is_empty())
            .map(|launched| launched.issue_number)
            .collect::<BTreeSet<_>>();
        let unbound_reservation = prefs
            .launching_issues
            .iter()
            .map(|entry| entry.issue_number)
            .chain(
                prefs
                    .pending_launch_deliveries
                    .iter()
                    .map(|delivery| delivery.issue_number),
            )
            .filter(|issue_number| !bound_issue_numbers.contains(issue_number))
            .collect::<BTreeSet<_>>()
            .len();
        let bound_capacity = max_active.saturating_sub(unbound_reservation);
        let mut dropped_over_cap = Vec::new();
        for launched in prefs.launched_issues {
            if launched.window_id.is_empty() {
                continue;
            }
            if !state.active_launches.contains(&launched.issue_number)
                && state.active_launches.len() >= bound_capacity
            {
                dropped_over_cap.push(launched.issue_number);
                continue;
            }
            let issue_number = launched.issue_number;
            state
                .launched_windows
                .insert(issue_number, launched.window_id);
            if !state.active_launches.contains(&issue_number) {
                state.active_launches.push(issue_number);
            }
        }
        let retained_claim_issues = state
            .launched_windows
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        state
            .launched_claims
            .retain(|issue_number, _| retained_claim_issues.contains(issue_number));
        if !dropped_over_cap.is_empty() {
            tracing::warn!(
                dropped = ?dropped_over_cap,
                max_active,
                "issue monitor prefs held more launches than max_active; dropped the excess on restore"
            );
        }
        // Issue #3222: restore claimed-but-unbound launches so a reload (every
        // GUI handler) still sees the in-flight claim and cannot re-claim it.
        for entry in prefs.launching_issues {
            if !state.active_launches.contains(&entry.issue_number) {
                state.active_launches.push(entry.issue_number);
            }
            if let Some(claimed_at) = entry.claimed_at {
                state
                    .launching_claimed_at
                    .insert(entry.issue_number, claimed_at);
            }
        }
        for delivery in prefs.pending_launch_deliveries {
            if !state.active_launches.contains(&delivery.issue_number) {
                state.active_launches.push(delivery.issue_number);
            }
            state
                .launching_claimed_at
                .entry(delivery.issue_number)
                .or_insert_with(|| delivery.created_at.clone());
            state.pending_launch_deliveries.push_back(delivery);
        }
        for failed in prefs.failed_issues {
            if failed.message.trim().is_empty() {
                continue;
            }
            state
                .failed_issues
                .insert(failed.issue_number, failed.message);
            if let Some(window_id) = failed.window_id.filter(|id| !id.is_empty()) {
                state.failed_windows.insert(failed.issue_number, window_id);
            }
        }
        state.failure_release_version = prefs.failure_release_version;
        for release in prefs.released_failures {
            state
                .released_failures
                .insert(release.issue_number, release);
        }
        state.merge_requeue_audit(prefs.requeue_audit);
        state.issue_completion_migration_version = prefs.issue_completion_migration_version;
        for record in prefs.completion_records {
            state.merge_completion_record(record);
        }
        for issue_number in prefs.merged_issues {
            if !state.completion_records.contains_key(&issue_number) {
                state.merge_completion_record(IssueCompletionRecord::legacy(issue_number));
            }
        }
        state.refresh_merged_issue_projection();
        for record in prefs.closure_records {
            state.merge_closure_record(record);
        }
        state.autonomous_mode = prefs.autonomous_mode;
        state.effect_authority_epoch = prefs.effect_authority_epoch;
        state.pending_effects = prefs.pending_effects;
        state.last_control_receipt = prefs.last_control_receipt;
        state.autonomous_tuning = prefs.autonomous_tuning;
        for record in prefs.autonomous_records {
            state.autonomous_records.insert(record.issue_number, record);
        }
        state.autonomous_handoffs = prefs.autonomous_handoffs;
        let completed = state.merged_issues.iter().copied().collect::<Vec<_>>();
        for issue_number in completed {
            state.apply_merged_terminal_state(issue_number);
        }
        // Issue #3757: positive recovery authority outranks contradictory
        // failed/in-flight companions even on a cold restart. A valid fresh
        // claim consumes the release atomically before persistence.
        state.enforce_failure_release_fences();
        state
            .queued_launch_session_strategies
            .retain(|issue_number, _| {
                !state.failed_issues.contains_key(issue_number)
                    && !state.merged_issues.contains(issue_number)
            });
        state.apply_closed_issue_facts();
        state
    }

    pub fn prefs(&self) -> IssueMonitorPrefs {
        IssueMonitorPrefs {
            enabled: self.config.enabled,
            max_active_agents: self.config.max_active.max(1),
            priority_order: self.priority_order.clone(),
            legacy_git_launch_failure_migration_version: self
                .legacy_git_launch_failure_migration_version,
            launch_profile: self.launch_profile.clone(),
            provider_quota_holds: self.provider_quota_holds.clone(),
            provider_quota_hold_evidence: self.provider_quota_hold_evidence.clone(),
            provider_quota_hold_releases: self.provider_quota_hold_releases.clone(),
            launched_issues: self
                .launched_windows
                .iter()
                .map(|(issue_number, window_id)| IssueMonitorLaunchedIssue {
                    issue_number: *issue_number,
                    window_id: window_id.clone(),
                })
                .collect(),
            launched_claims: self.launched_claims.clone(),
            launching_issues: self
                .active_launches
                .iter()
                .filter(|issue_number| !self.launched_windows.contains_key(issue_number))
                .map(|issue_number| IssueMonitorLaunchingIssue {
                    issue_number: *issue_number,
                    claimed_at: self.launching_claimed_at.get(issue_number).cloned(),
                })
                .collect(),
            pending_launch_deliveries: self.pending_launch_deliveries.iter().cloned().collect(),
            queued_launch_session_strategies: self.queued_launch_session_strategies.clone(),
            failed_issues: self
                .failed_issues
                .iter()
                .map(|(issue_number, message)| IssueMonitorFailedIssue {
                    issue_number: *issue_number,
                    message: message.clone(),
                    window_id: self.failed_windows.get(issue_number).cloned(),
                })
                .collect(),
            released_failures: self.released_failures.values().cloned().collect(),
            requeue_audit: self.requeue_audit.clone(),
            failure_release_version: self.failure_release_version,
            merged_issues: self.merged_issues.iter().copied().collect(),
            issue_completion_migration_version: self.issue_completion_migration_version,
            completion_records: self.completion_records.values().cloned().collect(),
            closure_records: self.closure_records.values().cloned().collect(),
            autonomous_mode: self.autonomous_mode,
            effect_authority_epoch: self.effect_authority_epoch,
            pending_effects: self.pending_effects.clone(),
            last_control_receipt: self.last_control_receipt.clone(),
            autonomous_tuning: self.autonomous_tuning.clone(),
            autonomous_records: self.autonomous_records.values().cloned().collect(),
            autonomous_handoffs: self.autonomous_handoffs.clone(),
            last_scan_at: self.last_scan_at.clone(),
        }
    }

    fn merge_closure_record(&mut self, incoming: IssueClosureRecord) {
        let merged = self
            .closure_records
            .get(&incoming.issue_number)
            .map_or_else(
                || incoming.clone(),
                |current| Self::merge_closure_pair(current, &incoming),
            );
        self.closure_records.insert(incoming.issue_number, merged);
    }

    fn merge_closure_pair(
        current: &IssueClosureRecord,
        incoming: &IssueClosureRecord,
    ) -> IssueClosureRecord {
        let mut winner = if Self::closure_record_should_replace(current, incoming) {
            incoming.clone()
        } else {
            current.clone()
        };
        if current.state == incoming.state {
            winner.issue_updated_at = Self::newest_closure_revision_floor(
                current.issue_updated_at.as_deref(),
                incoming.issue_updated_at.as_deref(),
            );
        }
        winner.generation = current.generation.max(incoming.generation);
        winner
    }

    fn closure_record_should_replace(
        current: &IssueClosureRecord,
        incoming: &IssueClosureRecord,
    ) -> bool {
        if let Some(should_replace) = Self::closure_revision_should_replace(current, incoming) {
            return should_replace;
        }
        if incoming.generation != current.generation {
            return incoming.generation > current.generation;
        }
        if incoming.state != current.state {
            return incoming.state == IssueClosureState::Closed;
        }
        if incoming.evidence != current.evidence {
            return Self::closure_evidence_rank(incoming.evidence)
                > Self::closure_evidence_rank(current.evidence);
        }
        incoming.issue_updated_at > current.issue_updated_at
    }

    fn closure_evidence_rank(evidence: IssueClosureEvidence) -> u8 {
        match evidence {
            IssueClosureEvidence::DirectRelease => 0,
            IssueClosureEvidence::CompleteLiveAbsence => 1,
            IssueClosureEvidence::ExplicitRevision => 2,
        }
    }

    fn closure_revision_should_replace(
        current: &IssueClosureRecord,
        incoming: &IssueClosureRecord,
    ) -> Option<bool> {
        let both_explicit = current.evidence == IssueClosureEvidence::ExplicitRevision
            && incoming.evidence == IssueClosureEvidence::ExplicitRevision;
        let ordering = Self::closure_revision_floor_order(
            current.issue_updated_at.as_deref(),
            incoming.issue_updated_at.as_deref(),
        );
        if both_explicit {
            return ordering.and_then(|ordering| match ordering {
                std::cmp::Ordering::Greater => Some(true),
                std::cmp::Ordering::Less => Some(false),
                std::cmp::Ordering::Equal if current.state != incoming.state => {
                    Some(incoming.state == IssueClosureState::Closed)
                }
                std::cmp::Ordering::Equal => None,
            });
        }

        // A carried floor is only a lower bound for a non-explicit closure,
        // not its exact revision. It can reject an explicit Open at or below
        // that floor, or prove an incoming closure is at/above the current
        // explicit Open. Other cross-process orderings remain generation
        // fenced until a subsequent complete Live scan resolves them.
        if current.state == IssueClosureState::Closed
            && current.evidence != IssueClosureEvidence::ExplicitRevision
            && current
                .issue_updated_at
                .as_deref()
                .is_some_and(Self::closure_revision_floor_is_valid)
            && incoming.state == IssueClosureState::Reopened
            && incoming.evidence == IssueClosureEvidence::ExplicitRevision
        {
            return ordering.and_then(|ordering| match ordering {
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal => Some(false),
                std::cmp::Ordering::Greater => None,
            });
        }
        if current.state == IssueClosureState::Reopened
            && current.evidence == IssueClosureEvidence::ExplicitRevision
            && incoming.state == IssueClosureState::Closed
            && incoming.evidence != IssueClosureEvidence::ExplicitRevision
            && incoming
                .issue_updated_at
                .as_deref()
                .is_some_and(Self::closure_revision_floor_is_valid)
        {
            return ordering.and_then(|ordering| match ordering {
                std::cmp::Ordering::Greater | std::cmp::Ordering::Equal => Some(true),
                std::cmp::Ordering::Less => None,
            });
        }
        None
    }

    fn closure_revision_floor_is_valid(value: &str) -> bool {
        chrono::DateTime::parse_from_rfc3339(value).is_ok()
    }

    fn closure_revision_floor_order(
        current_floor: Option<&str>,
        incoming_floor: Option<&str>,
    ) -> Option<std::cmp::Ordering> {
        let current_time =
            current_floor.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
        let incoming_time =
            incoming_floor.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
        match (current_time, incoming_time) {
            (Some(current), Some(incoming)) => Some(incoming.cmp(&current)),
            (Some(_), None) => Some(std::cmp::Ordering::Less),
            (None, Some(_)) => Some(std::cmp::Ordering::Greater),
            (None, None) => None,
        }
    }

    fn newest_closure_revision_floor(
        current_floor: Option<&str>,
        incoming_floor: Option<&str>,
    ) -> Option<String> {
        match Self::closure_revision_floor_order(current_floor, incoming_floor) {
            Some(std::cmp::Ordering::Greater) => incoming_floor.map(str::to_string),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal) => {
                current_floor.map(str::to_string)
            }
            None => None,
        }
    }

    fn closure_records_from_prefs(prefs: &IssueMonitorPrefs) -> BTreeMap<u64, IssueClosureRecord> {
        let mut records = BTreeMap::new();
        for record in &prefs.closure_records {
            let merged = records.get(&record.issue_number).map_or_else(
                || record.clone(),
                |current: &IssueClosureRecord| Self::merge_closure_pair(current, record),
            );
            records.insert(record.issue_number, merged);
        }
        records
    }

    fn merge_closure_records_from_prefs(&mut self, disk: &IssueMonitorPrefs) {
        for record in Self::closure_records_from_prefs(disk).into_values() {
            self.merge_closure_record(record);
        }
    }

    fn local_reopened_closure_fences(&self, disk: &IssueMonitorPrefs) -> BTreeSet<u64> {
        let disk_records = Self::closure_records_from_prefs(disk);
        self.closure_records
            .values()
            .filter(|local| local.state == IssueClosureState::Reopened)
            .filter(|local| {
                disk_records.get(&local.issue_number).map_or_else(
                    || self.closure_reopen_tombstones.contains(&local.issue_number),
                    |disk| {
                        disk.state == IssueClosureState::Closed
                            && Self::closure_record_should_replace(disk, local)
                    },
                )
            })
            .map(|record| record.issue_number)
            .collect()
    }

    fn transition_issue_closure(
        &mut self,
        issue_number: u64,
        state: IssueClosureState,
        evidence: IssueClosureEvidence,
        issue_updated_at: Option<String>,
    ) {
        if let Some(current) = self
            .closure_records
            .get(&issue_number)
            .filter(|current| current.state == state)
            .cloned()
        {
            // An authoritative observation may advance while its lifecycle
            // state is unchanged. Preserve that revision fence so a delayed
            // opposite-state row cannot roll monitoring back.
            let advances = if evidence == IssueClosureEvidence::ExplicitRevision {
                match Self::closure_revision_floor_order(
                    current.issue_updated_at.as_deref(),
                    issue_updated_at.as_deref(),
                ) {
                    Some(std::cmp::Ordering::Greater) => true,
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal) => false,
                    None => current.evidence != IssueClosureEvidence::ExplicitRevision,
                }
            } else {
                true
            };
            if advances {
                let revision_floor = Self::newest_closure_revision_floor(
                    current.issue_updated_at.as_deref(),
                    issue_updated_at.as_deref(),
                );
                self.closure_records.insert(
                    issue_number,
                    IssueClosureRecord {
                        issue_number,
                        generation: current.generation.saturating_add(1),
                        state,
                        evidence,
                        issue_updated_at: revision_floor,
                    },
                );
            }
            if state == IssueClosureState::Closed {
                self.clear_closed_issue_current_state(issue_number);
            }
            return;
        }
        if self
            .closure_records
            .get(&issue_number)
            .is_some_and(|current| {
                !Self::closure_transition_is_fresh(
                    current,
                    state,
                    evidence,
                    issue_updated_at.as_deref(),
                )
            })
        {
            return;
        }
        let reopening_closed = state == IssueClosureState::Reopened
            && self
                .closure_records
                .get(&issue_number)
                .is_some_and(|current| current.state == IssueClosureState::Closed);
        let generation = self
            .closure_records
            .get(&issue_number)
            .map(|record| record.generation.saturating_add(1))
            .unwrap_or(1);
        let revision_floor = self.closure_records.get(&issue_number).map_or_else(
            || issue_updated_at.clone(),
            |record| {
                Self::newest_closure_revision_floor(
                    record.issue_updated_at.as_deref(),
                    issue_updated_at.as_deref(),
                )
            },
        );
        self.closure_records.insert(
            issue_number,
            IssueClosureRecord {
                issue_number,
                generation,
                state,
                evidence,
                issue_updated_at: revision_floor,
            },
        );
        if reopening_closed {
            self.closure_reopen_tombstones.insert(issue_number);
        } else if state == IssueClosureState::Closed {
            self.closure_reopen_tombstones.remove(&issue_number);
        }
        if state == IssueClosureState::Closed {
            self.clear_closed_issue_current_state(issue_number);
        }
    }

    fn closure_transition_is_fresh(
        current: &IssueClosureRecord,
        incoming_state: IssueClosureState,
        incoming_evidence: IssueClosureEvidence,
        incoming_updated_at: Option<&str>,
    ) -> bool {
        if incoming_evidence == IssueClosureEvidence::ExplicitRevision {
            if let Some(ordering) = Self::closure_revision_floor_order(
                current.issue_updated_at.as_deref(),
                incoming_updated_at,
            ) {
                return match ordering {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Less => false,
                    std::cmp::Ordering::Equal => incoming_state == IssueClosureState::Closed,
                };
            }
        }
        if current.state == IssueClosureState::Closed
            && current.issue_updated_at.is_none()
            && incoming_state == IssueClosureState::Reopened
            && incoming_evidence == IssueClosureEvidence::ExplicitRevision
        {
            // A process that has already adopted an absence/direct close may
            // trust its next positive complete-Live GitHub observation. An
            // unseen stale process still loses during generation merge.
            return true;
        }
        // Non-comparable evidence and ties resolve fail-closed.
        incoming_state == IssueClosureState::Closed
    }

    fn issue_is_closed(&self, issue_number: u64) -> bool {
        self.closure_records
            .get(&issue_number)
            .is_some_and(|record| record.state == IssueClosureState::Closed)
    }

    fn current_action_issue_numbers(&self) -> BTreeSet<u64> {
        self.active_launches
            .iter()
            .copied()
            .chain(self.queue.iter().copied())
            .chain(self.inbox.iter().map(|item| item.issue.number))
            .chain(self.failed_issues.keys().copied())
            .chain(self.autonomous_records.keys().copied())
            .chain(
                self.pending_launches
                    .iter()
                    .map(|pending| pending.issue_number),
            )
            .chain(
                self.pending_launch_deliveries
                    .iter()
                    .map(|pending| pending.issue_number),
            )
            .chain(
                self.pending_review_dispatches
                    .iter()
                    .map(|pending| pending.issue_number),
            )
            .chain(self.queued_launch_session_strategies.keys().copied())
            .chain(
                self.pending_effects
                    .iter()
                    .filter_map(|effect| match &effect.payload {
                        IssueMonitorEffectPayload::AcquireClaim { issue_number, .. }
                        | IssueMonitorEffectPayload::ArmAutoMerge { issue_number, .. } => {
                            Some(*issue_number)
                        }
                        IssueMonitorEffectPayload::ReleaseClaim { .. }
                        | IssueMonitorEffectPayload::DisarmAutoMerge { .. } => None,
                    }),
            )
            .chain(
                self.autonomous_handoffs
                    .iter()
                    .filter(|handoff| {
                        handoff.state != AutonomousHandoffState::Resumed
                            || handoff.delivered_at.is_none()
                    })
                    .map(|handoff| handoff.issue_number),
            )
            .chain(
                self.closure_records
                    .values()
                    .filter(|record| record.state == IssueClosureState::Reopened)
                    .map(|record| record.issue_number),
            )
            .collect()
    }

    fn clear_closed_issue_current_state(&mut self, issue_number: u64) {
        let removed_banner = self
            .failed_issues
            .get(&issue_number)
            .map(|message| format!("issue #{issue_number}: {message}"));
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.inbox.retain(|item| item.issue.number != issue_number);
        self.failed_issues.remove(&issue_number);
        self.failed_windows.remove(&issue_number);
        self.released_failures.remove(&issue_number);
        if let Some(pr_number) = self
            .autonomous_records
            .get(&issue_number)
            .filter(|record| record.phase == AutonomousPhase::Delivering)
            .and_then(|record| record.pr_number)
        {
            let source_effect_id = format!("closed-delivery:{issue_number}:{pr_number}");
            ensure_auto_merge_disarm_effect(
                &mut self.pending_effects,
                self.effect_authority_epoch,
                &source_effect_id,
                issue_number,
                pr_number,
            );
        }
        self.autonomous_records.remove(&issue_number);
        self.autonomous_handoffs.retain(|handoff| {
            handoff.issue_number != issue_number
                || (handoff.state == AutonomousHandoffState::Resumed
                    && handoff.delivered_at.is_some())
        });
        self.pending_autonomous_notices
            .retain(|notice| notice.issue_number != issue_number);
        revoke_uncommitted_effects_for_closed_issue(
            &mut self.pending_effects,
            self.effect_authority_epoch,
            issue_number,
        );
        if self.last_error.as_ref() == removed_banner.as_ref() {
            self.last_error = self.first_failed_issue_banner();
        }
    }

    fn apply_closed_issue_facts(&mut self) {
        let closed = self
            .closure_records
            .values()
            .filter(|record| record.state == IssueClosureState::Closed)
            .map(|record| record.issue_number)
            .collect::<Vec<_>>();
        for issue_number in closed {
            self.clear_closed_issue_current_state(issue_number);
        }
    }

    fn merge_completion_record(&mut self, incoming: IssueCompletionRecord) {
        let replace = self
            .completion_records
            .get(&incoming.issue_number)
            .is_none_or(|current| {
                incoming.generation > current.generation
                    || (incoming.generation == current.generation
                        && incoming.state == IssueCompletionState::Reopened
                        && current.state == IssueCompletionState::Completed)
            });
        if replace {
            self.completion_records
                .insert(incoming.issue_number, incoming);
        }
    }

    fn completion_records_from_prefs(
        prefs: &IssueMonitorPrefs,
    ) -> BTreeMap<u64, IssueCompletionRecord> {
        let mut records = BTreeMap::new();
        for record in &prefs.completion_records {
            let replace =
                records
                    .get(&record.issue_number)
                    .is_none_or(|current: &IssueCompletionRecord| {
                        record.generation > current.generation
                            || (record.generation == current.generation
                                && record.state == IssueCompletionState::Reopened
                                && current.state == IssueCompletionState::Completed)
                    });
            if replace {
                records.insert(record.issue_number, record.clone());
            }
        }
        for issue_number in &prefs.merged_issues {
            records
                .entry(*issue_number)
                .or_insert_with(|| IssueCompletionRecord::legacy(*issue_number));
        }
        records
    }

    fn refresh_merged_issue_projection(&mut self) {
        self.merged_issues = self
            .completion_records
            .values()
            .filter(|record| record.state == IssueCompletionState::Completed)
            .map(|record| record.issue_number)
            .collect();
    }

    fn merge_completion_records_from_prefs(&mut self, disk: &IssueMonitorPrefs) {
        let disk_records = Self::completion_records_from_prefs(disk);
        if disk.issue_completion_migration_version > self.issue_completion_migration_version {
            self.completion_records = disk_records;
            self.issue_completion_migration_version = disk.issue_completion_migration_version;
        } else {
            for record in disk_records.into_values() {
                self.merge_completion_record(record);
            }
        }
        self.refresh_merged_issue_projection();
        let completed = self.merged_issues.iter().copied().collect::<Vec<_>>();
        for issue_number in completed {
            self.apply_merged_terminal_state(issue_number);
        }
        let reopened = self
            .completion_records
            .values()
            .filter(|record| record.state == IssueCompletionState::Reopened)
            .map(|record| record.issue_number)
            .collect::<Vec<_>>();
        for issue_number in reopened {
            self.restore_reopened_candidate(issue_number);
        }
    }

    /// Reopened records created by the current transaction fence only disk
    /// companions that predate that record. Once the same Reopened generation
    /// is durable on disk, a disk launch may be a legitimate successor and is
    /// therefore absorbable again.
    fn local_reopened_completion_fences(&self, disk: &IssueMonitorPrefs) -> BTreeSet<u64> {
        let disk_records = Self::completion_records_from_prefs(disk);
        self.completion_records
            .values()
            .filter(|local| local.state == IssueCompletionState::Reopened)
            .filter(|local| {
                disk_records.get(&local.issue_number).is_none_or(|disk| {
                    local.generation > disk.generation
                        || (local.generation == disk.generation
                            && disk.state == IssueCompletionState::Completed)
                })
            })
            .map(|record| record.issue_number)
            .collect()
    }

    fn issue_completion_decision(issue: &IssueMonitorIssue) -> IssueCompletionDecision {
        let is_spec = issue
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("gwt-spec"));
        if !is_spec {
            return if issue.state == IssueMonitorIssueState::Closed {
                IssueCompletionDecision::Complete
            } else {
                IssueCompletionDecision::Incomplete
            };
        }
        if issue
            .updated_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .is_none()
        {
            return IssueCompletionDecision::Unknown;
        }
        match issue.readiness {
            IssueMonitorReadiness::ReadyWithCompletedTasks => IssueCompletionDecision::Complete,
            IssueMonitorReadiness::ReadyWithOpenTasks => IssueCompletionDecision::Incomplete,
            IssueMonitorReadiness::NotApplicable
            | IssueMonitorReadiness::Ready
            | IssueMonitorReadiness::NotReady => IssueCompletionDecision::Unknown,
        }
    }

    fn transition_completion(
        &mut self,
        issue_number: u64,
        state: IssueCompletionState,
        issue_updated_at: Option<String>,
        evidence: IssueCompletionEvidence,
    ) {
        if self
            .completion_records
            .get(&issue_number)
            .is_some_and(|current| {
                current.state == state
                    && current.issue_updated_at == issue_updated_at
                    && current.evidence == evidence
            })
        {
            return;
        }
        let generation = self
            .completion_records
            .get(&issue_number)
            .map(|record| record.generation.saturating_add(1))
            .unwrap_or(1);
        self.completion_records.insert(
            issue_number,
            IssueCompletionRecord {
                issue_number,
                generation,
                state,
                issue_updated_at,
                evidence,
            },
        );
        self.issue_completion_migration_version = ISSUE_COMPLETION_MIGRATION_VERSION;
        self.refresh_merged_issue_projection();
    }

    fn restore_reopened_candidate(&mut self, issue_number: u64) {
        self.merged_issues.remove(&issue_number);
        let Some(issue) = self.inbox_item(issue_number).map(|item| item.issue.clone()) else {
            return;
        };
        if issue.state == IssueMonitorIssueState::Open {
            self.record_candidate(issue);
        }
    }

    fn reopen_completion(
        &mut self,
        issue_number: u64,
        issue_updated_at: Option<String>,
        live_candidate_present: bool,
    ) {
        // The delivery is finished even though the Issue is not. Release all
        // launch capacity before publishing the successor generation so no
        // stale active companion can keep the slot occupied.
        self.clear_active_tracking(issue_number);
        self.clear_autonomous_record(issue_number);
        self.transition_completion(
            issue_number,
            IssueCompletionState::Reopened,
            issue_updated_at,
            IssueCompletionEvidence::Legacy,
        );
        if live_candidate_present {
            self.set_inbox_state(issue_number, MonitorInboxState::Queued);
            self.require_fresh_launch_session(issue_number);
        } else {
            self.queue.retain(|queued| *queued != issue_number);
            self.inbox.retain(|item| item.issue.number != issue_number);
        }
    }

    fn reconcile_live_completion_records(&mut self, issues: &[IssueMonitorIssue]) {
        let live = issues
            .iter()
            .map(|issue| (issue.number, issue))
            .collect::<BTreeMap<_, _>>();
        let records = self
            .completion_records
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for record in records {
            if record.state != IssueCompletionState::Completed {
                continue;
            }
            let current = live.get(&record.issue_number).copied();
            let invalid = if record.evidence == IssueCompletionEvidence::Legacy {
                true
            } else if let Some(issue) = current {
                match Self::issue_completion_decision(issue) {
                    IssueCompletionDecision::Incomplete => true,
                    IssueCompletionDecision::Unknown => false,
                    IssueCompletionDecision::Complete => false,
                }
            } else {
                false
            };
            if invalid {
                self.reopen_completion(
                    record.issue_number,
                    current.and_then(|issue| issue.updated_at.clone()),
                    current.is_some(),
                );
            }
        }
        self.issue_completion_migration_version = ISSUE_COMPLETION_MIGRATION_VERSION;
    }

    pub fn last_control_receipt(&self) -> Option<&IssueMonitorControlReceipt> {
        self.last_control_receipt.as_ref()
    }

    pub fn set_last_control_receipt(&mut self, receipt: IssueMonitorControlReceipt) {
        self.last_control_receipt = Some(receipt);
    }

    /// SPEC #3200 T-022: read-only access to an issue's autonomous record.
    pub fn autonomous_record(&self, issue_number: u64) -> Option<&AutonomousIssueRecord> {
        self.autonomous_records.get(&issue_number)
    }

    fn autonomous_record_mut(&mut self, issue_number: u64) -> &mut AutonomousIssueRecord {
        self.autonomous_records
            .entry(issue_number)
            .or_insert_with(|| AutonomousIssueRecord::new(issue_number))
    }

    /// SPEC #3200 T-016 / FR-026: failed/started attempts recorded for an issue.
    pub fn attempt_count(&self, issue_number: u64) -> u32 {
        self.autonomous_records
            .get(&issue_number)
            .map(|record| record.attempts)
            .unwrap_or(0)
    }

    /// SPEC #3200 T-016 / FR-026: increment the per-issue attempt counter,
    /// returning the new count. Drives max-attempts escalation to `NeedsHuman`.
    pub fn record_attempt(&mut self, issue_number: u64) -> u32 {
        let record = self.autonomous_record_mut(issue_number);
        record.attempts = record.attempts.saturating_add(1);
        record.attempts
    }

    /// SPEC #3200 T-022: set the lifecycle phase of an issue's current attempt.
    pub fn set_autonomous_phase(&mut self, issue_number: u64, phase: AutonomousPhase) {
        self.autonomous_record_mut(issue_number).phase = phase;
    }

    /// SPEC #3200 T-022 / FR-013: bind (or clear) the launch id of the in-flight
    /// attempt — the anchor for stuck detection and reviewed-SHA binding.
    pub fn set_active_launch_id(&mut self, issue_number: u64, launch_id: Option<String>) {
        self.autonomous_record_mut(issue_number).active_launch_id = launch_id;
    }

    /// SPEC #3200 T-018 / FR-014: capture the launch-time acceptance snapshot,
    /// compared against the re-classified Issue body at gate time.
    pub fn capture_acceptance_snapshot(
        &mut self,
        issue_number: u64,
        snapshot: crate::issue_monitor_gate::AcceptanceSnapshot,
    ) {
        self.autonomous_record_mut(issue_number).acceptance_snapshot = Some(snapshot);
    }

    /// SPEC #3200 T-016/T-022: drop an issue's autonomous record (resets the
    /// attempt counter) once the work merges or is otherwise resolved.
    pub fn clear_autonomous_record(&mut self, issue_number: u64) {
        self.autonomous_records.remove(&issue_number);
    }

    /// SPEC #3200 T-042/T-033 / FR-026 / Issue #3944 AC-2/AC-3: dispatch an
    /// autonomous attempt failure (launch failure, abnormal exit, lost launch,
    /// gate remediation). Counts the attempt and re-queues for a bounded-backoff
    /// retry: frees the slot and returns the issue to `Queued` for resume —
    /// never a fabricated "done" state and never `NeedsHuman`. At or past
    /// `max_attempts` the ladder keeps its capped backoff and the record carries
    /// a steering request, so the PM looks at why the attempts keep failing
    /// instead of the Issue silently parking.
    pub fn record_autonomous_failure(
        &mut self,
        issue_number: u64,
        message: impl Into<String>,
        now: &str,
    ) -> AutonomousFailureOutcome {
        let message = message.into();
        let attempt = self.record_attempt(issue_number);
        let max = self.autonomous_tuning.max_attempts;
        let backoff = autonomous_retry_backoff_secs(
            attempt,
            self.autonomous_tuning.retry_backoff_base_secs,
            self.autonomous_tuning.retry_backoff_cap_secs,
        );
        // FR-034: surface the transient retry (attempt + reason) so an
        // unattended failure loop is visible to the operator.
        self.push_autonomous_notice(
            "warn",
            issue_number,
            format!(
                "Issue #{issue_number} attempt {attempt}/{max} failed (retry scheduled): {message}"
            ),
        );
        self.clear_active_tracking(issue_number);
        self.require_fresh_launch_session(issue_number);
        self.set_autonomous_phase(issue_number, AutonomousPhase::Idle);
        self.set_active_launch_id(issue_number, None);
        let record = self.autonomous_record_mut(issue_number);
        record.retry_not_before = rfc3339_plus_secs(now, backoff);
        // Issue #3616: this backoff is the retry ladder, not a provider
        // hold. Leaving a stale quota reason attached would tell the PM to
        // wait out a reset that no longer gates anything.
        record.retry_hold_reason = None;
        record.retry_hold_provider = None;
        if attempt >= max {
            self.request_autonomous_steering(
                issue_number,
                format!(
                    "autonomous attempts exhausted ({attempt}/{max}): {message}; retrying after backoff"
                ),
                now,
            );
        }
        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
        AutonomousFailureOutcome::Retry { attempt }
    }

    /// Issue #3941 AC-3: return a launch that transient infrastructure aborted
    /// to the queue without spending an attempt. The reason stays on the inbox
    /// item so an operator can see why the Issue is waiting, and the base
    /// backoff keeps the retry off the scan that just saturated the host.
    /// Applies in every mode: the failure is about the host, not the Issue.
    pub fn record_transient_launch_retry(&mut self, issue_number: u64, message: String, now: &str) {
        let backoff = autonomous_retry_backoff_secs(
            1,
            self.autonomous_tuning.retry_backoff_base_secs,
            self.autonomous_tuning.retry_backoff_cap_secs,
        );
        self.push_autonomous_notice(
            "info",
            issue_number,
            format!(
                "Issue #{issue_number} launch hit transient infrastructure (retry in {backoff}s, no attempt consumed): {message}"
            ),
        );
        self.clear_active_tracking(issue_number);
        self.require_fresh_launch_session(issue_number);
        self.set_autonomous_phase(issue_number, AutonomousPhase::Idle);
        self.set_active_launch_id(issue_number, None);
        let record = self.autonomous_record_mut(issue_number);
        record.retry_not_before = rfc3339_plus_secs(now, backoff);
        record.retry_hold_reason = None;
        record.retry_hold_provider = None;
        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.error_message = Some(message);
        }
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
    }

    /// SPEC #3200 T-043/FR-029: whether `issue_number` may relaunch now. `true`
    /// when no retry backoff is pending or the backoff window has elapsed. An
    /// unparseable clock fails open so a glitch never permanently blocks a retry.
    pub fn retry_ready(&self, issue_number: u64, now: &str) -> bool {
        let Some(not_before) = self
            .autonomous_records
            .get(&issue_number)
            .and_then(|record| record.retry_not_before.as_deref())
        else {
            return true;
        };
        match (
            chrono::DateTime::parse_from_rfc3339(now),
            chrono::DateTime::parse_from_rfc3339(not_before),
        ) {
            (Ok(now_t), Ok(nb_t)) => now_t >= nb_t,
            _ => true,
        }
    }

    fn retry_ready_for_saved_profile(&self, issue_number: u64, now: &str) -> bool {
        let held_provider = self
            .autonomous_records
            .get(&issue_number)
            .and_then(|record| record.retry_hold_provider.as_deref())
            .and_then(normalize_issue_monitor_provider);
        // Issue #3923 AC-1: the provider-wide map is the admission source of
        // truth and the row's deadline only mirrors it. Once the provider's
        // hold is released (or has expired) the row is admissible, whichever
        // process recorded the mirror.
        if let Some(held) = held_provider.as_deref() {
            let provider_hold_active = self
                .provider_quota_holds
                .get(held)
                .and_then(|reset_at| parse_rfc3339_utc(reset_at))
                .zip(parse_rfc3339_utc(now))
                .is_some_and(|(reset_at, now)| reset_at > now);
            if !provider_hold_active {
                return true;
            }
        }
        let switched_to_healthy_provider = held_provider
            .zip(self.saved_launch_provider())
            .is_some_and(|(held, saved)| {
                let Some(now) = parse_rfc3339_utc(now) else {
                    return false;
                };
                held != saved
                    && self
                        .provider_quota_holds
                        .get(&held)
                        .and_then(|reset_at| parse_rfc3339_utc(reset_at))
                        .is_some_and(|reset_at| reset_at > now)
                    && self
                        .provider_quota_holds
                        .get(&saved)
                        .and_then(|reset_at| parse_rfc3339_utc(reset_at))
                        .is_none_or(|reset_at| reset_at <= now)
            });
        if switched_to_healthy_provider {
            // The operator explicitly selected a healthy provider. The
            // provider-wide SOT now authorizes admission even though this row
            // retains the old provider's audit/retry deadline.
            return true;
        }
        self.retry_ready(issue_number, now)
    }

    /// SPEC #3200 T-045/FR-013: record an observed liveness signal from the
    /// launched agent for `issue_number`. Resets the stuck-detection window.
    pub fn record_autonomous_heartbeat(&mut self, issue_number: u64, now: &str) {
        let record = self.autonomous_record_mut(issue_number);
        record.last_heartbeat = Some(now.to_string());
        // Issue #3944 AC-2: progress answers the steering request.
        record.steering = None;
    }

    /// Issue #3844 AC-1: record that the launched agent for `issue_number` is
    /// waiting (`reason`) until `resume_condition`. Refused for anything
    /// without a live launch. Declaring is itself a liveness signal. A
    /// re-declaration refreshes the text but keeps the original `since`, so
    /// the [`AUTONOMOUS_WAIT_MAX_SECS`] cap cannot be extended by renewing.
    pub fn declare_autonomous_wait(
        &mut self,
        issue_number: u64,
        reason: &str,
        resume_condition: &str,
        now: &str,
    ) -> AutonomousWaitOutcome {
        if !self.active_launches.contains(&issue_number) {
            return AutonomousWaitOutcome::NotLaunched;
        }
        self.record_autonomous_heartbeat(issue_number, now);
        let record = self.autonomous_record_mut(issue_number);
        let since = record
            .wait
            .as_ref()
            .map(|wait| wait.since.clone())
            .unwrap_or_else(|| now.to_string());
        record.wait = Some(AutonomousWaitDeclaration {
            reason: reason.trim().to_string(),
            resume_condition: resume_condition.trim().to_string(),
            since: since.clone(),
            declared_at: now.to_string(),
        });
        AutonomousWaitOutcome::Declared {
            expires_at: autonomous_wait_expires_at(&since),
            since,
        }
    }

    /// Issue #3844: the agent resumed; ordinary stuck detection applies again
    /// from this (liveness) instant.
    pub fn clear_autonomous_wait(&mut self, issue_number: u64, now: &str) -> AutonomousWaitOutcome {
        let Some(record) = self.autonomous_records.get_mut(&issue_number) else {
            return AutonomousWaitOutcome::NotWaiting;
        };
        if record.wait.take().is_none() {
            return AutonomousWaitOutcome::NotWaiting;
        }
        self.record_autonomous_heartbeat(issue_number, now);
        AutonomousWaitOutcome::Cleared
    }

    /// Issue #3844: the current wait declaration for `issue_number`, if any.
    pub fn autonomous_wait(&self, issue_number: u64) -> Option<&AutonomousWaitDeclaration> {
        self.autonomous_records
            .get(&issue_number)
            .and_then(|record| record.wait.as_ref())
    }

    /// SPEC #3200 T-044/T-035/FR-013: launched autonomous issues whose agent has
    /// shown no liveness for longer than `stuck_timeout_secs`. Pipeline-in-flight
    /// phases are excluded because they self-heal without a liveness signal:
    /// `Reviewing` is resumed on a daemon restart (see
    /// [`resume_inflight_reviews_after_restart`](Self::resume_inflight_reviews_after_restart)),
    /// and `Delivering` re-polls the persisted PR for its merge commit. Terminal
    /// phases are excluded too. Issues with no heartbeat yet are conservatively
    /// NOT judged stuck (no liveness data). Issue #3844: a record whose agent
    /// declared a wait (see [`Self::declare_autonomous_wait`]) is skipped
    /// while the declaration is within [`AUTONOMOUS_WAIT_MAX_SECS`].
    pub fn stuck_autonomous_issues(&self, now: &str) -> Vec<u64> {
        let timeout = self.autonomous_tuning.stuck_timeout_secs as i64;
        self.autonomous_records
            .values()
            .filter(|record| self.active_launches.contains(&record.issue_number))
            .filter(|record| {
                matches!(
                    record.phase,
                    AutonomousPhase::Idle | AutonomousPhase::Implementing
                )
            })
            // Issue #3844 AC-1/AC-3: a declared wait is not a stall while its
            // cap holds; past the cap the heartbeat rule below applies again.
            .filter(|record| !autonomous_wait_in_force(record, now))
            // Issue #3944 AC-2: a steering request re-arms the window, so a live
            // but silent agent is flagged once per timeout, not on every scan.
            .filter(|record| {
                autonomous_liveness_anchor_elapsed_secs(record, now)
                    .is_some_and(|elapsed| elapsed >= timeout)
            })
            .map(|record| record.issue_number)
            .collect()
    }

    /// SPEC #3200 T-044/T-045/FR-013 / Issue #3944 AC-2: handle every stuck
    /// autonomous launch. A launch whose window is still tracked (alive) is
    /// left in place and the PM is asked to steer it — tearing down a live
    /// agent loses its work, and a stall is not a decision the user can make.
    /// A launch with no window is lost: it is requeued as a transient failure
    /// (fresh launch). Neither path parks the Issue. Idempotent per stuck
    /// window: a steering request re-arms the timeout and a requeued issue is
    /// no longer launched.
    pub fn recover_stuck_autonomous(&mut self, now: &str) -> Vec<(u64, AutonomousFailureOutcome)> {
        // Fail-closed gate: never mutate autonomous state when the mode is off
        // (default), so the SPEC #3165 path is untouched.
        if !self.autonomous_mode {
            return Vec::new();
        }
        self.stuck_autonomous_issues(now)
            .into_iter()
            .map(|issue_number| {
                let outcome = if self.launched_windows.contains_key(&issue_number) {
                    let count = self.request_autonomous_steering(
                        issue_number,
                        "stuck/idle timeout: the agent window is alive but made no progress within stuck_timeout_secs; send it a one-line instruction",
                        now,
                    );
                    AutonomousFailureOutcome::SteeringRequested { count }
                } else {
                    self.record_autonomous_failure(
                        issue_number,
                        "stuck/idle timeout: no agent window is bound and no progress within stuck_timeout_secs",
                        now,
                    )
                };
                (issue_number, outcome)
            })
            .collect()
    }

    /// SPEC #3200 (review follow-up): restore self-healing for a `Reviewing`
    /// record whose review-agent dispatch was lost across a daemon restart.
    ///
    /// The review-agent spawn request (`pending_review_dispatches`) is NOT
    /// persisted, but the record's phase IS. A record reloaded in `Reviewing`
    /// therefore waits forever for a verdict that no agent will produce (the
    /// review agent was never re-spawned) — `advance_autonomous_in_flight`'s
    /// `Reviewing` branch only waits, and the `Implementing` branch (which
    /// re-detects the open PR and re-issues `begin_review` + the review dispatch)
    /// is never reached. Resetting the phase to `Implementing` restores exactly
    /// the pre-persist self-healing (a restart used to revert the in-memory
    /// phase): the next scan rebuilds the launch plan, re-detects the PR, and
    /// re-dispatches the review, binding to the current head SHA.
    ///
    /// `Delivering` is intentionally left untouched: its watch loop polls the
    /// persisted `pr_number` for the merge commit, so it self-heals across a
    /// restart on its own — and its GitHub auto-merge is already armed, so
    /// re-driving it would double-work and could invalidate the armed merge.
    ///
    /// The resumed record's `last_heartbeat` is refreshed to `now`: a restart is
    /// not a failed attempt, but the reset to `Implementing` makes the record
    /// eligible for stuck/idle detection (`stuck_autonomous_issues` covers
    /// `Idle | Implementing`), which runs BEFORE the re-dispatch on the next scan.
    /// Without the refresh, a persisted stale `last_heartbeat` (e.g. a review that
    /// ran longer than `stuck_timeout_secs` before the restart) would trip
    /// `recover_stuck_autonomous` and wrongly count a failed attempt / backoff
    /// (or escalate to `NeedsHuman` at the cap) before the review is even
    /// re-issued. The fresh stamp gives the resumed record a full window to reach
    /// `Reviewing` again.
    ///
    /// Idempotent and safe to call once right after loading persisted prefs; it
    /// only touches records parked in `Reviewing` while still awaiting a verdict.
    /// A durable pass/fail verdict must continue to the gate on the next scan.
    pub fn resume_inflight_reviews_after_restart(&mut self, now: &str) -> Vec<u64> {
        let reviewing = self
            .autonomous_records
            .values()
            .filter(|record| {
                record.phase == AutonomousPhase::Reviewing && record.review_passed.is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        self.resume_inflight_reviews_after_restart_for(&reviewing, now)
    }

    /// Resume only the records observed in `Reviewing` at daemon startup.
    ///
    /// Durable effect reconciliation may finish after normal operation has
    /// already begun. Binding recovery to the complete captured startup record
    /// prevents an unrelated runtime effect from rewinding a newly-dispatched
    /// live review for the same Issue.
    pub fn resume_inflight_reviews_after_restart_for(
        &mut self,
        startup_reviews: &[AutonomousIssueRecord],
        now: &str,
    ) -> Vec<u64> {
        let startup_reviews = startup_reviews
            .iter()
            .map(|record| (record.issue_number, record))
            .collect::<BTreeMap<_, _>>();
        let mut resumed = Vec::new();
        for record in self.autonomous_records.values_mut() {
            if record.phase == AutonomousPhase::Reviewing
                && record.review_passed.is_none()
                && startup_reviews
                    .get(&record.issue_number)
                    .is_some_and(|startup_record| &*record == *startup_record)
            {
                record.phase = AutonomousPhase::Implementing;
                record.review_passed = None;
                record.last_heartbeat = Some(now.to_string());
                resumed.push(record.issue_number);
            }
        }
        resumed
    }

    /// Issue #3944 AC-2: ask the PM to steer `issue_number` — a live launch the
    /// monitor will not tear down, an attempt ladder past its cap, or a gate
    /// held by the environment. Records the request on the autonomous record
    /// (visible in `issue.monitor.status`) and raises a warn notice. The same
    /// reason is not re-raised within one `stuck_timeout_secs`; past it the
    /// request is renewed with an incremented `count`. Returns the count.
    pub fn request_autonomous_steering(
        &mut self,
        issue_number: u64,
        reason: impl Into<String>,
        now: &str,
    ) -> u32 {
        let reason = reason.into();
        let timeout = self.autonomous_tuning.stuck_timeout_secs as i64;
        let record = self.autonomous_record_mut(issue_number);
        if let Some(existing) = record.steering.as_ref() {
            let renewed = rfc3339_elapsed_secs(&existing.requested_at, now)
                .is_none_or(|elapsed| elapsed >= timeout);
            if existing.reason == reason && !renewed {
                return existing.count;
            }
        }
        let count = record
            .steering
            .as_ref()
            .map_or(1, |existing| existing.count.saturating_add(1));
        record.steering = Some(AutonomousSteeringRequest {
            reason: reason.clone(),
            requested_at: now.to_string(),
            count,
        });
        self.push_autonomous_notice(
            "warn",
            issue_number,
            format!("Issue #{issue_number} steering requested ({count}): {reason}"),
        );
        count
    }

    /// SPEC #3200 FR-027 / Issue #3944 AC-1: park an issue in the terminal
    /// `NeedsHuman` state — frees the slot, records the reason and its
    /// human-answerable `kind`, marks the autonomous phase, and never
    /// auto-relaunches. The `kind` is mandatory so no caller can park an Issue
    /// for a mechanical cause (stuck, exhausted, launch, readiness, CI, review).
    pub fn escalate_to_needs_human(
        &mut self,
        issue_number: u64,
        kind: NeedsHumanKind,
        reason: impl Into<String>,
    ) {
        if self.issue_is_closed(issue_number) {
            return;
        }
        let reason = reason.into();
        // FR-034: an unattended escalation is exactly what the operator must see.
        self.push_autonomous_notice(
            "error",
            issue_number,
            format!(
                "Issue #{issue_number} needs human [{}]: {reason}",
                kind.as_str()
            ),
        );
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.set_autonomous_phase(issue_number, AutonomousPhase::NeedsHuman);
        self.set_active_launch_id(issue_number, None);
        let record = self.autonomous_record_mut(issue_number);
        record.needs_human_kind = Some(kind);
        record.steering = None;
        self.failed_issues.insert(issue_number, reason.clone());
        self.released_failures.remove(&issue_number);
        self.last_error = Some(format!("issue #{issue_number}: {reason}"));
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::NeedsHuman;
            item.launched_window_id = None;
            item.error_message = Some(reason);
        }
    }

    /// Issue #3478 (FR-025): all question handoffs known to this driver, in
    /// arrival order.
    pub fn autonomous_handoffs(&self) -> &[AutonomousQuestionHandoff] {
        &self.autonomous_handoffs
    }

    /// The handoff still blocking `issue_number`, if any. `Answered` and
    /// `Resumed` handoffs are history and no longer block.
    pub fn open_autonomous_handoff(&self, issue_number: u64) -> Option<&AutonomousQuestionHandoff> {
        self.autonomous_handoffs
            .iter()
            .find(|handoff| handoff.issue_number == issue_number && handoff.is_open())
    }

    /// Issue #3944 AC-1/AC-5: the park kind and one-line reason for the
    /// question that owns `issue_number`'s open handoff, with an optional
    /// `detail` about why it is (re)parked. A question with no handoff on
    /// record is a user choice by definition.
    fn handoff_needs_human(
        &self,
        issue_number: u64,
        detail: Option<&str>,
    ) -> (NeedsHumanKind, String) {
        match self.open_autonomous_handoff(issue_number) {
            Some(handoff) => {
                let reason = handoff.needs_human_reason();
                let reason = match detail {
                    Some(detail) => format!("{reason} [{detail}]"),
                    None => reason,
                };
                (handoff.reason_code.needs_human_kind(), reason)
            }
            None => (
                NeedsHumanKind::UserChoiceRequired,
                format!(
                    "{}: {}",
                    NeedsHumanKind::UserChoiceRequired.label(),
                    detail.unwrap_or("a question handoff is awaiting a human")
                ),
            ),
        }
    }

    /// Absorb handoffs observed outside this driver (hook writes, another
    /// process's answer) without losing driver-owned lifecycle transitions.
    ///
    /// Ownership per state is explicit: `Pending` is written by the
    /// intercepting hook and `Answered` by the canonical answer operation —
    /// both are inputs the driver must observe. `AwaitingHuman` and `Resumed`
    /// are driver-owned outputs, so a stale inbound copy of them never rewinds
    /// a transition this driver already made.
    pub fn absorb_autonomous_handoffs(
        &mut self,
        incoming: impl IntoIterator<Item = AutonomousQuestionHandoff>,
    ) {
        let mut adopted_delivery_transitions = Vec::new();
        for handoff in incoming {
            match self
                .autonomous_handoffs
                .iter_mut()
                .find(|known| known.handoff_id == handoff.handoff_id)
            {
                None => self.autonomous_handoffs.push(handoff),
                Some(known) => {
                    let incoming_answer_is_newer = handoff.answer_revision > known.answer_revision;
                    if incoming_answer_is_newer {
                        if matches!(
                            handoff.delivery,
                            AutonomousHandoffDeliveryState::RetryBackoff { .. }
                                | AutonomousHandoffDeliveryState::Ambiguous { .. }
                                | AutonomousHandoffDeliveryState::Exhausted { .. }
                        ) {
                            adopted_delivery_transitions
                                .push((handoff.issue_number, handoff.delivery.clone()));
                        }
                        *known = handoff;
                        continue;
                    }
                    if handoff.answer_revision < known.answer_revision {
                        // Older answer identities may be delayed writes. Never
                        // let them replace the current delivery fence.
                        continue;
                    }
                    if handoff.answer != known.answer || handoff.answered_at != known.answered_at {
                        // Equal revisions with different payloads indicate a
                        // legacy/concurrent conflict. This method is called
                        // with the freshly locked disk record as `handoff`, so
                        // disk wins rather than being overwritten by a stale
                        // resident driver on its next save.
                        *known = handoff;
                        continue;
                    }
                    if autonomous_handoff_delivery_progress(&handoff.delivery)
                        > autonomous_handoff_delivery_progress(&known.delivery)
                    {
                        known.delivery = handoff.delivery.clone();
                        known.delivered_at = handoff.delivered_at.clone();
                        known.state = handoff.state;
                        adopted_delivery_transitions.push((handoff.issue_number, handoff.delivery));
                    }
                }
            }
        }
        for (issue_number, delivery) in adopted_delivery_transitions {
            match delivery {
                AutonomousHandoffDeliveryState::RetryBackoff {
                    retry_not_before, ..
                } => {
                    self.clear_active_tracking(issue_number);
                    self.set_autonomous_phase(issue_number, AutonomousPhase::Idle);
                    self.set_active_launch_id(issue_number, None);
                    let record = self.autonomous_record_mut(issue_number);
                    record.retry_not_before = Some(retry_not_before);
                    record.retry_hold_reason = None;
                    record.retry_hold_provider = None;
                    self.set_inbox_state(issue_number, MonitorInboxState::Queued);
                    if !self.queue.contains(&issue_number) {
                        self.queue.push_back(issue_number);
                        self.apply_priority_order_to_queue();
                    }
                    self.queued_launch_session_strategies.insert(
                        issue_number,
                        IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
                    );
                }
                AutonomousHandoffDeliveryState::Ambiguous { reason, .. } => {
                    let (kind, reason) = self.handoff_needs_human(issue_number, Some(&reason));
                    self.escalate_to_needs_human(issue_number, kind, reason);
                }
                AutonomousHandoffDeliveryState::Exhausted { last_error, .. } => {
                    let detail =
                        format!("answered handoff delivery attempts are exhausted: {last_error}");
                    let (kind, reason) = self.handoff_needs_human(issue_number, Some(&detail));
                    self.escalate_to_needs_human(issue_number, kind, reason);
                }
                AutonomousHandoffDeliveryState::Pending
                | AutonomousHandoffDeliveryState::Attempting { .. }
                | AutonomousHandoffDeliveryState::Delivered { .. } => {}
            }
        }
    }

    /// SPEC #3200 FR-025 / Issue #3478 (AC-4, AC-7): park every Issue whose
    /// autonomous agent hit a human-judgment question and free its active slot
    /// in the same pass — the recognized question never waits for
    /// `stuck_timeout_secs`.
    ///
    /// Fail-closed like every other autonomous transition: a no-op while
    /// autonomous mode is off, so the SPEC #3165 human-gated flow is untouched.
    /// Idempotent — a handoff leaves `Pending` exactly once.
    pub fn apply_pending_autonomous_handoffs(&mut self, now: &str) -> Vec<u64> {
        if !self.autonomous_mode {
            return Vec::new();
        }
        let unresolved = self
            .autonomous_handoffs
            .iter()
            .filter_map(|handoff| match &handoff.delivery {
                AutonomousHandoffDeliveryState::Attempting {
                    attempt,
                    target,
                    materializer_pid,
                    materializer_started_at,
                    ..
                } if handoff.state == AutonomousHandoffState::Resumed
                    && handoff.delivered_at.is_none()
                    && !target.as_ref().map_or_else(
                        || {
                            Self::autonomous_handoff_materializer_is_live(
                                *materializer_pid,
                                *materializer_started_at,
                            )
                        },
                        |target| self.autonomous_handoff_delivery_target_is_live(target),
                    ) =>
                {
                    Some((
                        handoff.handoff_id.clone(),
                        handoff.session_id.clone(),
                        handoff.issue_number,
                        *attempt,
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut parked = Vec::new();
        for (handoff_id, session_id, issue_number, attempt) in unresolved {
            if self.mark_autonomous_handoff_delivery_ambiguous(
                &handoff_id,
                &session_id,
                attempt,
                "unresolved submit fence observed during monitor restart reconciliation",
                now,
            ) && !parked.contains(&issue_number)
            {
                parked.push(issue_number);
            }
        }
        let pending = self
            .autonomous_handoffs
            .iter_mut()
            .filter(|handoff| handoff.state == AutonomousHandoffState::Pending)
            .map(|handoff| {
                handoff.state = AutonomousHandoffState::AwaitingHuman;
                // Issue #3944 AC-1/AC-5: the park kind follows the question's
                // boundary and the reason line says what is being decided.
                (
                    handoff.issue_number,
                    handoff.reason_code.needs_human_kind(),
                    handoff.needs_human_reason(),
                )
            })
            .collect::<Vec<_>>();
        for (issue_number, kind, reason) in pending {
            self.escalate_to_needs_human(issue_number, kind, reason);
            if !parked.contains(&issue_number) {
                parked.push(issue_number);
            }
        }
        parked
    }

    /// Issue #3478 (AC-5): register a human answer for one open handoff.
    /// Returns `false` for an unknown or already-answered handoff so a caller
    /// never reports a delivered answer that went nowhere.
    pub fn answer_autonomous_handoff(&mut self, handoff_id: &str, answer: &str, now: &str) -> bool {
        let Some(handoff) = self
            .autonomous_handoffs
            .iter_mut()
            .find(|handoff| handoff.handoff_id == handoff_id && handoff.is_open())
        else {
            return false;
        };
        handoff.answer = Some(answer.to_string());
        handoff.answered_at = Some(now.to_string());
        handoff.answer_revision = handoff.answer_revision.saturating_add(1);
        // A new human answer is a new delivery identity. This is also the
        // explicit recovery path from Ambiguous/Exhausted: a delayed receipt
        // for the old answer can no longer match the canonical prompt hash.
        handoff.delivered_at = None;
        handoff.delivery = AutonomousHandoffDeliveryState::Pending;
        handoff.state = AutonomousHandoffState::Answered;
        true
    }

    /// Commit the write-ahead delivery fence for an answered handoff.
    ///
    /// This method returns a prompt only for a newly committed `Attempting`
    /// state. Re-observing an unresolved attempt converts it to `Ambiguous`,
    /// parks the Issue in `NeedsHuman`, and releases every active/pending
    /// launch anchor so restart can never replay the answer automatically.
    pub fn prepare_autonomous_handoff_delivery(
        &mut self,
        issue_number: u64,
        now: &str,
    ) -> Option<AutonomousHandoffDeliveryPreparation> {
        let index = self.autonomous_handoffs.iter().position(|handoff| {
            handoff.issue_number == issue_number
                && handoff.state == AutonomousHandoffState::Resumed
                && handoff.delivered_at.is_none()
        })?;
        let delivery = self.autonomous_handoffs[index].delivery.clone();
        let (attempt, ambiguity) = match delivery {
            AutonomousHandoffDeliveryState::Pending => (1, None),
            AutonomousHandoffDeliveryState::RetryBackoff {
                attempt,
                retry_not_before,
                ..
            } => {
                let retry_ready = match (
                    chrono::DateTime::parse_from_rfc3339(now),
                    chrono::DateTime::parse_from_rfc3339(&retry_not_before),
                ) {
                    (Ok(now), Ok(not_before)) => now >= not_before,
                    // A malformed clock must not strand an explicitly bounded
                    // retry forever; fail open to the next attempt.
                    _ => true,
                };
                if !retry_ready {
                    let handoff = &self.autonomous_handoffs[index];
                    return Some(AutonomousHandoffDeliveryPreparation::Backoff {
                        handoff_id: handoff.handoff_id.clone(),
                        session_id: handoff.session_id.clone(),
                        attempt,
                        retry_not_before,
                    });
                }
                (attempt.saturating_add(1), None)
            }
            AutonomousHandoffDeliveryState::Attempting {
                attempt,
                prompt_sha256,
                target,
                materializer_pid,
                materializer_started_at,
                ..
            } => {
                let target_session_id = target.as_ref().and_then(|target| {
                    self.autonomous_handoff_delivery_target_is_live(target)
                        .then(|| target.gwt_session_id.clone())
                });
                let materializer_live = target_session_id.is_some()
                    || (target.is_none()
                        && Self::autonomous_handoff_materializer_is_live(
                            materializer_pid,
                            materializer_started_at,
                        ));
                if materializer_live {
                    let handoff = &self.autonomous_handoffs[index];
                    return Some(AutonomousHandoffDeliveryPreparation::InFlight {
                        handoff_id: handoff.handoff_id.clone(),
                        session_id: handoff.session_id.clone(),
                        attempt,
                        target_session_id: target_session_id.unwrap_or_default(),
                    });
                }
                let reason = format!(
                    "answered handoff delivery attempt {attempt} has an ambiguous submit outcome after restart"
                );
                self.autonomous_handoffs[index].delivery =
                    AutonomousHandoffDeliveryState::Ambiguous {
                        attempt,
                        prompt_sha256,
                        detected_at: now.to_string(),
                        reason: reason.clone(),
                    };
                self.autonomous_handoffs[index].state = AutonomousHandoffState::AwaitingHuman;
                (attempt, Some(reason))
            }
            AutonomousHandoffDeliveryState::Ambiguous {
                attempt, reason, ..
            } => (attempt, Some(reason)),
            AutonomousHandoffDeliveryState::Exhausted {
                attempt,
                last_error,
                ..
            } => (
                attempt,
                Some(format!(
                    "answered handoff delivery attempts are exhausted: {last_error}"
                )),
            ),
            AutonomousHandoffDeliveryState::Delivered { .. } => return None,
        };
        if let Some(reason) = ambiguity {
            if self
                .autonomous_records
                .get(&issue_number)
                .is_none_or(|record| record.phase != AutonomousPhase::NeedsHuman)
            {
                let (kind, park_reason) = self.handoff_needs_human(issue_number, Some(&reason));
                self.escalate_to_needs_human(issue_number, kind, park_reason);
            } else {
                // Even if another process already projected NeedsHuman, an
                // unresolved local delivery may still retain launch anchors.
                self.clear_active_tracking(issue_number);
                self.queue.retain(|queued| *queued != issue_number);
            }
            let handoff = &self.autonomous_handoffs[index];
            return Some(AutonomousHandoffDeliveryPreparation::Ambiguous {
                handoff_id: handoff.handoff_id.clone(),
                session_id: handoff.session_id.clone(),
                attempt,
                reason,
            });
        }

        let handoff = &self.autonomous_handoffs[index];
        let body = autonomous_handoff_answer_prompt(handoff);
        let prompt = protected_autonomous_handoff_answer_prompt(
            &body,
            &handoff.handoff_id,
            &handoff.session_id,
            attempt,
        )?;
        let marker = parse_protected_autonomous_handoff_answer_prompt(&prompt)?;
        let prepared = AutonomousHandoffDeliveryAttempt {
            handoff_id: handoff.handoff_id.clone(),
            issue_number,
            session_id: handoff.session_id.clone(),
            attempt,
            prompt_sha256: marker.prompt_sha256.clone(),
            prompt,
        };
        let materializer_pid = std::process::id();
        let materializer_started_at =
            crate::process::host_process_start_time(materializer_pid).unwrap_or_default();
        self.autonomous_handoffs[index].delivery = AutonomousHandoffDeliveryState::Attempting {
            attempt,
            prompt_sha256: marker.prompt_sha256,
            started_at: now.to_string(),
            materializer_pid,
            materializer_started_at,
            target: None,
        };
        self.queued_launch_session_strategies.insert(
            issue_number,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        for delivery in self
            .pending_launch_deliveries
            .iter_mut()
            .filter(|delivery| delivery.issue_number == issue_number)
        {
            delivery.launch_session_strategy = IssueMonitorLaunchSessionStrategy::ResumeIfSafe;
        }
        Some(AutonomousHandoffDeliveryPreparation::Ready(prepared))
    }

    /// Record a failure proved to have occurred before any submit byte crossed
    /// the provider boundary. Ambiguous failures deliberately do not use this
    /// API: they remain `Attempting` until receipt or restart reconciliation.
    pub fn record_autonomous_handoff_delivery_failure(
        &mut self,
        handoff_id: &str,
        session_id: &str,
        attempt: u32,
        message: impl Into<String>,
        now: &str,
    ) -> AutonomousHandoffDeliveryFailureOutcome {
        let message = message.into();
        let Some(index) = self.autonomous_handoffs.iter().position(|handoff| {
            handoff.handoff_id == handoff_id
                && handoff.session_id == session_id
                && handoff.state == AutonomousHandoffState::Resumed
                && matches!(
                    &handoff.delivery,
                    AutonomousHandoffDeliveryState::Attempting {
                        attempt: current_attempt,
                        ..
                    } if *current_attempt == attempt
                )
        }) else {
            return AutonomousHandoffDeliveryFailureOutcome::Rejected;
        };
        let max_attempts = self.autonomous_tuning.max_attempts.max(1);
        if attempt >= max_attempts {
            let reason = format!(
                "answered handoff delivery attempts exhausted ({attempt}/{max_attempts}): {message}"
            );
            self.autonomous_handoffs[index].delivery = AutonomousHandoffDeliveryState::Exhausted {
                attempt,
                failed_at: now.to_string(),
                last_error: message,
            };
            self.autonomous_handoffs[index].state = AutonomousHandoffState::AwaitingHuman;
            let issue_number = self.autonomous_handoffs[index].issue_number;
            let (kind, park_reason) = self.handoff_needs_human(issue_number, Some(&reason));
            self.escalate_to_needs_human(issue_number, kind, park_reason);
            return AutonomousHandoffDeliveryFailureOutcome::Escalated { attempt, reason };
        }

        let backoff = autonomous_retry_backoff_secs(
            attempt,
            self.autonomous_tuning.retry_backoff_base_secs,
            self.autonomous_tuning.retry_backoff_cap_secs,
        );
        let retry_not_before = rfc3339_plus_secs(now, backoff).unwrap_or_else(|| now.to_string());
        let issue_number = self.autonomous_handoffs[index].issue_number;
        self.autonomous_handoffs[index].delivery = AutonomousHandoffDeliveryState::RetryBackoff {
            attempt,
            retry_not_before: retry_not_before.clone(),
            last_error: message.clone(),
        };
        self.push_autonomous_notice(
            "warn",
            issue_number,
            format!(
                "Issue #{issue_number} answered-handoff delivery {attempt}/{max_attempts} failed before submit (exact-session retry scheduled): {message}"
            ),
        );
        self.clear_active_tracking(issue_number);
        self.set_autonomous_phase(issue_number, AutonomousPhase::Idle);
        self.set_active_launch_id(issue_number, None);
        let record = self.autonomous_record_mut(issue_number);
        record.retry_not_before = Some(retry_not_before.clone());
        record.retry_hold_reason = None;
        record.retry_hold_provider = None;
        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
        // Unlike a generic launch failure, an answered handoff must never
        // switch to FreshRequired and strand its answer in another Session.
        self.queued_launch_session_strategies.insert(
            issue_number,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        AutonomousHandoffDeliveryFailureOutcome::Retry {
            attempt,
            retry_not_before,
        }
    }

    /// Resolve an exact write-ahead fence whose physical submit may have
    /// started. This transition is terminal until the human records a new
    /// answer: replay would risk delivering the same decision twice.
    pub fn mark_autonomous_handoff_delivery_ambiguous(
        &mut self,
        handoff_id: &str,
        session_id: &str,
        attempt: u32,
        reason: impl Into<String>,
        now: &str,
    ) -> bool {
        let reason = reason.into();
        let Some(index) = self.autonomous_handoffs.iter().position(|handoff| {
            handoff.handoff_id == handoff_id
                && handoff.session_id == session_id
                && handoff.state == AutonomousHandoffState::Resumed
                && matches!(
                    &handoff.delivery,
                    AutonomousHandoffDeliveryState::Attempting {
                        attempt: current_attempt,
                        ..
                    } if *current_attempt == attempt
                )
        }) else {
            return false;
        };
        let prompt_sha256 = match &self.autonomous_handoffs[index].delivery {
            AutonomousHandoffDeliveryState::Attempting { prompt_sha256, .. } => {
                prompt_sha256.clone()
            }
            _ => return false,
        };
        let issue_number = self.autonomous_handoffs[index].issue_number;
        self.autonomous_handoffs[index].delivery = AutonomousHandoffDeliveryState::Ambiguous {
            attempt,
            prompt_sha256,
            detected_at: now.to_string(),
            reason: reason.clone(),
        };
        self.autonomous_handoffs[index].state = AutonomousHandoffState::AwaitingHuman;
        let detail = format!(
            "answered handoff delivery attempt {attempt} has an ambiguous submit outcome: {reason}"
        );
        let (kind, park_reason) = self.handoff_needs_human(issue_number, Some(&detail));
        self.escalate_to_needs_human(issue_number, kind, park_reason);
        true
    }

    /// Issue #3478 (AC-5): un-park every answered handoff and return the
    /// resume instructions for its owning session.
    ///
    /// The parked attempt is resumed, not restarted: the autonomous record
    /// returns to `Implementing` with its attempt counter untouched, and the
    /// Issue is queued exactly once so the launch path re-engages the stored
    /// session instead of spawning a second agent for the same work.
    pub fn resume_answered_autonomous_handoffs(
        &mut self,
        now: &str,
    ) -> Vec<AutonomousHandoffResumption> {
        if !self.autonomous_mode {
            return Vec::new();
        }
        let answered = self
            .autonomous_handoffs
            .iter_mut()
            .filter(|handoff| handoff.state == AutonomousHandoffState::Answered)
            .map(|handoff| {
                handoff.state = AutonomousHandoffState::Resumed;
                AutonomousHandoffResumption {
                    handoff_id: handoff.handoff_id.clone(),
                    issue_number: handoff.issue_number,
                    session_id: handoff.session_id.clone(),
                    prompt: autonomous_handoff_answer_prompt(handoff),
                }
            })
            .collect::<Vec<_>>();
        for resumption in &answered {
            self.unpark_answered_autonomous_issue(resumption.issue_number, now);
        }
        answered
    }

    /// Issue #3478 (AC-5): take the answer prompt owed to `issue_number`'s
    /// resumed launch, marking it delivered so it is handed over exactly once.
    ///
    /// One-shot by construction: a replayed prompt would re-inject a stale
    /// human decision into a session that already acted on it.
    pub fn take_autonomous_resume_prompt(
        &mut self,
        issue_number: u64,
        now: &str,
    ) -> Option<String> {
        match self.prepare_autonomous_handoff_delivery(issue_number, now)? {
            AutonomousHandoffDeliveryPreparation::Ready(attempt) => Some(attempt.prompt),
            AutonomousHandoffDeliveryPreparation::Backoff { .. }
            | AutonomousHandoffDeliveryPreparation::InFlight { .. }
            | AutonomousHandoffDeliveryPreparation::Ambiguous { .. } => None,
        }
    }

    /// Reverse exactly what [`escalate_to_needs_human`](Self::escalate_to_needs_human)
    /// did for a question park, leaving the attempt counter and the acceptance
    /// snapshot alone.
    fn unpark_answered_autonomous_issue(&mut self, issue_number: u64, now: &str) {
        self.failed_issues.remove(&issue_number);
        self.failed_windows.remove(&issue_number);
        self.last_error = None;
        self.set_autonomous_phase(issue_number, AutonomousPhase::Implementing);
        let record = self.autonomous_record_mut(issue_number);
        record.last_heartbeat = Some(now.to_string());
        record.needs_human_kind = None;
        // A human answer belongs to the parked conversation. It overrides a
        // stale FreshRequired retry policy left by an earlier generic failure;
        // otherwise the accepted answer would be stranded while a fresh agent
        // starts without it.
        self.queued_launch_session_strategies.insert(
            issue_number,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        for delivery in self
            .pending_launch_deliveries
            .iter_mut()
            .filter(|delivery| delivery.issue_number == issue_number)
        {
            delivery.launch_session_strategy = IssueMonitorLaunchSessionStrategy::ResumeIfSafe;
        }
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Queued;
            item.error_message = None;
        }
        if !self.queue.contains(&issue_number) && !self.active_launches.contains(&issue_number) {
            self.queue.push_back(issue_number);
        }
        self.apply_priority_order_to_queue();
        self.push_autonomous_notice(
            "info",
            issue_number,
            format!("Issue #{issue_number} answered — resuming the parked session"),
        );
    }

    pub fn set_gui_connected(&mut self, connected: bool) {
        self.gui_connected = connected;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
        self.launch_auth_required = false;
        if !enabled {
            self.active_launches.clear();
            self.pending_launches.clear();
            self.queued_launch_session_strategies.clear();
            self.clear_launched_window_bindings();
        }
    }

    /// Issue #3627: turning the monitor off cleared `active_launches` but left
    /// `launched_windows` in place, and [`Self::prefs`] serializes
    /// `launched_issues` from exactly that map. The next reload therefore
    /// restored every slot the operator had just released, which is why
    /// toggling the monitor OFF and back ON — the one manual escape hatch from
    /// a wedged queue — never recovered anything.
    ///
    /// Disabling the monitor stops managing launches; it does not close the
    /// windows. Dropping the bindings is what makes OFF/ON a true reset.
    fn clear_launched_window_bindings(&mut self) {
        self.launched_windows.clear();
        self.launched_claims.clear();
        self.launched_branches.clear();
        self.launching_claimed_at.clear();
    }

    /// Advance remote-effect authority together with the global monitor switch.
    /// Turning the monitor off cancels effects that provably were not submitted
    /// and appends compensation for every outcome-ambiguous claim/arm attempt.
    /// A checked epoch overflow rejects the complete transition.
    pub fn set_enabled_with_effect_revocation(&mut self, enabled: bool) -> Option<u64> {
        if self.config.enabled == enabled {
            return Some(self.effect_authority_epoch);
        }
        let next_epoch =
            advance_effect_authority(&mut self.effect_authority_epoch, &mut self.pending_effects)?;

        self.config.enabled = enabled;
        self.launch_auth_required = false;
        if !enabled {
            for delivery in &self.pending_launch_deliveries {
                let already_pending = self.pending_effects.iter().any(|effect| {
                    matches!(
                        &effect.payload,
                        IssueMonitorEffectPayload::ReleaseClaim {
                            issue_number,
                            claim_id,
                            owner,
                        } if *issue_number == delivery.issue_number
                            && claim_id == &delivery.claim_id
                            && owner == &delivery.claim_owner
                    )
                });
                if !already_pending {
                    self.pending_effects
                        .push(PendingIssueMonitorEffect::prepared(
                            format!("release:{}:{next_epoch}", delivery.delivery_id),
                            next_epoch,
                            IssueMonitorEffectPayload::ReleaseClaim {
                                issue_number: delivery.issue_number,
                                claim_id: delivery.claim_id.clone(),
                                owner: delivery.claim_owner.clone(),
                            },
                        ));
                }
            }
            self.active_launches.clear();
            self.pending_launches.clear();
            self.pending_launch_deliveries.clear();
            self.queued_launch_session_strategies.clear();
            self.clear_launched_window_bindings();
        }
        Some(next_epoch)
    }

    /// Advance authority for a control that changes daemon decision state but
    /// does not itself require a compensation rewrite.
    pub fn advance_effect_authority_epoch(&mut self) -> Option<u64> {
        advance_effect_authority(&mut self.effect_authority_epoch, &mut self.pending_effects)
    }

    pub fn set_max_active_agents(&mut self, max_active_agents: usize) {
        self.config.max_active = max_active_agents.max(1);
    }

    /// Put back the durable scan timestamp after a projection-only rebuild.
    ///
    /// Issue #3633 (AC-5): a reader that reconstructs the queue from the local
    /// Issue cache has not scanned anything. Leaving the rebuild's timestamp in
    /// place would report a cadence that never happened.
    pub fn restore_persisted_last_scan_at(&mut self, last_scan_at: Option<String>) {
        self.last_scan_at = last_scan_at;
    }

    pub fn record_scan_error(&mut self, now: impl Into<String>, error: impl Into<String>) {
        self.last_scan_at = Some(now.into());
        self.last_error = Some(error.into());
        self.launch_auth_required = false;
    }

    /// Project an uncommitted control-plane failure without changing any
    /// durable preference or authority field. A later successful scan clears
    /// this transient operator-facing error through the normal scan path.
    pub fn record_control_commit_error(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
        self.launch_auth_required = false;
    }

    pub fn record_launch_auth_required(&mut self, now: impl Into<String>) {
        self.last_scan_at = Some(now.into());
        self.last_error = Some(github_auth_setup_message().to_string());
        self.launch_auth_required = true;
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn queued_issue_numbers(&self) -> Vec<u64> {
        self.queue.iter().copied().collect()
    }

    pub fn active_issue_number(&self) -> Option<u64> {
        self.active_launches.first().copied()
    }

    pub fn active_issue_numbers(&self) -> Vec<u64> {
        self.active_launches.clone()
    }

    pub fn active_count(&self) -> usize {
        self.active_launches.len()
    }

    pub fn has_launch_profile(&self) -> bool {
        self.launch_profile.is_some()
    }

    /// Refresh the user-configured fields from the latest committed snapshot.
    /// Explicit GUI/control mutations run after rebase, so they still win their
    /// transaction while stale scan writers cannot roll a newer config back.
    fn refresh_disk_owned_prefs(&mut self, disk: &IssueMonitorPrefs) {
        self.config.enabled = disk.enabled;
        self.config.max_active = disk.max_active_agents.max(1);
        self.priority_order = disk.priority_order.clone();
        self.apply_priority_order_to_queue();
        self.apply_priority_order_to_inbox();
        self.launch_profile = disk.launch_profile.clone();
        self.autonomous_mode = disk.autonomous_mode;
        self.effect_authority_epoch = disk.effect_authority_epoch;
        self.pending_effects = disk.pending_effects.clone();
        self.pending_launch_deliveries = disk.pending_launch_deliveries.iter().cloned().collect();
        self.queued_launch_session_strategies = disk.queued_launch_session_strategies.clone();
        self.last_control_receipt = disk.last_control_receipt.clone();
        self.autonomous_tuning = disk.autonomous_tuning.clone();
        // Issue #3478: the hook (and the answer operation) write handoffs from
        // outside this process, so disk is the inbound source for them.
        self.absorb_autonomous_handoffs(disk.autonomous_handoffs.iter().cloned());
    }

    fn merge_provider_quota_holds_from_prefs(&mut self, disk: &IssueMonitorPrefs) {
        for (provider, reset_at) in &disk.provider_quota_holds {
            merge_provider_quota_hold(&mut self.provider_quota_holds, provider, reset_at);
        }
        for (provider, evidence) in normalize_provider_keyed(&disk.provider_quota_hold_evidence) {
            let newer = self
                .provider_quota_hold_evidence
                .get(&provider)
                .and_then(|local| parse_rfc3339_utc(&local.recorded_at))
                .zip(parse_rfc3339_utc(&evidence.recorded_at))
                .is_none_or(|(local, disk)| disk > local);
            if newer {
                self.provider_quota_hold_evidence.insert(provider, evidence);
            }
        }
        for (provider, release) in normalize_provider_keyed(&disk.provider_quota_hold_releases) {
            let newer = self
                .provider_quota_hold_releases
                .get(&provider)
                .and_then(|local| parse_rfc3339_utc(&local.released_at))
                .zip(parse_rfc3339_utc(&release.released_at))
                .is_none_or(|(local, disk)| disk > local);
            if newer {
                self.provider_quota_hold_releases.insert(provider, release);
            }
        }
        // Issue #3923 AC-1: applied after the joins above, which otherwise
        // restore the very hold the operator released.
        self.enforce_provider_quota_hold_releases();
    }

    /// Issue #3923 AC-1: drop every hold that a recorded release fences.
    fn enforce_provider_quota_hold_releases(&mut self) {
        let released = self
            .provider_quota_holds
            .keys()
            .filter(|provider| {
                provider_quota_hold_is_released(
                    self.provider_quota_hold_evidence.get(*provider),
                    self.provider_quota_hold_releases.get(*provider),
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        for provider in released {
            self.provider_quota_holds.remove(&provider);
            self.provider_quota_hold_evidence.remove(&provider);
        }
    }

    /// Issue #3923 AC-5: switch the saved launch profile to `agent` from the
    /// CLI, so a PM can move the fleet off a held provider without the GUI.
    ///
    /// Only the agent changes. Model, reasoning, and pinned version are
    /// provider-specific and are cleared so the new agent's defaults apply;
    /// session mode, permissions, runtime target, and Docker choices are the
    /// wizard's and are kept. Idempotent for the current agent.
    pub fn switch_launch_profile_agent(
        &mut self,
        agent: &str,
    ) -> Result<IssueMonitorLaunchProfile, IssueMonitorLaunchProfileSwitchError> {
        let Some(agent) = normalize_issue_monitor_provider(agent) else {
            return Err(IssueMonitorLaunchProfileSwitchError::InvalidAgent);
        };
        let Some(profile) = self.launch_profile.as_mut() else {
            return Err(IssueMonitorLaunchProfileSwitchError::NoSavedProfile);
        };
        if normalize_issue_monitor_provider(&profile.agent_id).as_deref() != Some(agent.as_str()) {
            profile.agent_id = agent;
            profile.model = None;
            profile.reasoning = None;
            profile.version = None;
        }
        Ok(profile.clone())
    }

    /// Issue #3923 AC-1: release `provider`'s quota hold on the operator's
    /// authority.
    ///
    /// The release is recorded even when nothing is held in this snapshot, so
    /// a hold another process formed but has not committed yet is fenced too.
    /// Every issue the hold was holding is readmitted: the provider-wide map is
    /// the admission source of truth, and the per-issue retry deadline only
    /// mirrored it.
    pub fn clear_provider_quota_hold(
        &mut self,
        provider: &str,
        reason: &str,
        now: &str,
    ) -> IssueMonitorProviderQuotaHoldClearOutcome {
        let Some(provider) = normalize_issue_monitor_provider(provider) else {
            return IssueMonitorProviderQuotaHoldClearOutcome::UnknownProvider;
        };
        let released_reset_at = self.provider_quota_holds.remove(&provider);
        let evidence = self.provider_quota_hold_evidence.remove(&provider);
        self.provider_quota_hold_releases.insert(
            provider.clone(),
            IssueMonitorProviderQuotaHoldRelease {
                released_at: now.to_string(),
                reason: reason.to_string(),
                released_reset_at: released_reset_at.clone(),
            },
        );
        let mut released_issues = Vec::new();
        for (issue_number, record) in &mut self.autonomous_records {
            let held_by_provider = record
                .retry_hold_provider
                .as_deref()
                .and_then(normalize_issue_monitor_provider)
                .is_some_and(|held| held == provider);
            if held_by_provider {
                record.retry_not_before = None;
                record.retry_hold_reason = None;
                record.retry_hold_provider = None;
                released_issues.push(*issue_number);
            }
        }
        tracing::warn!(
            provider = %provider,
            released_reset_at = ?released_reset_at,
            released_issues = ?released_issues,
            evidence = ?evidence,
            reason = %reason,
            "Issue Monitor provider quota hold released by operator (Issue #3923)"
        );
        IssueMonitorProviderQuotaHoldClearOutcome::Cleared {
            released_reset_at,
            released_issues,
        }
    }

    /// Rebase a GUI observer on the latest committed prefs. The GUI does not
    /// drive autonomous lifecycle transitions, so disk owns the complete
    /// autonomous record map, including an updated value for an existing key.
    pub fn rebase_gui_observer_prefs(&mut self, disk: &IssueMonitorPrefs) {
        self.rebase_cross_process_prefs(disk, AutonomousRecordRebasePolicy::DiskAuthoritative);
    }

    /// Rebase the daemon driver on the latest committed prefs. A scan result
    /// already held by the daemon owns an existing autonomous-record key, while
    /// records written for other Issues are absorbed from disk.
    pub fn rebase_daemon_driver_prefs(&mut self, disk: &IssueMonitorPrefs) {
        self.rebase_cross_process_prefs(
            disk,
            AutonomousRecordRebasePolicy::LocalSameKeyAuthoritative,
        );
    }

    /// Rebase before applying an exact window-close control under the prefs
    /// lock. A normal daemon scan keeps its same-Issue in-memory launch, but a
    /// destructive CAS must compare against the generation that is currently
    /// durable. Replace the window and claim together so a stale predecessor
    /// cannot make its close match a same-window successor.
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) fn rebase_daemon_driver_prefs_for_exact_window_close(
        &mut self,
        disk: &IssueMonitorPrefs,
        issue_number: u64,
    ) {
        self.rebase_daemon_driver_prefs(disk);
        let Some(disk_window_id) = disk
            .launched_issues
            .iter()
            .rev()
            .find(|launched| {
                launched.issue_number == issue_number && !launched.window_id.is_empty()
            })
            .map(|launched| &launched.window_id)
        else {
            return;
        };
        let disk_claim_id = disk.launched_claims.get(&issue_number);
        if self.launched_windows.get(&issue_number) == Some(disk_window_id)
            && self.launched_claims.get(&issue_number) == disk_claim_id
        {
            return;
        }

        self.launched_windows
            .insert(issue_number, disk_window_id.clone());
        match disk_claim_id {
            Some(claim_id) => {
                self.launched_claims.insert(issue_number, claim_id.clone());
            }
            None => {
                self.launched_claims.remove(&issue_number);
            }
        }
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
    }

    /// Rebase a stale process-local state on the latest committed prefs before
    /// applying this transaction's explicit lifecycle mutation. Real launches,
    /// merged completions, and failures are reconciled before the caller's
    /// mutation, which therefore remains authoritative for its Issue.
    fn rebase_cross_process_prefs(
        &mut self,
        disk: &IssueMonitorPrefs,
        autonomous_policy: AutonomousRecordRebasePolicy,
    ) {
        let reopened_closure_fences = self.local_reopened_closure_fences(disk);
        let reopened_candidates = reopened_closure_fences
            .iter()
            .filter_map(|issue_number| {
                self.inbox_item(*issue_number)
                    .map(|item| (*issue_number, item.issue.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        self.merge_closure_records_from_prefs(disk);
        let reopened_completion_fences = self.local_reopened_completion_fences(disk);
        // Issue #3757: resolve the exact recovery/fresh-claim authority before
        // union-merging in-flight companions. Otherwise a stale local launch
        // can mask the fresh disk launch or survive a disk recovery fence.
        self.consume_releases_superseded_by_disk_launches(disk);
        self.adopt_failure_releases_from_prefs(disk);
        self.enforce_failure_release_fences();
        self.merge_completion_records_from_prefs(disk);
        let reopened_fences = reopened_completion_fences
            .union(&reopened_closure_fences)
            .copied()
            .collect::<BTreeSet<_>>();
        self.merge_inflight_launches_from_disk_excluding(disk, &reopened_fences);
        let rejected_terminal_companions = self.rejected_disk_terminal_failure_companions(disk);
        match autonomous_policy {
            AutonomousRecordRebasePolicy::DiskAuthoritative => {
                let protected_local_records = rejected_terminal_companions
                    .iter()
                    .filter_map(|issue_number| {
                        if self.merged_issues.contains(issue_number) {
                            return None;
                        }
                        self.autonomous_records
                            .get(issue_number)
                            .cloned()
                            .map(|record| (*issue_number, record))
                    })
                    .collect::<Vec<_>>();
                self.autonomous_records = disk
                    .autonomous_records
                    .iter()
                    .filter(|record| {
                        !self.merged_issues.contains(&record.issue_number)
                            && !reopened_completion_fences.contains(&record.issue_number)
                            && !reopened_closure_fences.contains(&record.issue_number)
                            && (record.phase != AutonomousPhase::NeedsHuman
                                || !rejected_terminal_companions.contains(&record.issue_number))
                    })
                    .map(|record| (record.issue_number, record.clone()))
                    .collect();
                self.autonomous_records.extend(protected_local_records);
            }
            AutonomousRecordRebasePolicy::LocalSameKeyAuthoritative => {
                for record in &disk.autonomous_records {
                    if self.merged_issues.contains(&record.issue_number)
                        || reopened_completion_fences.contains(&record.issue_number)
                        || reopened_closure_fences.contains(&record.issue_number)
                        || (record.phase == AutonomousPhase::NeedsHuman
                            && rejected_terminal_companions.contains(&record.issue_number))
                    {
                        continue;
                    }
                    self.autonomous_records
                        .entry(record.issue_number)
                        .or_insert_with(|| record.clone());
                }
            }
        }
        let adopted = self.adopt_newer_legacy_git_launch_failure_migration_from_prefs(disk);
        if !adopted
            && disk.legacy_git_launch_failure_migration_version
                == self.legacy_git_launch_failure_migration_version
        {
            self.merge_equal_marker_disk_failures(disk);
        } else if !adopted
            && disk.legacy_git_launch_failure_migration_version
                < self.legacy_git_launch_failure_migration_version
        {
            self.merge_older_marker_disk_failures(disk);
        }
        // Issue #3645: applied after the failure merges above, which otherwise
        // restore the very hold the operator released — leaving this process
        // convinced the issue is still broken and writing that view back over
        // the recovery on its next commit.
        self.merge_requeue_audit(disk.requeue_audit.iter().cloned());
        self.enforce_failure_release_fences();
        self.drop_releases_for_failed_issues();
        // A provider hold is positive monotonic admission evidence. Absence or
        // an older reset in a stale disk snapshot may never erase newer local
        // evidence, so join it before refreshing disk-owned replacement fields.
        self.merge_provider_quota_holds_from_prefs(disk);
        self.refresh_disk_owned_prefs(disk);
        // The refresh replaces durable delivery/effect projections. Reapply
        // the fence so a contradictory disk snapshot cannot restore an
        // abandoned delivery after the first cleanup.
        self.enforce_failure_release_fences();
        let terminal_issue_numbers = self
            .merged_issues
            .iter()
            .copied()
            .chain(self.failed_issues.keys().copied())
            .chain(
                self.inbox
                    .iter()
                    .filter(|item| item.state.is_terminal())
                    .map(|item| item.issue.number),
            )
            .collect::<BTreeSet<_>>();
        let stale_disk_companions = terminal_issue_numbers
            .union(&reopened_completion_fences)
            .copied()
            .collect::<BTreeSet<_>>();
        self.pending_launch_deliveries
            .retain(|delivery| !stale_disk_companions.contains(&delivery.issue_number));
        self.queued_launch_session_strategies
            .retain(|issue_number, _| !stale_disk_companions.contains(issue_number));

        // SPEC-3431 FR-033: a stop another process committed is terminal for
        // this one too. Applied last, after the in-flight launch merge above,
        // which otherwise restores the very launch the stop revoked — leaving
        // the daemon convinced the agent is still running, holding its slot,
        // and writing that view back over the stop on its next commit.
        let stopped = disk
            .failed_issues
            .iter()
            .filter_map(|failed| {
                failed
                    .message
                    .strip_prefix(STOP_ONLY_REASON_PREFIX)
                    .map(|reason| (failed.issue_number, reason.to_string()))
            })
            .collect::<Vec<_>>();
        for (issue_number, reason) in stopped {
            self.adopt_stopped_terminal_state(issue_number, &reason);
        }
        // Issue #3602: closure is applied after every additive/authoritative
        // companion merge above, otherwise a stale failure or launch can be
        // reintroduced later in the same transaction. A newer local reopen
        // similarly fences disk companions from the older closed generation,
        // then restores the live candidate captured before the merge.
        self.apply_closed_issue_facts();
        for issue_number in reopened_closure_fences {
            if self.issue_is_closed(issue_number) {
                continue;
            }
            self.clear_closed_issue_current_state(issue_number);
            if let Some(issue) = reopened_candidates.get(&issue_number) {
                self.record_candidate(issue.clone());
            }
        }
    }

    /// SPEC-3431 FR-033: converge on a stop committed elsewhere.
    ///
    /// Mirrors what [`Self::stop_only`] did in the committing process, minus
    /// the authority epoch bump (that already happened once, and repeating it
    /// on every rebase would revoke effects this stop has nothing to do with)
    /// and minus the operator notice (the stop was already reported).
    fn adopt_stopped_terminal_state(&mut self, issue_number: u64, reason: &str) {
        // A local merge outranks a stop: the work finished before it landed.
        if self.merged_issues.contains(&issue_number) {
            return;
        }
        let message = format!("{STOP_ONLY_REASON_PREFIX}{reason}");
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.set_autonomous_phase(issue_number, AutonomousPhase::NeedsHuman);
        self.set_active_launch_id(issue_number, None);
        let record = self.autonomous_record_mut(issue_number);
        record.needs_human_kind = Some(NeedsHumanKind::UserChoiceRequired);
        record.steering = None;
        self.failed_issues.insert(issue_number, message.clone());
        self.released_failures.remove(&issue_number);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::NeedsHuman;
            item.launched_window_id = None;
            item.error_message = Some(message);
        }
    }

    /// A terminal record paired with a rejected disk failure is part of the
    /// same stale terminal transition and must not bypass lifecycle precedence
    /// through the general autonomous-record merge.
    fn rejected_disk_terminal_failure_companions(&self, disk: &IssueMonitorPrefs) -> BTreeSet<u64> {
        disk.failed_issues
            .iter()
            .filter(|failed| {
                disk.autonomous_records.iter().any(|record| {
                    record.issue_number == failed.issue_number
                        && record.phase == AutonomousPhase::NeedsHuman
                })
            })
            .filter(|failed| {
                failed.message.trim().is_empty()
                    || self.launched_windows.contains_key(&failed.issue_number)
                    || self.merged_issues.contains(&failed.issue_number)
                    || (disk.legacy_git_launch_failure_migration_version
                        < self.legacy_git_launch_failure_migration_version
                        && self.is_migrated_failure_tombstone(failed))
                    || (disk.legacy_git_launch_failure_migration_version
                        <= self.legacy_git_launch_failure_migration_version
                        && self.failed_issues.contains_key(&failed.issue_number))
            })
            .map(|failed| failed.issue_number)
            .collect()
    }

    fn is_migrated_failure_tombstone(&self, failed: &IssueMonitorFailedIssue) -> bool {
        failed.window_id.is_none()
            && self
                .legacy_git_launch_failure_migration_tombstones
                .get(&failed.issue_number)
                .is_some_and(|message| message == &failed.message)
    }

    /// Adopt a strictly newer cross-process migration result. Failure rows
    /// removed by the authoritative disk snapshot are removed before the next
    /// scan can reconcile candidates; retained unrelated failures stay intact.
    /// This direct adoption path is a no-op for equal/older markers so a newly
    /// recorded same-text failure can never be erased by stale disk state. The
    /// outer rebase separately unions safe disk-only failures.
    pub fn adopt_newer_legacy_git_launch_failure_migration_from_prefs(
        &mut self,
        disk: &IssueMonitorPrefs,
    ) -> bool {
        if disk.legacy_git_launch_failure_migration_version
            <= self.legacy_git_launch_failure_migration_version
        {
            return false;
        }

        let old_failures = self.failed_issues.clone();
        let old_banners = old_failures
            .iter()
            .map(|(issue_number, message)| format!("issue #{issue_number}: {message}"))
            .collect::<BTreeSet<_>>();
        let mut disk_failures = BTreeMap::new();
        let mut disk_windows = BTreeMap::new();
        let mut disk_needs_human = BTreeMap::new();
        for failed in &disk.failed_issues {
            // A window currently owned by this process is stronger evidence
            // than a newer marker paired with another process's stale failure
            // snapshot. Keep the live launch internally and on the next prefs
            // roundtrip instead of creating a launched+failed split-brain row.
            if self.launched_windows.contains_key(&failed.issue_number)
                || self.merged_issues.contains(&failed.issue_number)
            {
                continue;
            }
            if failed.message.trim().is_empty() {
                continue;
            }
            disk_failures.insert(failed.issue_number, failed.message.clone());
            if let Some(window_id) = failed.window_id.as_ref().filter(|id| !id.is_empty()) {
                disk_windows.insert(failed.issue_number, window_id.clone());
            }
            if let Some(record) = disk.autonomous_records.iter().find(|record| {
                record.issue_number == failed.issue_number
                    && record.phase == AutonomousPhase::NeedsHuman
            }) {
                disk_needs_human.insert(failed.issue_number, record.clone());
            }
        }
        // A newer marker proves only that the exact legacy launch-failure
        // migration ran. Failures carrying a real window or a terminal
        // NeedsHuman state are explicitly outside that migration's target set,
        // so they may have been created locally after the newer disk snapshot.
        // Preserve those local companions without reviving an unqualified
        // legacy failure that the other process intentionally removed.
        for (issue_number, message) in &old_failures {
            let local_needs_human = self
                .autonomous_records
                .get(issue_number)
                .filter(|record| record.phase == AutonomousPhase::NeedsHuman);
            let inbox_needs_human = self
                .inbox_item(*issue_number)
                .is_some_and(|item| item.state == MonitorInboxState::NeedsHuman);
            let failed_window = self.failed_windows.get(issue_number);
            if (local_needs_human.is_none() && !inbox_needs_human && failed_window.is_none())
                || disk_failures.contains_key(issue_number)
                || self.launched_windows.contains_key(issue_number)
                || self.merged_issues.contains(issue_number)
            {
                continue;
            }
            disk_failures.insert(*issue_number, message.clone());
            if let Some(window_id) = failed_window {
                disk_windows.insert(*issue_number, window_id.clone());
            }
            if let Some(record) = local_needs_human {
                disk_needs_human.insert(*issue_number, record.clone());
            }
        }
        let needs_human_issue_numbers = disk_needs_human
            .keys()
            .copied()
            .chain(
                self.inbox
                    .iter()
                    .filter(|item| item.state == MonitorInboxState::NeedsHuman)
                    .map(|item| item.issue.number),
            )
            .collect::<BTreeSet<_>>();
        let removed = old_failures
            .keys()
            .filter(|issue_number| !disk_failures.contains_key(issue_number))
            .copied()
            .collect::<BTreeSet<_>>();

        self.failed_issues = disk_failures;
        self.failed_windows = disk_windows;
        for issue_number in self.failed_issues.keys().copied().collect::<Vec<_>>() {
            self.queue.retain(|queued| *queued != issue_number);
            self.pending_launches
                .retain(|pending| pending.issue_number != issue_number);
            self.pending_launch_deliveries
                .retain(|delivery| delivery.issue_number != issue_number);
            self.queued_launch_session_strategies.remove(&issue_number);
            if !self.launched_windows.contains_key(&issue_number) {
                self.active_launches
                    .retain(|active| *active != issue_number);
                self.launching_claimed_at.remove(&issue_number);
            }
            if let Some(record) = disk_needs_human.get(&issue_number) {
                self.autonomous_records.insert(issue_number, record.clone());
            }
        }
        self.inbox.retain(|item| {
            !(removed.contains(&item.issue.number)
                && matches!(
                    item.state,
                    MonitorInboxState::LaunchFailed | MonitorInboxState::AgentFailed
                ))
        });
        for item in &mut self.inbox {
            let Some(message) = self.failed_issues.get(&item.issue.number) else {
                continue;
            };
            if matches!(
                item.state,
                MonitorInboxState::Merged | MonitorInboxState::Released
            ) {
                continue;
            }
            if needs_human_issue_numbers.contains(&item.issue.number) {
                item.state = MonitorInboxState::NeedsHuman;
            } else if item.state != MonitorInboxState::LaunchFailed {
                item.state = MonitorInboxState::AgentFailed;
            }
            item.error_message = Some(message.clone());
            item.launched_window_id = None;
        }
        if self
            .last_error
            .as_ref()
            .is_some_and(|error| old_banners.contains(error))
        {
            self.last_error = self.first_failed_issue_banner();
        }
        self.legacy_git_launch_failure_migration_version =
            disk.legacy_git_launch_failure_migration_version;
        true
    }

    fn merge_equal_marker_disk_failures(&mut self, disk: &IssueMonitorPrefs) {
        self.merge_disk_only_failures(disk, false);
    }

    fn merge_older_marker_disk_failures(&mut self, disk: &IssueMonitorPrefs) {
        self.merge_disk_only_failures(disk, true);
    }

    fn merge_disk_only_failures(
        &mut self,
        disk: &IssueMonitorPrefs,
        skip_migration_tombstones: bool,
    ) {
        for failed in &disk.failed_issues {
            let issue_number = failed.issue_number;
            if failed.message.trim().is_empty()
                || (skip_migration_tombstones && self.is_migrated_failure_tombstone(failed))
                || disk
                    .released_failures
                    .iter()
                    .any(|release| release.issue_number == issue_number)
                || self.failed_issues.contains_key(&issue_number)
                || self.launched_windows.contains_key(&issue_number)
                || self.merged_issues.contains(&issue_number)
            {
                continue;
            }

            // A positive disk failure with no matching disk release is newer
            // than a stale process-local recovery fence.
            self.released_failures.remove(&issue_number);
            self.failed_issues
                .insert(issue_number, failed.message.clone());
            let needs_human = disk
                .autonomous_records
                .iter()
                .find(|record| {
                    record.issue_number == issue_number
                        && record.phase == AutonomousPhase::NeedsHuman
                })
                .cloned();
            if let Some(record) = &needs_human {
                self.autonomous_records.insert(issue_number, record.clone());
            }
            match failed.window_id.as_ref().filter(|id| !id.is_empty()) {
                Some(window_id) => {
                    self.failed_windows.insert(issue_number, window_id.clone());
                }
                None => {
                    self.failed_windows.remove(&issue_number);
                }
            }
            self.queue.retain(|queued| *queued != issue_number);
            self.active_launches
                .retain(|active| *active != issue_number);
            self.launching_claimed_at.remove(&issue_number);
            self.pending_launches
                .retain(|pending| pending.issue_number != issue_number);
            self.pending_launch_deliveries
                .retain(|delivery| delivery.issue_number != issue_number);
            self.queued_launch_session_strategies.remove(&issue_number);
            if let Some(item) = self
                .inbox
                .iter_mut()
                .find(|item| item.issue.number == issue_number)
            {
                if matches!(
                    item.state,
                    MonitorInboxState::Merged
                        | MonitorInboxState::Released
                        | MonitorInboxState::NeedsHuman
                ) {
                    continue;
                }
                if needs_human.is_some() {
                    item.state = MonitorInboxState::NeedsHuman;
                } else if item.state != MonitorInboxState::LaunchFailed {
                    item.state = MonitorInboxState::AgentFailed;
                }
                item.error_message = Some(failed.message.clone());
                item.launched_window_id = None;
            }
        }
    }

    fn apply_legacy_git_launch_failure_migration(&mut self, project_root: &Path) {
        if self.legacy_git_launch_failure_migration_version
            >= LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        {
            return;
        }

        let targets = self
            .failed_issues
            .iter()
            .filter_map(|(issue_number, message)| {
                let needs_human = self
                    .autonomous_records
                    .get(issue_number)
                    .is_some_and(|record| record.phase == AutonomousPhase::NeedsHuman)
                    || self
                        .inbox_item(*issue_number)
                        .is_some_and(|item| item.state == MonitorInboxState::NeedsHuman);
                (!needs_human
                    && !self.failed_windows.contains_key(issue_number)
                    && is_legacy_git_launch_failure_for_project(message, project_root))
                .then_some((*issue_number, message.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let removed_banners = targets
            .iter()
            .map(|(issue_number, message)| format!("issue #{issue_number}: {message}"))
            .collect::<BTreeSet<_>>();

        self.legacy_git_launch_failure_migration_tombstones
            .extend(targets.clone());

        for issue_number in targets.keys() {
            self.failed_issues.remove(issue_number);
        }
        self.inbox.retain(|item| {
            !(targets.contains_key(&item.issue.number)
                && matches!(
                    item.state,
                    MonitorInboxState::LaunchFailed | MonitorInboxState::AgentFailed
                ))
        });
        if self
            .last_error
            .as_ref()
            .is_some_and(|error| removed_banners.contains(error))
        {
            self.last_error = self.first_failed_issue_banner();
        }
        self.legacy_git_launch_failure_migration_version =
            LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION;
    }

    fn first_failed_issue_banner(&self) -> Option<String> {
        self.failed_issues
            .iter()
            .next()
            .map(|(issue_number, message)| format!("issue #{issue_number}: {message}"))
    }

    /// #3223 follow-up (codex P1): absorb the OTHER process's in-flight launch
    /// accounting from disk. The GUI and the daemon both claim launches; the
    /// daemon only refreshed profile/tuning, so GUI-written `launching`/
    /// `launched` entries were invisible to it — it saw free slots (over-cap
    /// claims) and its next persist dropped the GUI's in-flight claims.
    /// Union-merge: entries already known in memory win; removals propagate via
    /// the existing control frames (Launched / LaunchFailed / WindowClosed).
    /// A merge completion produced by the current scan is terminal, however,
    /// and must win over the stale in-flight entry still present on disk until
    /// this transaction commits the new merged marker.
    pub fn merge_inflight_launches_from_disk(&mut self, disk: &IssueMonitorPrefs) {
        let reopened_completion_fences = self.local_reopened_completion_fences(disk);
        self.merge_inflight_launches_from_disk_excluding(disk, &reopened_completion_fences);
    }

    fn merge_inflight_launches_from_disk_excluding(
        &mut self,
        disk: &IssueMonitorPrefs,
        reopened_completion_fences: &BTreeSet<u64>,
    ) {
        for launched in &disk.launched_issues {
            if launched.window_id.is_empty()
                || self.merged_issues.contains(&launched.issue_number)
                || reopened_completion_fences.contains(&launched.issue_number)
            {
                continue;
            }
            self.launched_windows
                .entry(launched.issue_number)
                .or_insert_with(|| launched.window_id.clone());
            // PR #3787 review: a completion can clear the local claim while a
            // newer window stays bound. Import the disk claim only when the
            // retained window is the disk row's own window — a stale claim
            // paired with a replaced window would invalidate the
            // exact-identity fence (live_claim_id / stop_only /
            // failover_restart / requeue_exact_window).
            let window_matches_disk = self
                .launched_windows
                .get(&launched.issue_number)
                .is_some_and(|window_id| *window_id == launched.window_id);
            if window_matches_disk {
                if let Some(claim_id) = disk.launched_claims.get(&launched.issue_number) {
                    self.launched_claims
                        .entry(launched.issue_number)
                        .or_insert_with(|| claim_id.clone());
                }
            }
            if !self.active_launches.contains(&launched.issue_number) {
                self.active_launches.push(launched.issue_number);
            }
        }
        for entry in &disk.launching_issues {
            if self.merged_issues.contains(&entry.issue_number)
                || reopened_completion_fences.contains(&entry.issue_number)
            {
                continue;
            }
            if !self.active_launches.contains(&entry.issue_number) {
                self.active_launches.push(entry.issue_number);
                if let Some(claimed_at) = &entry.claimed_at {
                    self.launching_claimed_at
                        .insert(entry.issue_number, claimed_at.clone());
                }
            }
        }
    }

    /// #3223 follow-up (codex P2 / coderabbit): release claimed-but-unbound
    /// launches whose claim anchor is older than `claim_ttl_secs`. A crash
    /// between the claim-save and the launch ACK would otherwise hold a
    /// max-active slot forever. Entries restored without an anchor (legacy
    /// bare-id shape) are stamped `now` so their clock starts here. Released
    /// issues return to `Queued` and re-enter the queue so the next scan can
    /// relaunch them (mirroring the expired GitHub claim, which lapses after
    /// the same TTL).
    pub fn expire_stale_unbound_launches(&mut self, now: &str) -> Vec<u64> {
        let ttl = self.config.claim_ttl_secs as i64;
        let unbound: Vec<u64> = self
            .active_launches
            .iter()
            .filter(|issue_number| !self.launched_windows.contains_key(issue_number))
            .copied()
            .collect();
        let mut expired = Vec::new();
        for issue_number in unbound {
            if self
                .pending_launch_deliveries
                .iter()
                .any(|delivery| delivery.issue_number == issue_number)
            {
                continue;
            }
            match self.launching_claimed_at.get(&issue_number) {
                Some(claimed_at) => {
                    let stale =
                        rfc3339_elapsed_secs(claimed_at, now).is_some_and(|elapsed| elapsed >= ttl);
                    if stale {
                        self.active_launches
                            .retain(|active| *active != issue_number);
                        self.launching_claimed_at.remove(&issue_number);
                        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
                        if !self.queue.contains(&issue_number) {
                            self.queue.push_back(issue_number);
                            self.apply_priority_order_to_queue();
                        }
                        expired.push(issue_number);
                    }
                }
                None => {
                    // Legacy entry without an anchor: start its clock now.
                    self.launching_claimed_at
                        .insert(issue_number, now.to_string());
                }
            }
        }
        expired
    }

    fn saved_launch_provider(&self) -> Option<String> {
        self.launch_profile
            .as_ref()
            .and_then(|profile| normalize_issue_monitor_provider(&profile.agent_id))
    }

    fn provider_quota_hold_at(&self, now: &str) -> Option<IssueMonitorProviderQuotaHold> {
        let provider = self.saved_launch_provider()?;
        let reset_at = self.provider_quota_holds.get(&provider)?;
        let deadline = parse_rfc3339_utc(reset_at)?;
        let now = parse_rfc3339_utc(now)?;
        (deadline > now).then(|| IssueMonitorProviderQuotaHold {
            evidence: self.provider_quota_hold_evidence.get(&provider).cloned(),
            provider,
            reset_at: format_rfc3339_utc(deadline),
        })
    }

    /// Issue #3923 AC-1: every provider hold still in force at `now`, whether
    /// or not it matches the saved launch profile.
    fn active_provider_quota_holds_at(&self, now: &str) -> Vec<IssueMonitorProviderQuotaHold> {
        let Some(now) = parse_rfc3339_utc(now) else {
            return Vec::new();
        };
        self.provider_quota_holds
            .iter()
            .filter_map(|(provider, reset_at)| {
                let deadline = parse_rfc3339_utc(reset_at)?;
                (deadline > now).then(|| IssueMonitorProviderQuotaHold {
                    provider: provider.clone(),
                    reset_at: format_rfc3339_utc(deadline),
                    evidence: self.provider_quota_hold_evidence.get(provider).cloned(),
                })
            })
            .collect()
    }

    fn saved_profile_provider_is_held_at(&self, now: &str) -> bool {
        self.provider_quota_hold_at(now).is_some()
    }

    pub fn status_view(&self) -> IssueMonitorStatusView {
        let now = format_rfc3339_utc(chrono::Utc::now());
        self.status_view_with_quota_hold(self.provider_quota_hold_at(&now))
    }

    fn status_view_with_quota_hold(
        &self,
        quota_hold: Option<IssueMonitorProviderQuotaHold>,
    ) -> IssueMonitorStatusView {
        let last_error = self.last_error.clone().or_else(|| {
            self.failed_issues
                .iter()
                .next()
                .map(|(issue_number, message)| format!("issue #{issue_number}: {message}"))
        });
        IssueMonitorStatusView {
            enabled: self.config.enabled,
            state: if !self.config.enabled {
                "disabled".to_string()
            } else if last_error.is_some() {
                "error".to_string()
            } else if self.launch_auth_required {
                "auth_required".to_string()
            } else if quota_hold.is_some() {
                "quota_hold".to_string()
            } else if !self.active_launches.is_empty() {
                if self
                    .active_launches
                    .iter()
                    .all(|issue_number| self.launched_windows.contains_key(issue_number))
                {
                    "active".to_string()
                } else {
                    "launching".to_string()
                }
            } else if self.launch_profile.is_none()
                && !self.queue.is_empty()
                && self.active_launches.is_empty()
            {
                "settings_required".to_string()
            } else {
                "idle".to_string()
            },
            queue_len: self.queue.len(),
            active_count: self.active_launches.len(),
            max_active_agents: self.config.max_active,
            total_candidates: self.inbox.len(),
            active_issue_number: self.active_issue_number(),
            last_scan_at: self.last_scan_at.clone(),
            last_error,
            launch_profile_source: self
                .launch_profile
                .as_ref()
                .map(|_| IssueMonitorLaunchProfileSource::Saved)
                .unwrap_or(IssueMonitorLaunchProfileSource::Default),
            launch_profile_summary: self
                .launch_profile
                .clone()
                .map(LaunchWizardPreviousProfile::from)
                .as_ref()
                .map(issue_monitor_launch_profile_summary)
                .unwrap_or_else(|| "configure before auto start".to_string()),
            autonomous_mode: self.autonomous_mode,
            quota_hold,
            autonomous_issues: self
                .autonomous_records
                .values()
                .map(|record| {
                    let needs_human = record.phase == AutonomousPhase::NeedsHuman;
                    let handoff = self.open_autonomous_handoff(record.issue_number);
                    AutonomousIssueSummary {
                        issue_number: record.issue_number,
                        phase: record.phase,
                        attempts: record.attempts,
                        needs_human,
                        needs_human_reason: needs_human
                            .then(|| self.failed_issues.get(&record.issue_number).cloned())
                            .flatten(),
                        needs_human_kind: needs_human.then_some(record.needs_human_kind).flatten(),
                        steering: record.steering.clone(),
                        pending_question: handoff.map(|handoff| AutonomousPendingQuestion {
                            handoff_id: handoff.handoff_id.clone(),
                            question: handoff.question.clone(),
                            options: handoff
                                .options
                                .iter()
                                .map(|option| option.label.clone())
                                .collect(),
                            reason_code: handoff.reason_code.as_str().to_string(),
                            session_id: handoff.session_id.clone(),
                            provider: handoff.provider.clone(),
                            created_at: handoff.created_at.clone(),
                            // A stored session id is what makes the parked work
                            // resumable rather than restartable.
                            resumable: !handoff.session_id.trim().is_empty(),
                        }),
                    }
                })
                .collect(),
        }
    }

    fn completion_anomaly(&self, item: &IssueMonitorInboxItem) -> (bool, Option<String>) {
        if item.state != MonitorInboxState::Merged
            || item.issue.state != IssueMonitorIssueState::Open
        {
            return (false, None);
        }
        let Some(record) = self.completion_records.get(&item.issue.number) else {
            return (true, Some("legacy_unverified".to_string()));
        };
        if record.evidence == IssueCompletionEvidence::Legacy {
            return (true, Some("legacy_unverified".to_string()));
        }
        let is_spec = item
            .issue
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("gwt-spec"));
        if !is_spec {
            return (true, Some("github_issue_open".to_string()));
        }
        if Self::issue_completion_decision(&item.issue) == IssueCompletionDecision::Incomplete {
            return (true, Some("incomplete_spec_tasks".to_string()));
        }
        (false, None)
    }

    pub fn agent_status(&self) -> IssueMonitorAgentStatus {
        let now = format_rfc3339_utc(chrono::Utc::now());
        self.agent_status_without_scan_at(&now)
    }

    fn agent_status_without_scan_at(&self, now: &str) -> IssueMonitorAgentStatus {
        let status = self.status_view_with_quota_hold(self.provider_quota_hold_at(now));
        IssueMonitorAgentStatus {
            queue: self.queued_issue_numbers(),
            active_launches: self.active_issue_numbers(),
            max_active: self.config.max_active.max(1),
            enabled: self.config.enabled,
            autonomous_mode: self.autonomous_mode,
            has_launch_profile: self.has_launch_profile(),
            quota_hold: status.quota_hold.clone(),
            provider_quota_holds: self.active_provider_quota_holds_at(now),
            needs_human: status
                .autonomous_issues
                .iter()
                .filter(|summary| summary.needs_human)
                .map(|summary| summary.issue_number)
                .collect(),
            inbox: self
                .inbox
                .iter()
                .map(|item| {
                    let (recoverable_merged, completion_reason) = self.completion_anomaly(item);
                    IssueMonitorInboxSummary {
                        issue_number: item.issue.number,
                        state: item.state,
                        github_state: item.issue.state,
                        issue_updated_at: item.issue.updated_at.clone(),
                        readiness: item.issue.readiness,
                        recoverable_merged,
                        completion_reason,
                        blocked_by_owner: item.blocked_by_owner.clone(),
                        launched_window_id: item.launched_window_id.clone(),
                        error_message: item.error_message.clone(),
                        // SPEC-3431 FR-068: the autonomous record already carries
                        // the heartbeat that hook arrivals refresh. Surfacing it here
                        // rather than adding a parallel field keeps one clock, so
                        // a reader and the stuck detector can never disagree.
                        last_activity_at: self
                            .autonomous_records
                            .get(&item.issue.number)
                            .and_then(|record| record.last_heartbeat.clone()),
                        // Issue #3616 AC-3/AC-4: same record, same clock as the
                        // `retry_ready` gate, so what the PM reads is exactly
                        // what holds the launch back.
                        retry_not_before: self
                            .autonomous_records
                            .get(&item.issue.number)
                            .and_then(|record| record.retry_not_before.clone()),
                        retry_hold_reason: self
                            .autonomous_records
                            .get(&item.issue.number)
                            .and_then(|record| record.retry_hold_reason.clone()),
                        // SPEC-3431 FR-024/FR-033: the identity `stop_only` demands
                        // has to be readable from the same snapshot, or the exact
                        // match it enforces is unsatisfiable from the PM's side.
                        claim_id: self.live_claim_id(item.issue.number),
                        delivery_id: self.pending_launch_delivery_id(item.issue.number),
                        waiting: self.autonomous_wait(item.issue.number).map(|wait| {
                            IssueMonitorWaitSummary {
                                reason: wait.reason.clone(),
                                resume_condition: wait.resume_condition.clone(),
                                since: wait.since.clone(),
                                expires_at: autonomous_wait_expires_at(&wait.since),
                            }
                        }),
                        steering: self
                            .autonomous_record(item.issue.number)
                            .and_then(|record| record.steering.clone()),
                    }
                })
                .collect(),
            last_error: status.last_error,
            last_scan_at: status.last_scan_at,
            scan_stall: None,
        }
    }

    /// The agent-facing snapshot with Issue #3633's scan-cadence check applied.
    ///
    /// [`Self::agent_status`] answers "what is in the queue"; this answers the
    /// question a reader actually has when the queue stops moving: "is anything
    /// still scanning?". Both the daemon projection and the offline CLI
    /// fallback publish through here so the two branches cannot disagree about
    /// whether a project is being driven.
    pub fn agent_status_at(&self, now: &str) -> IssueMonitorAgentStatus {
        let mut status = self.agent_status_without_scan_at(now);
        status.scan_stall = self.scan_stall_at(now);
        status
    }

    /// Describe a stopped scan cadence, or `None` while it is healthy.
    ///
    /// A disabled monitor is stopped on purpose and never reports a stall —
    /// flagging it would train readers to ignore the field. An unparsable or
    /// future timestamp fails open for the same reason.
    fn scan_stall_at(&self, now: &str) -> Option<String> {
        if !self.config.enabled {
            return None;
        }
        let Some(last_scan_at) = self.last_scan_at.as_deref() else {
            // The #3633 state itself: enabled, queued work, and no driver has
            // ever completed a scan for this project.
            return Some(
                "Issue Monitor has never completed a scan for this project; no driver is running"
                    .to_string(),
            );
        };
        let elapsed_secs = u64::try_from(rfc3339_elapsed_secs(last_scan_at, now)?).ok()?;
        let stall_after = self.config.poll_interval_secs.saturating_mul(3);
        (elapsed_secs >= stall_after).then(|| {
            format!(
                "Issue Monitor scan stalled for {elapsed_secs}s (poll interval {interval}s); \
                 last scan at {last_scan_at}",
                interval = self.config.poll_interval_secs
            )
        })
    }

    /// Project scan staleness onto the existing status error surface using a
    /// caller-supplied clock so daemon publication and tests stay deterministic.
    pub fn status_view_at(&self, now: &str) -> IssueMonitorStatusView {
        let mut status = self.status_view_with_quota_hold(self.provider_quota_hold_at(now));
        if !status.enabled || status.last_error.is_some() {
            return status;
        }
        let Some(last_scan_at) = status.last_scan_at.as_deref() else {
            return status;
        };
        let Some(elapsed_secs) = rfc3339_elapsed_secs(last_scan_at, now) else {
            return status;
        };
        let Ok(elapsed_secs) = u64::try_from(elapsed_secs) else {
            return status;
        };
        if elapsed_secs >= self.config.poll_interval_secs.saturating_mul(3) {
            status.state = "error".to_string();
            status.last_error = Some(format!(
                "Issue Monitor scan stalled; last scan at {last_scan_at}"
            ));
        }
        status
    }

    /// SPEC #3200 T-001/FR-001: read the opt-in autonomous mode flag.
    pub fn autonomous_mode(&self) -> bool {
        self.autonomous_mode
    }

    /// SPEC #3200 T-047/FR-001: toggle unattended autonomous mode. Default OFF
    /// keeps the SPEC #3165 human-gated behavior exactly.
    pub fn set_autonomous_mode(&mut self, enabled: bool) {
        self.autonomous_mode = enabled;
    }

    /// Current durable remote-effect authority generation.
    pub fn effect_authority_epoch(&self) -> u64 {
        self.effect_authority_epoch
    }

    /// Read the durable journal without granting callers mutable access around
    /// the exact delivery-tuple guards.
    pub fn pending_effects(&self) -> &[PendingIssueMonitorEffect] {
        &self.pending_effects
    }

    fn claim_effect_is_blocked_by_provider_hold(
        &self,
        payload: &IssueMonitorEffectPayload,
    ) -> bool {
        matches!(
            payload,
            IssueMonitorEffectPayload::AcquireClaim { heartbeat_at, .. }
                if self.saved_profile_provider_is_held_at(heartbeat_at)
        )
    }

    /// Add a fully formed scan proposal. Only a current-authority Prepared
    /// entry with a fresh stable id may enter the local proposal journal.
    pub fn prepare_effect(&mut self, effect: PendingIssueMonitorEffect) -> bool {
        if effect.effect_id.is_empty()
            || effect.state != IssueMonitorEffectState::Prepared
            || effect.authority_epoch != self.effect_authority_epoch
            || self.claim_effect_is_blocked_by_provider_hold(&effect.payload)
            || self
                .pending_effects
                .iter()
                .any(|pending| pending.effect_id == effect.effect_id)
        {
            return false;
        }
        self.pending_effects.push(effect);
        true
    }

    /// Add one Prepared effect under the current authority. An empty or reused
    /// stable id is rejected without changing the journal.
    pub fn prepare_pending_effect(
        &mut self,
        effect_id: impl Into<String>,
        payload: IssueMonitorEffectPayload,
    ) -> Option<IssueMonitorEffectAttemptKey> {
        let effect_id = effect_id.into();
        if effect_id.is_empty()
            || self.claim_effect_is_blocked_by_provider_hold(&payload)
            || self
                .pending_effects
                .iter()
                .any(|effect| effect.effect_id == effect_id)
        {
            return None;
        }
        let effect =
            PendingIssueMonitorEffect::prepared(effect_id, self.effect_authority_epoch, payload);
        let key = effect.attempt_key();
        self.pending_effects.push(effect);
        Some(key)
    }

    pub fn mark_pending_effect_attempting(&mut self, key: &IssueMonitorEffectAttemptKey) -> bool {
        mark_effect_attempting(&mut self.pending_effects, key)
    }

    pub fn complete_pending_effect(
        &mut self,
        key: &IssueMonitorEffectAttemptKey,
    ) -> Option<PendingIssueMonitorEffect> {
        complete_attempting_effect(&mut self.pending_effects, key)
    }

    pub fn retry_pending_effect(&mut self, key: &IssueMonitorEffectAttemptKey) -> bool {
        let Some(effect) = self.pending_effects.iter_mut().find(|effect| {
            effect.state == IssueMonitorEffectState::Attempting && effect_matches_key(effect, key)
        }) else {
            return false;
        };
        let Some(next_attempt) = effect.attempt.checked_add(1) else {
            return false;
        };
        effect.attempt = next_attempt;
        effect.state = IssueMonitorEffectState::Prepared;
        true
    }

    pub fn set_autonomous_mode_with_effect_revocation(&mut self, enabled: bool) -> Option<u64> {
        advance_autonomous_effect_authority(
            &mut self.autonomous_mode,
            &mut self.effect_authority_epoch,
            &mut self.pending_effects,
            enabled,
        )
    }

    /// SPEC #3200 T-032/FR-003/004: the pure two-stage opt-in pre-gate — an issue
    /// is an autonomous candidate ONLY when autonomous mode is on AND the issue
    /// carries the `auto-merge` label. Branch-protection / acceptance-criteria /
    /// attempt safety preconditions are applied later by [`autonomous_eligibility`].
    pub fn is_autonomous_two_stage_candidate(&self, issue: &IssueMonitorIssue) -> bool {
        self.autonomous_mode && issue_has_auto_merge_label(issue)
    }

    /// #3165/#3200 error-window lifecycle: decide whether a just-failed agent
    /// window should be auto-closed. An autonomous (two-stage opt-in) issue
    /// auto-closes its stale window so the bounded retry relaunches into a clean
    /// canvas; a default (non-autonomous) issue KEEPS its failed window so the
    /// human can inspect the error output and relaunch explicitly via Launch Now.
    /// The issue is looked up in the inbox, where a freshly recorded failure
    /// still carries the issue and its labels.
    pub fn should_autoclose_failed_window(&self, issue_number: u64) -> bool {
        self.inbox
            .iter()
            .find(|item| item.issue.number == issue_number)
            .map(|item| self.is_autonomous_two_stage_candidate(&item.issue))
            .unwrap_or(false)
    }

    /// #3165 error-window lifecycle: remove and return the stale agent window id
    /// retained for a failed issue, so an explicit Launch Now (default mode) can
    /// close it before relaunching into a fresh window. `None` when no stale
    /// window was recorded for the issue.
    pub fn take_failed_window(&mut self, issue_number: u64) -> Option<String> {
        self.failed_windows.remove(&issue_number)
    }

    /// SPEC #3200 T-041 (FR-003..FR-010): pre-launch autonomous decision + state
    /// capture for one candidate, given the freshly fetched base-branch
    /// `branch_protection`. Composes the pure [`autonomous_eligibility`] predicate
    /// with the issue body's acceptance criteria and the persisted attempt count,
    /// then applies the side effects:
    ///
    /// - non-two-stage candidate ⇒ `HumanGate` (caller uses the existing #3165
    ///   human-gated launch path, no autonomous state created);
    /// - `NeedsHuman` ⇒ escalate (terminal, removed from the launch queue);
    /// - `Eligible` ⇒ capture the acceptance snapshot + set `Implementing` phase
    ///   (idempotent: only on a fresh, not-yet-launched candidate).
    ///
    /// Returns the [`EligibilityDecision`] so the caller knows whether to launch.
    /// Default `autonomous_mode` OFF makes this a no-op `HumanGate` for every
    /// issue, preserving SPEC #3165 behavior exactly.
    pub fn prepare_autonomous_candidate(
        &mut self,
        issue: &IssueMonitorIssue,
        branch_protection: &gwt_git::branch_protection::BranchProtectionStatus,
        now: &str,
    ) -> EligibilityDecision {
        if !self.is_autonomous_two_stage_candidate(issue) {
            return EligibilityDecision::HumanGate("not an autonomous candidate".to_string());
        }
        let number = issue.number;
        if self.inbox_item(number).is_some_and(|item| {
            matches!(
                item.state,
                MonitorInboxState::NotReady | MonitorInboxState::HoldExcluded
            )
        }) {
            return EligibilityDecision::HumanGate(
                "candidate is excluded from automatic launch".to_string(),
            );
        }
        // Idempotency: a candidate already in flight is left alone.
        if self.active_launches.contains(&number) {
            return EligibilityDecision::Eligible;
        }
        // SPEC #3200 T-043/FR-029: honor the transient-retry backoff — a candidate
        // whose backoff window has not elapsed is skipped this scan (no capture,
        // no escalation) so the exponential backoff is actually enforced.
        if !self.retry_ready_for_saved_profile(number, now) {
            return EligibilityDecision::HumanGate("retry backoff window not elapsed".to_string());
        }
        let criteria = crate::issue_monitor_gate::classify_acceptance_criteria(
            issue.body.as_deref().unwrap_or(""),
        );
        let is_needs_human = self
            .autonomous_record(number)
            .map(|record| record.phase == AutonomousPhase::NeedsHuman)
            .unwrap_or(false);
        let decision = autonomous_eligibility(
            self.autonomous_mode,
            issue_has_auto_merge_label(issue),
            &criteria,
            branch_protection,
            is_needs_human,
        );
        match &decision {
            EligibilityDecision::Eligible => {
                self.capture_acceptance_snapshot(number, criteria.snapshot());
                self.set_autonomous_phase(number, AutonomousPhase::Implementing);
                // The launch consumes the scheduled retry, so the backoff marker
                // is cleared to avoid stale state on the in-flight attempt.
                let record = self.autonomous_record_mut(number);
                record.retry_not_before = None;
                record.retry_hold_reason = None;
                record.retry_hold_provider = None;
                // Issue #3844: a wait belongs to one launch; never inherit it.
                record.wait = None;
                // SPEC #3200 T-045/FR-025: seed the liveness baseline at launch so
                // stuck detection actually fires for an agent that hangs without
                // producing a PR within stuck_timeout_secs. Real progress (a
                // heartbeat, or the Implementing→Reviewing transition) resets it.
                self.record_autonomous_heartbeat(number, now);
            }
            EligibilityDecision::NotReady(reason) => {
                self.mark_autonomous_not_ready(number, reason.clone());
            }
            // Already parked: the predicate reports it and creates nothing new.
            EligibilityDecision::NeedsHuman(_) | EligibilityDecision::HumanGate(_) => {}
        }
        decision
    }

    /// Issue #3944 AC-4: project a readiness failure as `not_ready` with its
    /// reason — the same row state a gwt-spec Issue without plan/tasks gets —
    /// and drop the row from the launch queue. The next scan re-evaluates it,
    /// so fixing the Issue body or the repository settings is the whole remedy.
    fn mark_autonomous_not_ready(&mut self, issue_number: u64, reason: String) {
        self.queue.retain(|queued| *queued != issue_number);
        revoke_uncommitted_claims_for_issue(
            &mut self.pending_effects,
            self.effect_authority_epoch,
            issue_number,
        );
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::NotReady;
            item.exclusion_reason = Some(reason);
            item.error_message = None;
        }
    }

    /// SPEC #3200: autonomous issues currently in flight (phase Implementing /
    /// Reviewing / Delivering) — the set the daemon orchestration loop advances
    /// each tick. Terminal/Idle phases are excluded.
    pub fn autonomous_in_flight_issues(&self) -> Vec<u64> {
        self.autonomous_records
            .values()
            .filter(|record| Self::is_in_flight_phase(record.phase))
            .map(|record| record.issue_number)
            .collect()
    }

    fn is_in_flight_phase(phase: AutonomousPhase) -> bool {
        matches!(
            phase,
            AutonomousPhase::Implementing
                | AutonomousPhase::Reviewing
                | AutonomousPhase::Delivering
        )
    }

    /// SPEC #3200 (review follow-up): true when `issue_number` has an autonomous
    /// record actively in flight (`Implementing` / `Reviewing` / `Delivering`).
    /// A launch/agent failure for such an issue must be routed through the
    /// autonomous retry/backoff/escalation machinery rather than the plain
    /// human-gated launch-failed path, or the record strands in a non-`Idle`
    /// phase forever (e.g. `Reviewing` after a failed review-agent spawn, where
    /// the daemon waits for a verdict that will never arrive).
    pub fn is_autonomous_in_flight(&self, issue_number: u64) -> bool {
        self.autonomous_records
            .get(&issue_number)
            .is_some_and(|record| Self::is_in_flight_phase(record.phase))
    }

    /// SPEC #3200: transition Implementing→Reviewing once the implementation
    /// agent has produced an open PR. Binds the PR number and the reviewed SHA
    /// (the TOCTOU anchor) and clears any prior verdict.
    pub fn begin_review(
        &mut self,
        issue_number: u64,
        pr_number: u64,
        reviewed_sha: impl Into<String>,
    ) {
        let record = self.autonomous_record_mut(issue_number);
        record.phase = AutonomousPhase::Reviewing;
        record.pr_number = Some(pr_number);
        record.reviewed_sha = Some(reviewed_sha.into());
        record.review_passed = None;
    }

    /// SPEC #3200 FR-015: record the independent-review verdict for the in-flight
    /// reviewed SHA. The gate is evaluated on the next tick.
    pub fn record_review_verdict(&mut self, issue_number: u64, passed: bool) {
        self.autonomous_record_mut(issue_number).review_passed = Some(passed);
    }

    /// SPEC #3200 FR-015/FR-016: apply a raw review verdict reported by the
    /// (untrusted) review agent. The verdict is parsed and judged HERE (the
    /// trusted daemon), not by the agent — and only accepted when its
    /// `reviewed_sha` matches the SHA this issue is actually under review for
    /// (a stale / wrong-SHA verdict is rejected). Returns `None` when rejected
    /// (no record / SHA mismatch), else `Some(passed)`.
    pub fn apply_review_verdict(
        &mut self,
        issue_number: u64,
        reviewed_sha: &str,
        verdict_raw: &str,
    ) -> Option<bool> {
        let record = self.autonomous_records.get(&issue_number)?;
        // Reject a verdict that is not for the SHA we are reviewing.
        if record.reviewed_sha.as_deref() != Some(reviewed_sha) {
            return None;
        }
        let required = record
            .acceptance_snapshot
            .as_ref()
            .map(|snapshot| snapshot.ids.clone())
            .unwrap_or_default();
        let outcome = crate::issue_monitor_review::evaluate_review_verdict(verdict_raw, &required);
        let passed = matches!(
            outcome,
            crate::issue_monitor_review::ReviewGateOutcome::Pass
        );
        self.record_review_verdict(issue_number, passed);
        Some(passed)
    }

    /// SPEC #3200: transition Reviewing→Delivering once the strong gate passes
    /// (the auto-merge is being armed).
    pub fn begin_delivering(&mut self, issue_number: u64) {
        self.set_autonomous_phase(issue_number, AutonomousPhase::Delivering);
    }

    /// SPEC #3200 FR-034 (codex #3217 review): announce a SUCCESSFUL auto-merge
    /// arm. Called by the worker only after `gh pr merge --auto` actually
    /// succeeded — never before — so the operator toast cannot claim an arm
    /// that failed (the merge helper's fail-closed contract).
    pub fn record_auto_merge_armed(&mut self, issue_number: u64) {
        self.push_autonomous_notice(
            "info",
            issue_number,
            format!("Issue #{issue_number} gate passed — auto-merge armed"),
        );
    }

    /// SPEC #3200 FR-009..FR-016: assemble the strong-gate inputs for an issue
    /// under review, from the record (reviewed SHA + review verdict + acceptance
    /// snapshot) and freshly-fetched signals (branch protection, CI rollup JSON,
    /// the current HEAD SHA, the current Issue body). Returns `None` when the
    /// review verdict has not yet arrived (gate not ready ⇒ caller waits).
    pub fn autonomous_gate_inputs(
        &self,
        issue_number: u64,
        branch_protection: gwt_git::branch_protection::BranchProtectionStatus,
        ci_rollup_json: &str,
        current_head_sha: &str,
        current_issue_body: &str,
    ) -> Option<crate::issue_monitor_gate::AutonomousGateInputs> {
        use crate::issue_monitor_gate::{classify_acceptance_criteria, classify_ci_rollup};
        use crate::issue_monitor_review::ReviewGateOutcome;
        let record = self.autonomous_records.get(&issue_number)?;
        let reviewed_sha = record.reviewed_sha.clone()?;
        // Review must have returned a verdict; otherwise the gate is not ready.
        let review_passed = record.review_passed?;
        let required_checks = match &branch_protection {
            gwt_git::branch_protection::BranchProtectionStatus::Verified { required_checks } => {
                required_checks.clone()
            }
            _ => Vec::new(),
        };
        let ci = classify_ci_rollup(ci_rollup_json, &required_checks);
        let acceptance_unchanged = record
            .acceptance_snapshot
            .as_ref()
            .map(|snapshot| snapshot.matches(&classify_acceptance_criteria(current_issue_body)))
            .unwrap_or(false);
        let review = if review_passed {
            ReviewGateOutcome::Pass
        } else {
            ReviewGateOutcome::Fail("independent review rejected".to_string())
        };
        Some(crate::issue_monitor_gate::AutonomousGateInputs {
            branch_protection,
            ci,
            review,
            acceptance_unchanged,
            reviewed_sha,
            head_sha: current_head_sha.to_string(),
        })
    }

    pub fn inbox_item(&self, issue_number: u64) -> Option<&IssueMonitorInboxItem> {
        self.inbox
            .iter()
            .find(|item| item.issue.number == issue_number)
    }

    pub fn record_claimed(&mut self, issue: IssueMonitorIssue, claim_id: impl Into<String>) {
        let issue_number = issue.number;
        let error_message = self.failed_issues.get(&issue_number).cloned();
        let launched_window_id = if error_message.is_some() {
            None
        } else {
            self.launched_windows.get(&issue_number).cloned()
        };
        let state = if error_message.is_some() {
            MonitorInboxState::AgentFailed
        } else if launched_window_id.is_some() {
            MonitorInboxState::Launched
        } else if self.active_launches.contains(&issue_number) {
            MonitorInboxState::Launching
        } else {
            MonitorInboxState::Queued
        };
        let item = IssueMonitorInboxItem {
            launch_plan: Some(issue_monitor_launch_plan(&issue)),
            issue,
            state,
            claim_id: Some(claim_id.into()),
            blocked_by_owner: None,
            claim_expires_at: None,
            launched_window_id,
            error_message,
            exclusion_reason: None,
        };
        self.upsert_inbox(item);
        if state == MonitorInboxState::Queued
            && !self.queue.contains(&issue_number)
            && !self.active_launches.contains(&issue_number)
        {
            self.queue.push_back(issue_number);
        }
        self.apply_priority_order_to_inbox();
    }

    pub fn record_candidate(&mut self, issue: IssueMonitorIssue) {
        let issue_number = issue.number;
        let existing = self.inbox_item(issue_number).cloned();
        let exclusion = issue_monitor_candidate_exclusion(&issue);
        let error_message = self.failed_issues.get(&issue_number).cloned().or_else(|| {
            existing.as_ref().and_then(|item| {
                if matches!(
                    item.state,
                    MonitorInboxState::AgentFailed | MonitorInboxState::LaunchFailed
                ) {
                    item.error_message.clone()
                } else {
                    None
                }
            })
        });
        let merged = self.merged_issues.contains(&issue_number);
        let launched_window_id = if error_message.is_some() || merged {
            None
        } else {
            self.launched_windows.get(&issue_number).cloned()
        };
        let state = if merged {
            // A validated Issue-wide completion stays terminal. Complete Live
            // reconciliation removes stale ordinary-Open projections before
            // candidate recording reaches this branch.
            MonitorInboxState::Merged
        } else if error_message.is_some() {
            existing
                .as_ref()
                .filter(|item| {
                    matches!(
                        item.state,
                        MonitorInboxState::LaunchFailed | MonitorInboxState::NeedsHuman
                    )
                })
                .map(|item| item.state)
                .or_else(|| {
                    self.autonomous_records
                        .get(&issue_number)
                        .is_some_and(|record| record.phase == AutonomousPhase::NeedsHuman)
                        .then_some(MonitorInboxState::NeedsHuman)
                })
                .unwrap_or(MonitorInboxState::AgentFailed)
        } else if launched_window_id.is_some() {
            MonitorInboxState::Launched
        } else if self.active_launches.contains(&issue_number) {
            // Issue #3222: a claimed launch whose window is not bound yet stays
            // visibly in-flight; the queue-push guard below then skips it.
            MonitorInboxState::Launching
        } else if existing
            .as_ref()
            .is_some_and(|item| item.state == MonitorInboxState::NeedsHuman)
        {
            MonitorInboxState::NeedsHuman
        } else if let Some((state, _)) = exclusion.as_ref() {
            *state
        } else {
            match existing.as_ref().map(|item| item.state) {
                // A reopened Issue previously marked Released/Merged (but no
                // longer tracked as merged) returns to the queue.
                Some(MonitorInboxState::Released)
                | Some(MonitorInboxState::Merged)
                | Some(MonitorInboxState::NotReady)
                | Some(MonitorInboxState::HoldExcluded)
                | None => MonitorInboxState::Queued,
                Some(other) => other,
            }
        };
        let exclusion_reason = exclusion.as_ref().and_then(|(excluded_state, reason)| {
            (*excluded_state == state).then(|| reason.clone())
        });
        let item = IssueMonitorInboxItem {
            launch_plan: Some(issue_monitor_launch_plan(&issue)),
            issue,
            state,
            claim_id: existing.as_ref().and_then(|item| item.claim_id.clone()),
            blocked_by_owner: existing
                .as_ref()
                .and_then(|item| item.blocked_by_owner.clone()),
            claim_expires_at: existing
                .as_ref()
                .and_then(|item| item.claim_expires_at.clone()),
            launched_window_id: launched_window_id.or_else(|| {
                existing
                    .as_ref()
                    .and_then(|item| item.launched_window_id.clone())
            }),
            error_message,
            exclusion_reason,
        };
        if matches!(
            state,
            MonitorInboxState::NotReady | MonitorInboxState::HoldExcluded
        ) {
            revoke_uncommitted_claims_for_issue(
                &mut self.pending_effects,
                self.effect_authority_epoch,
                issue_number,
            );
        }
        self.upsert_inbox(item);
        if state != MonitorInboxState::Queued {
            self.queue.retain(|queued| *queued != issue_number);
        }
        if state == MonitorInboxState::Queued
            && !self.queue.contains(&issue_number)
            && !self.active_launches.contains(&issue_number)
        {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
        self.apply_priority_order_to_inbox();
    }

    /// Issue #3683 (AC-1/AC-2): a `BlockedByClaim` hold is only as durable as
    /// the claim behind it. The scan path (`record_candidate`) deliberately
    /// preserves the state between cycles, so without this sweep an issue
    /// whose blocking claim lapsed — including every claim written before the
    /// identity migration changed the owner format — stayed out of the queue
    /// forever. Runs at the entry of both claim-proposal paths: releasing on
    /// the recorded expiry is safe because the next acquire re-validates
    /// against the live GitHub claims and re-records the block while a
    /// foreign claim is genuinely active.
    pub fn requeue_expired_claim_blocks(&mut self, now: &str) -> Vec<u64> {
        let expired = self
            .inbox
            .iter()
            .filter(|item| {
                item.state == MonitorInboxState::BlockedByClaim
                    && item
                        .claim_expires_at
                        .as_deref()
                        // A block without a recorded expiry cannot outlive the
                        // claim TTL either; fail open toward the queue and let
                        // the acquire path re-verify.
                        .is_none_or(|expires_at| expires_at <= now)
            })
            .map(|item| item.issue.number)
            .collect::<Vec<_>>();
        for issue_number in &expired {
            if let Some(item) = self
                .inbox
                .iter_mut()
                .find(|item| item.issue.number == *issue_number)
            {
                item.state = MonitorInboxState::Queued;
                item.blocked_by_owner = None;
                item.claim_expires_at = None;
            }
            if !self.queue.contains(issue_number) && !self.active_launches.contains(issue_number) {
                self.queue.push_back(*issue_number);
            }
        }
        if !expired.is_empty() {
            self.apply_priority_order_to_queue();
            self.apply_priority_order_to_inbox();
        }
        expired
    }

    pub fn record_blocked_by_claim(
        &mut self,
        issue: IssueMonitorIssue,
        owner: impl Into<String>,
        expires_at: impl Into<String>,
    ) -> bool {
        self.queue.retain(|queued| *queued != issue.number);
        if !self
            .inbox_item(issue.number)
            .is_some_and(|item| item.state == MonitorInboxState::Queued)
        {
            return false;
        }
        self.upsert_inbox(IssueMonitorInboxItem {
            launch_plan: Some(issue_monitor_launch_plan(&issue)),
            issue,
            state: MonitorInboxState::BlockedByClaim,
            claim_id: None,
            blocked_by_owner: Some(owner.into()),
            claim_expires_at: Some(expires_at.into()),
            launched_window_id: None,
            error_message: None,
            exclusion_reason: None,
        });
        self.apply_priority_order_to_inbox();
        true
    }

    pub fn reorder_queued_issues(&mut self, issue_numbers: &[u64]) {
        self.priority_order = issue_numbers.to_vec();
        self.apply_priority_order_to_queue();
        self.apply_priority_order_to_inbox();
    }

    pub fn set_priority_order(&mut self, issue_numbers: Vec<u64>) {
        self.priority_order = issue_numbers;
        self.apply_priority_order_to_queue();
        self.apply_priority_order_to_inbox();
    }

    fn apply_priority_order_to_queue(&mut self) {
        let mut remaining: Vec<u64> = self.queue.iter().copied().collect();
        let mut reordered = VecDeque::new();
        for number in &self.priority_order {
            if self.active_launches.contains(number) {
                continue;
            }
            if let Some(index) = remaining.iter().position(|queued| queued == number) {
                reordered.push_back(*number);
                remaining.remove(index);
            }
        }
        for number in remaining {
            reordered.push_back(number);
        }
        self.queue = reordered;
    }

    fn apply_priority_order_to_inbox(&mut self) {
        if self.priority_order.is_empty() || self.inbox.len() < 2 {
            return;
        }
        let order = self.priority_order.clone();
        self.inbox.sort_by(|left, right| {
            let left_index = order.iter().position(|number| *number == left.issue.number);
            let right_index = order
                .iter()
                .position(|number| *number == right.issue.number);
            match (left_index, right_index) {
                (Some(left_index), Some(right_index)) => left_index.cmp(&right_index),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    }

    fn require_fresh_launch_session(&mut self, issue_number: u64) {
        self.queued_launch_session_strategies.insert(
            issue_number,
            IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    }

    fn take_launch_session_strategy(
        &mut self,
        issue_number: u64,
    ) -> IssueMonitorLaunchSessionStrategy {
        self.queued_launch_session_strategies
            .remove(&issue_number)
            .unwrap_or_default()
    }

    /// Restore one-shot launch policy produced by a scan clone after the
    /// daemon rebases that clone on the latest disk-owned preferences.
    ///
    /// The rebase intentionally replaces the persisted policy map wholesale.
    /// A transient autonomous failure discovered by the scan is newer than
    /// that snapshot, however, so its `FreshRequired` decision must survive.
    /// Newer terminal/disabled disk state remains authoritative and prevents a
    /// stale scan from reviving the marker.
    #[cfg(unix)]
    pub(crate) fn restore_scanned_launch_session_strategies(
        &mut self,
        proposed: &BTreeMap<u64, IssueMonitorLaunchSessionStrategy>,
    ) {
        if !self.config.enabled || !self.autonomous_mode {
            return;
        }
        for (&issue_number, &strategy) in proposed {
            if strategy != IssueMonitorLaunchSessionStrategy::FreshRequired
                || self.merged_issues.contains(&issue_number)
                || self.failed_issues.contains_key(&issue_number)
                || self
                    .inbox_item(issue_number)
                    .is_some_and(|item| item.state.is_terminal())
                || !self.queue.contains(&issue_number)
            {
                continue;
            }
            self.require_fresh_launch_session(issue_number);
        }
    }

    pub fn next_launch_request(&mut self, now: &str) -> Option<IssueMonitorLaunchRequest> {
        let max_active = self.config.max_active.max(1);
        if !self.gui_connected
            || self.active_launches.len() >= max_active
            || self.saved_profile_provider_is_held_at(now)
        {
            return None;
        }
        let issue_number = self.queue.pop_front()?;
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
        // #3223 follow-up: anchor the claim so a restored-but-never-acked
        // launch can expire after claim_ttl_secs instead of leaking the slot.
        self.launching_claimed_at
            .insert(issue_number, now.to_string());
        let linked_issue_kind = if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Launching;
            issue_monitor_linked_issue_kind(&item.issue)
        } else {
            LinkedIssueKind::Issue
        };
        Some(IssueMonitorLaunchRequest {
            issue_number,
            branch_name: knowledge_launch_target_branch_name(linked_issue_kind, issue_number),
            linked_issue_kind,
            delivery_id: None,
            launch_session_strategy: self.take_launch_session_strategy(issue_number),
        })
    }

    pub fn claim_next_launch_requests<C: IssueClient>(
        &mut self,
        client: &C,
        owner: &str,
        now: &str,
    ) -> Vec<IssueMonitorLaunchRequest> {
        self.claim_next_launch_requests_with_active_cap(
            client,
            owner,
            now,
            self.config.max_active.max(1),
        )
    }

    /// Select durable claim proposals without calling GitHub. The returned
    /// count is the number of newly prepared logical claims; active slots are
    /// consumed only after the serialized executor confirms acquisition.
    pub fn prepare_claim_effects_with_probe(
        &mut self,
        owner: &str,
        now: &str,
        active_cap: usize,
        completed_probe: impl Fn(u64) -> bool,
    ) -> usize {
        match self.try_prepare_claim_effects_with_probe(owner, now, active_cap, |issue_number| {
            Ok::<bool, std::convert::Infallible>(completed_probe(issue_number))
        }) {
            Ok(prepared) => prepared,
            Err(never) => match never {},
        }
    }

    /// The claim slots still open, paired with the ordered queue entries a
    /// proposal may consult to fill them.
    ///
    /// Issue #3528: a completion probe costs one `gh` process per issue, so a
    /// scan that probes every open issue burns its whole deadline before it can
    /// commit anything. The planner below only ever walks this list, so a scan
    /// only ever needs to probe this list.
    pub fn claim_probe_plan(&self, active_cap: usize) -> (usize, Vec<u64>) {
        let max_active = self.config.max_active.max(1).min(active_cap);
        if !self.config.enabled || max_active == 0 {
            return (0, Vec::new());
        }
        let pending_claims = self
            .pending_effects
            .iter()
            .filter_map(|effect| match effect.payload {
                IssueMonitorEffectPayload::AcquireClaim { issue_number, .. } => Some(issue_number),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let available = max_active
            .saturating_sub(self.active_launches.len())
            .saturating_sub(pending_claims.len());
        if available == 0 {
            return (0, Vec::new());
        }
        let candidates = self
            .queue
            .iter()
            .copied()
            .filter(|issue_number| !pending_claims.contains(issue_number))
            .collect();
        (available, candidates)
    }

    /// Fallible proposal planner for deadline-integral scan transactions. A
    /// probe error is returned to the scan owner instead of being collapsed to
    /// `false`; callers discard the cloned proposal state on error.
    pub fn try_prepare_claim_effects_with_probe<E>(
        &mut self,
        owner: &str,
        now: &str,
        active_cap: usize,
        mut completed_probe: impl FnMut(u64) -> Result<bool, E>,
    ) -> Result<usize, E> {
        // Issue #3683: expired claim blocks re-enter the queue before slots
        // are planned, so a lapsed claim can never starve its issue.
        self.requeue_expired_claim_blocks(now);
        if !self.gui_connected || self.saved_profile_provider_is_held_at(now) {
            return Ok(0);
        }
        let (mut available, candidates) = self.claim_probe_plan(active_cap);
        if available == 0 {
            return Ok(0);
        }

        let mut prepared = 0;
        for issue_number in candidates {
            if available == 0 {
                break;
            }
            if !self.retry_ready_for_saved_profile(issue_number, now) {
                continue;
            }
            let Some(issue) = self.inbox_item(issue_number).map(|item| item.issue.clone()) else {
                continue;
            };
            if completed_probe(issue_number)? && self.record_linked_pr_completion(issue_number) {
                continue;
            }
            let kind = issue_monitor_linked_issue_kind(&issue);
            let launched_work_id = knowledge_launch_target_branch_name(kind, issue_number);
            let proposal_id = uuid::Uuid::new_v4();
            let effect_id = format!("claim:{issue_number}:{proposal_id}");
            let claim_id = format!("gwt-auto-improve:{proposal_id}");
            if self
                .prepare_pending_effect(
                    effect_id,
                    IssueMonitorEffectPayload::AcquireClaim {
                        issue_number,
                        claim_id,
                        owner: owner.to_string(),
                        heartbeat_at: now.to_string(),
                        expires_at: expiry_from_now_lexical(now, self.config.claim_ttl_secs),
                        launched_work_id: Some(launched_work_id),
                    },
                )
                .is_some()
            {
                prepared += 1;
                available -= 1;
            }
        }
        Ok(prepared)
    }

    pub fn claim_next_launch_requests_with_active_cap<C: IssueClient>(
        &mut self,
        client: &C,
        owner: &str,
        now: &str,
        active_cap: usize,
    ) -> Vec<IssueMonitorLaunchRequest> {
        self.claim_next_launch_requests_with_probe(client, owner, now, active_cap, |_| false)
    }

    /// Issue #3225: claim queued candidates, skipping issues with fresh,
    /// Issue-wide completion evidence from GitHub. Instance-local
    /// `merged_issues` memory is not enough because a fresh monitor would
    /// otherwise re-launch unchanged delivered work that stays open until
    /// release. The shared domain policy requeues incomplete SPEC work even if
    /// a caller reports a positive probe. Probe errors/false stay launchable.
    pub fn claim_next_launch_requests_with_probe<C: IssueClient>(
        &mut self,
        client: &C,
        owner: &str,
        now: &str,
        active_cap: usize,
        completed_probe: impl Fn(u64) -> bool,
    ) -> Vec<IssueMonitorLaunchRequest> {
        // Issue #3683: expired claim blocks re-enter the queue before slots
        // are consumed, so a lapsed claim can never starve its issue.
        self.requeue_expired_claim_blocks(now);
        let mut launches = Vec::new();
        let max_active = self.config.max_active.max(1).min(active_cap);
        if max_active == 0 || self.saved_profile_provider_is_held_at(now) {
            return launches;
        }
        while self.config.enabled && self.gui_connected && self.active_launches.len() < max_active {
            let Some(issue_number) = self
                .queue
                .iter()
                .copied()
                .find(|issue_number| self.retry_ready_for_saved_profile(*issue_number, now))
            else {
                break;
            };
            let Some(issue) = self.inbox_item(issue_number).map(|item| item.issue.clone()) else {
                self.queue.retain(|queued| *queued != issue_number);
                continue;
            };
            if completed_probe(issue.number) && self.record_linked_pr_completion(issue.number) {
                tracing::info!(
                    issue = issue.number,
                    "issue monitor: completion probe found current-generation evidence"
                );
                continue;
            }
            let kind = issue_monitor_linked_issue_kind(&issue);
            let branch_name = knowledge_launch_target_branch_name(kind, issue.number);
            let claim = ClaimComment {
                comment_id: None,
                claim_id: format!("gwt-auto-improve:{owner}:{}:{now}", issue.number),
                owner: owner.to_string(),
                issue_number: issue.number,
                status: ClaimStatus::Active,
                heartbeat_at: now.to_string(),
                expires_at: expiry_from_now_lexical(now, self.config.claim_ttl_secs),
                launched_work_id: Some(branch_name),
            };

            match acquire_claim(client, IssueNumber(issue.number), claim, now) {
                Ok(ClaimAcquireOutcome::Acquired(claim)) => {
                    let claim_id = claim.claim_id;
                    let synchronous_effect_id = format!("synchronous-claim:{claim_id}");
                    let delivery_id = format!("launch:{synchronous_effect_id}");
                    if self.apply_confirmed_claim(
                        issue.number,
                        claim_id,
                        owner,
                        &synchronous_effect_id,
                        now,
                    ) {
                        if let Some(request) = self
                            .pending_launch_deliveries
                            .iter()
                            .find(|delivery| {
                                delivery.issue_number == issue.number
                                    && delivery.delivery_id == delivery_id
                            })
                            .map(|delivery| IssueMonitorLaunchRequest {
                                issue_number: delivery.issue_number,
                                branch_name: delivery.branch_name.clone(),
                                linked_issue_kind: delivery.linked_issue_kind,
                                delivery_id: Some(delivery.delivery_id.clone()),
                                launch_session_strategy: delivery.launch_session_strategy,
                            })
                        {
                            launches.push(request);
                        }
                    }
                }
                Ok(ClaimAcquireOutcome::Blocked(claim)) => {
                    self.record_blocked_by_claim(issue, claim.owner, claim.expires_at);
                }
                Ok(ClaimAcquireOutcome::Lost { winning_claim, .. }) => {
                    self.record_blocked_by_claim(
                        issue,
                        winning_claim.owner,
                        winning_claim.expires_at,
                    );
                }
                Err(error) => {
                    self.last_error = Some(format!("issue #{}: {error}", issue.number));
                    break;
                }
            }
        }
        launches
    }

    pub fn take_pending_launch_requests(&mut self) -> Vec<IssueMonitorLaunchRequest> {
        let durable_issue_numbers = self
            .pending_launch_deliveries
            .iter()
            .map(|delivery| delivery.issue_number)
            .collect::<BTreeSet<_>>();
        let mut requests = self
            .pending_launches
            .drain(..)
            .filter(|request| !durable_issue_numbers.contains(&request.issue_number))
            .collect::<Vec<_>>();
        requests.extend(self.pending_launch_deliveries.iter().map(|delivery| {
            IssueMonitorLaunchRequest {
                issue_number: delivery.issue_number,
                branch_name: delivery.branch_name.clone(),
                linked_issue_kind: delivery.linked_issue_kind,
                delivery_id: Some(delivery.delivery_id.clone()),
                launch_session_strategy: delivery.launch_session_strategy,
            }
        }));
        requests
    }

    /// SPEC #3200 Option A: queue a review-agent spawn request (orchestration
    /// loop → GUI). Deduped on issue number so repeated ticks don't pile up.
    pub fn push_review_dispatch(&mut self, dispatch: AutonomousReviewDispatch) {
        self.pending_review_dispatches
            .retain(|pending| pending.issue_number != dispatch.issue_number);
        self.pending_review_dispatches.push_back(dispatch);
    }

    /// Drain queued review-agent spawn requests for emission to the GUI.
    pub fn take_pending_review_dispatches(&mut self) -> Vec<AutonomousReviewDispatch> {
        self.pending_review_dispatches.drain(..).collect()
    }

    /// SPEC #3200 FR-034 (T-111): queue an operator notice for an unattended
    /// autonomous transition. Fail-closed: a no-op unless autonomous mode is on,
    /// so the default-OFF human-gated flow (#3165) emits nothing extra. Bounded:
    /// oldest notices are dropped past [`AUTONOMOUS_NOTICE_CAP`] so a
    /// disconnected-GUI window never grows the queue without limit.
    fn push_autonomous_notice(
        &mut self,
        level: &str,
        issue_number: u64,
        message: impl Into<String>,
    ) {
        if !self.autonomous_mode {
            return;
        }
        while self.pending_autonomous_notices.len() >= AUTONOMOUS_NOTICE_CAP {
            self.pending_autonomous_notices.pop_front();
        }
        self.pending_autonomous_notices.push_back(AutonomousNotice {
            level: level.to_string(),
            issue_number,
            message: message.into(),
        });
    }

    /// Drain queued autonomous operator notices for emission as `toast`
    /// payloads. Call only when a GUI is connected so unattended-window notices
    /// are retained until someone can see them.
    pub fn take_autonomous_notices(&mut self) -> Vec<AutonomousNotice> {
        self.pending_autonomous_notices.drain(..).collect()
    }

    /// Queue an operator notice that must surface even though autonomous mode is
    /// already OFF — the kill-switch disarm results. Bypasses the fail-closed
    /// mode gate deliberately: these notices are feedback ABOUT turning the mode
    /// off, so gating them on the mode would silence exactly the events the
    /// operator just asked for.
    fn push_kill_switch_notice(&mut self, level: &str, issue_number: u64, message: String) {
        while self.pending_autonomous_notices.len() >= AUTONOMOUS_NOTICE_CAP {
            self.pending_autonomous_notices.pop_front();
        }
        self.pending_autonomous_notices.push_back(AutonomousNotice {
            level: level.to_string(),
            issue_number,
            message,
        });
    }

    /// SPEC #3200 kill switch (codex #3217/#3219 review): with autonomous mode
    /// OFF, every record still in `Delivering` has an armed GitHub auto-merge
    /// that must be ACTIVELY cancelled (`gh pr merge --disable-auto`), not just
    /// abandoned locally. Returns the `(issue_number, pr_number)` pairs to
    /// disarm WITHOUT mutating any record: a record leaves `Delivering` only
    /// after the disarm actually SUCCEEDS
    /// ([`record_kill_switch_disarm_result`](Self::record_kill_switch_disarm_result)),
    /// so a transient `gh` failure keeps it targeted and the next scan retries.
    /// A failed disarm must never strand a live armed auto-merge behind a
    /// NeedsHuman screen. No-op while the mode is ON.
    pub fn kill_switch_disarm_targets(&self) -> Vec<(u64, u64)> {
        if self.config.enabled && self.autonomous_mode {
            return Vec::new();
        }
        self.autonomous_records
            .values()
            .filter(|record| record.phase == AutonomousPhase::Delivering)
            .filter_map(|record| record.pr_number.map(|pr| (record.issue_number, pr)))
            .collect()
    }

    /// Record the outcome of a kill-switch auto-merge disarm attempt.
    ///
    /// - **Success**: the delivery is halted for good — escalate to `NeedsHuman`
    ///   (visible, never silently resumed) and emit a warn notice.
    /// - **Failure**: emit an error notice but LEAVE the record in `Delivering`
    ///   so [`kill_switch_disarm_targets`](Self::kill_switch_disarm_targets)
    ///   returns it again and the next scan retries the disarm (codex #3219: a
    ///   failed disarm must stay retryable while the remote auto-merge is live).
    ///
    /// Notices are ungated: the mode is OFF by definition here, and these are
    /// the operator's feedback for turning it off.
    pub fn record_kill_switch_disarm_result(
        &mut self,
        issue_number: u64,
        pr_number: u64,
        disarmed: bool,
    ) {
        if self.issue_is_closed(issue_number) {
            return;
        }
        if disarmed {
            self.escalate_to_needs_human(
                issue_number,
                NeedsHumanKind::UserChoiceRequired,
                "autonomous mode disabled — delivery halted; auto-merge disarmed",
            );
            self.push_kill_switch_notice(
                "warn",
                issue_number,
                format!(
                    "Issue #{issue_number}: auto-merge on PR #{pr_number} disarmed (kill switch)"
                ),
            );
        } else {
            self.push_kill_switch_notice(
                "error",
                issue_number,
                format!(
                    "Issue #{issue_number}: failed to disarm auto-merge on PR #{pr_number} — still armed on GitHub; will retry next scan"
                ),
            );
        }
    }

    /// Apply a confirmed durable claim effect and enqueue the corresponding GUI
    /// launch exactly once. Returns false if the scanned issue disappeared
    /// before the executor result committed.
    pub fn apply_confirmed_claim(
        &mut self,
        issue_number: u64,
        claim_id: impl Into<String>,
        claim_owner: impl Into<String>,
        claim_effect_id: &str,
        now: &str,
    ) -> bool {
        let claim_id = claim_id.into();
        let claim_owner = claim_owner.into();
        // An exact ReleaseClaim is the durable compensation written when this
        // claim crossed its remote-execution fence before a requeue/exclusion.
        // Its late success is old-cycle evidence and may never start a launch.
        if self.pending_effects.iter().any(|effect| {
            matches!(
                &effect.payload,
                IssueMonitorEffectPayload::ReleaseClaim {
                    issue_number: pending_issue,
                    claim_id: pending_claim,
                    owner: pending_owner,
                } if *pending_issue == issue_number
                    && pending_claim == &claim_id
                    && pending_owner == &claim_owner
            )
        }) {
            return false;
        }
        let Some(issue) = self
            .inbox_item(issue_number)
            .filter(|item| item.state == MonitorInboxState::Queued)
            .map(|item| item.issue.clone())
        else {
            ensure_claim_release_effect(
                &mut self.pending_effects,
                self.effect_authority_epoch,
                claim_effect_id,
                issue_number,
                &claim_id,
                &claim_owner,
            );
            return false;
        };
        let linked_issue_kind = issue_monitor_linked_issue_kind(&issue);
        let branch_name = knowledge_launch_target_branch_name(linked_issue_kind, issue_number);
        let delivery_id = format!("launch:{claim_effect_id}");
        let launch_session_strategy = self.take_launch_session_strategy(issue_number);
        // Issue #3757 / SPEC #3165 FR-134: the confirmed claim and its durable
        // delivery are the exact transition that starts the fresh lifecycle.
        // Consume the recovery fence here, never by comparing timestamps.
        self.released_failures.remove(&issue_number);
        self.record_claimed(issue, claim_id.clone());
        self.queue.retain(|queued| *queued != issue_number);
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
        self.launching_claimed_at
            .insert(issue_number, now.to_string());
        self.set_inbox_state(issue_number, MonitorInboxState::Launching);
        if !self
            .pending_launch_deliveries
            .iter()
            .any(|delivery| delivery.delivery_id == delivery_id)
        {
            self.pending_launch_deliveries
                .push_back(PendingIssueMonitorLaunchDelivery {
                    delivery_id,
                    issue_number,
                    branch_name,
                    linked_issue_kind,
                    claim_id,
                    claim_owner,
                    materializer_id: None,
                    materializer_pid: None,
                    materializer_window_id: None,
                    materialized_window_id: None,
                    workspace_durable_window_id: None,
                    launch_session_strategy,
                    created_at: now.to_string(),
                });
        }
        true
    }

    pub fn complete_active_launch_delivery(
        &mut self,
        issue_number: u64,
        window_id: impl Into<String>,
        delivery_id: Option<&str>,
    ) -> bool {
        let window_id = window_id.into();
        let mut launched_claim_id = None;
        if let Some(delivery_id) = delivery_id {
            match self.match_pending_launch_delivery(issue_number, delivery_id) {
                PendingLaunchDeliveryMatch::Matched(index) => {
                    let delivery = &self.pending_launch_deliveries[index];
                    if delivery.materialized_window_id.as_deref() != Some(window_id.as_str())
                        || delivery.workspace_durable_window_id.as_deref()
                            != Some(window_id.as_str())
                    {
                        return false;
                    }
                    launched_claim_id = Some(delivery.claim_id.clone());
                    self.pending_launch_deliveries.remove(index);
                }
                PendingLaunchDeliveryMatch::Missing | PendingLaunchDeliveryMatch::Mismatched => {
                    return false
                }
            }
        }
        self.complete_active_launch_with_claim(issue_number, window_id, launched_claim_id);
        true
    }

    pub fn claim_launch_delivery(
        &mut self,
        issue_number: u64,
        delivery_id: &str,
        materializer_id: &str,
        materializer_pid: u32,
        materializer_window_id: &str,
        is_process_alive: impl Fn(u32) -> bool,
    ) -> bool {
        let Some(delivery) = self.pending_launch_deliveries.iter_mut().find(|delivery| {
            delivery.issue_number == issue_number && delivery.delivery_id == delivery_id
        }) else {
            return false;
        };
        if delivery.materializer_id.as_deref() == Some(materializer_id) {
            delivery.materializer_pid = Some(materializer_pid);
            if delivery.materializer_window_id.as_deref() != Some(materializer_window_id) {
                delivery.materialized_window_id = None;
                delivery.workspace_durable_window_id = None;
            }
            delivery.materializer_window_id = Some(materializer_window_id.to_string());
            return true;
        }
        if delivery
            .materializer_pid
            .filter(|pid| *pid > 0)
            .is_some_and(is_process_alive)
        {
            return false;
        }
        delivery.materializer_id = Some(materializer_id.to_string());
        delivery.materializer_pid = Some(materializer_pid);
        delivery.materializer_window_id = Some(materializer_window_id.to_string());
        delivery.materialized_window_id = None;
        delivery.workspace_durable_window_id = None;
        true
    }

    pub fn mark_launch_delivery_materialized(
        &mut self,
        issue_number: u64,
        delivery_id: &str,
        materializer_id: &str,
        materializer_window_id: &str,
    ) -> bool {
        let Some(delivery) = self.pending_launch_deliveries.iter_mut().find(|delivery| {
            delivery.issue_number == issue_number && delivery.delivery_id == delivery_id
        }) else {
            return false;
        };
        if delivery.materializer_window_id.as_deref() != Some(materializer_window_id) {
            return false;
        }
        if delivery.materializer_id.as_deref() != Some(materializer_id) {
            return false;
        }
        if delivery.materialized_window_id.as_deref() == Some(materializer_window_id) {
            return true;
        }
        delivery.materialized_window_id = Some(materializer_window_id.to_string());
        true
    }

    pub fn mark_launch_delivery_workspace_durable(
        &mut self,
        issue_number: u64,
        delivery_id: &str,
        materializer_id: &str,
        materializer_window_id: &str,
    ) -> bool {
        let Some(delivery) = self.pending_launch_deliveries.iter_mut().find(|delivery| {
            delivery.issue_number == issue_number && delivery.delivery_id == delivery_id
        }) else {
            return false;
        };
        if delivery.materializer_id.as_deref() != Some(materializer_id)
            || delivery.materializer_window_id.as_deref() != Some(materializer_window_id)
            || delivery.materialized_window_id.as_deref() != Some(materializer_window_id)
        {
            return false;
        }
        delivery.workspace_durable_window_id = Some(materializer_window_id.to_string());
        true
    }

    pub fn complete_active_launch(&mut self, issue_number: u64, window_id: impl Into<String>) {
        self.complete_active_launch_with_claim(issue_number, window_id.into(), None);
    }

    fn complete_active_launch_with_claim(
        &mut self,
        issue_number: u64,
        window_id: String,
        claim_id: Option<String>,
    ) {
        self.launching_claimed_at.remove(&issue_number);
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
        self.launched_windows
            .insert(issue_number, window_id.clone());
        match claim_id {
            Some(claim_id) => {
                self.launched_claims.insert(issue_number, claim_id);
            }
            None => {
                self.launched_claims.remove(&issue_number);
            }
        }
        if let Some(branch) = self
            .inbox_item(issue_number)
            .and_then(|item| item.launch_plan.as_ref())
            .map(|plan| plan.branch_name.clone())
        {
            self.launched_branches.insert(issue_number, branch);
        }
        // A fresh launch supersedes any prior Merged completion (e.g. manual
        // Launch Now of already-merged work).
        if self
            .completion_records
            .get(&issue_number)
            .is_some_and(|record| record.state == IssueCompletionState::Completed)
        {
            let updated_at = self
                .inbox_item(issue_number)
                .and_then(|item| item.issue.updated_at.clone());
            self.transition_completion(
                issue_number,
                IssueCompletionState::Reopened,
                updated_at,
                IssueCompletionEvidence::WorkBranch,
            );
        }
        self.merged_issues.remove(&issue_number);
        self.failed_issues.remove(&issue_number);
        self.failed_windows.remove(&issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.pending_launches
            .retain(|pending| pending.issue_number != issue_number);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Launched;
            item.launched_window_id = Some(window_id);
            item.error_message = None;
        }
        // SPEC-3431 FR-068: start the activity clock here rather than only on
        // the autonomous path. Every launch needs it — an agent that stalls
        // without autonomous mode stalls just as silently — and this is the
        // one place every launch is confirmed.
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.record_autonomous_heartbeat(issue_number, &now);
    }

    pub fn record_launch_failed(&mut self, issue_number: u64, message: impl Into<String>) {
        self.record_failed_issue(issue_number, message, MonitorInboxState::LaunchFailed);
    }

    pub fn record_launch_failed_delivery(
        &mut self,
        issue_number: u64,
        message: impl Into<String>,
        delivery_id: Option<&str>,
        materializer_id: Option<&str>,
    ) -> bool {
        if delivery_id.is_none()
            && self
                .pending_launch_deliveries
                .iter()
                .any(|delivery| delivery.issue_number == issue_number)
        {
            return false;
        }
        if let Some(delivery_id) = delivery_id {
            match self.match_pending_launch_delivery(issue_number, delivery_id) {
                PendingLaunchDeliveryMatch::Matched(index) => {
                    if materializer_id.is_none()
                        || self.pending_launch_deliveries[index]
                            .materializer_id
                            .as_deref()
                            != materializer_id
                    {
                        return false;
                    }
                    self.pending_launch_deliveries.remove(index);
                }
                PendingLaunchDeliveryMatch::Missing | PendingLaunchDeliveryMatch::Mismatched => {
                    return false
                }
            }
        }
        self.record_launch_failed(issue_number, message);
        true
    }

    pub fn record_agent_window_failed(
        &mut self,
        window_id: &str,
        message: impl Into<String>,
    ) -> Option<u64> {
        let issue_number = self
            .launched_windows
            .iter()
            .find_map(|(issue_number, launched_window_id)| {
                issue_monitor_window_ids_match(launched_window_id, window_id)
                    .then_some(*issue_number)
            })
            .or_else(|| {
                self.inbox.iter().find_map(|item| {
                    item.launched_window_id
                        .as_deref()
                        .filter(|launched_window_id| {
                            issue_monitor_window_ids_match(launched_window_id, window_id)
                        })
                        .map(|_| item.issue.number)
                })
            })?;
        self.record_failed_issue(issue_number, message, MonitorInboxState::AgentFailed);
        Some(issue_number)
    }

    pub fn record_agent_issue_failed(&mut self, issue_number: u64, message: impl Into<String>) {
        self.record_failed_issue(issue_number, message, MonitorInboxState::AgentFailed);
    }

    /// Issue #3627: launched agent windows that the owning project tab no
    /// longer has on its canvas.
    ///
    /// Slot release used to be driven exclusively by PTY exit events, so a
    /// launch whose window disappeared without one — an app restart, a crash,
    /// an error pane reaped outside `close_window_events` — held its
    /// `max_active` slot forever. With the default caps that stops the whole
    /// queue, and `with_prefs` faithfully restored the dead entry on every
    /// reload, so the wedge survived restarts.
    ///
    /// The judgement source here is window existence on the owning tab's
    /// canvas, which is the same store that issues the ids. No pid is
    /// consulted, so pid reuse cannot mistake a dead launch for a live one (or
    /// the reverse). Equally important, a window that still exists keeps its
    /// slot no matter how long it has been silent: an unexplained stall is
    /// reported, not reclaimed (SPEC-3431 FR-069).
    ///
    /// Only windows qualified with `project_tab_id` are judged. A window
    /// belonging to another tab — including another gwt instance's tab — is
    /// invisible from here, and a legacy bare id has no trustworthy
    /// provenance, so both are retained exactly as
    /// `prune_inflight_launches_for_live_snapshot` retains them.
    pub fn vanished_launched_windows(
        &self,
        project_tab_id: &str,
        live_window_ids: &BTreeSet<String>,
    ) -> Vec<String> {
        self.launched_windows
            .values()
            .filter(|window_id| {
                issue_monitor_qualified_window_id(window_id)
                    .is_some_and(|(tab_id, _)| tab_id == project_tab_id)
            })
            .filter(|window_id| {
                !live_window_ids
                    .iter()
                    .any(|live| issue_monitor_window_ids_match(window_id, live))
            })
            .cloned()
            .collect()
    }

    /// Reverse-lookup the Issue associated with a launched agent `window_id`.
    pub fn launched_window_issue(&self, window_id: &str) -> Option<u64> {
        self.launched_windows
            .iter()
            .find_map(|(issue_number, launched_window_id)| {
                issue_monitor_window_ids_match(launched_window_id, window_id)
                    .then_some(*issue_number)
            })
            .or_else(|| {
                self.inbox.iter().find_map(|item| {
                    item.launched_window_id
                        .as_deref()
                        .filter(|launched_window_id| {
                            issue_monitor_window_ids_match(launched_window_id, window_id)
                        })
                        .map(|_| item.issue.number)
                })
            })
    }

    fn clear_active_tracking(&mut self, issue_number: u64) {
        self.active_launches
            .retain(|active| *active != issue_number);
        self.launching_claimed_at.remove(&issue_number);
        self.launched_windows.remove(&issue_number);
        self.launched_claims.remove(&issue_number);
        self.launched_branches.remove(&issue_number);
        // #3165 error-window lifecycle: terminal transitions (merged / released /
        // needs-human) and retry all funnel through here; drop any retained stale
        // failed-window id so it never orphans (and never persists into prefs)
        // when the issue ends without an explicit Launch Now relaunch.
        self.failed_windows.remove(&issue_number);
        self.pending_launches
            .retain(|pending| pending.issue_number != issue_number);
        self.pending_launch_deliveries
            .retain(|pending| pending.issue_number != issue_number);
        self.queued_launch_session_strategies.remove(&issue_number);
        self.pending_review_dispatches
            .retain(|pending| pending.issue_number != issue_number);
        // Issue #3844: the wait declaration is scoped to the launch that made
        // it; every path that ends the launch funnels through here.
        if let Some(record) = self.autonomous_records.get_mut(&issue_number) {
            record.wait = None;
        }
    }

    /// Reconcile persisted in-flight accounting against one complete live
    /// repository snapshot. A qualified window owned by another project tab is
    /// also stale for this GUI observer. Bare legacy window ids have no
    /// trustworthy project provenance and are therefore retained.
    fn prune_inflight_launches_for_live_snapshot(
        &mut self,
        live_issue_numbers: &BTreeSet<u64>,
        expected_project_tab_id: Option<&str>,
    ) {
        let absent_issues = self
            .active_launches
            .iter()
            .copied()
            .filter(|issue_number| !live_issue_numbers.contains(issue_number))
            .collect::<BTreeSet<_>>();
        let foreign_window_issues = expected_project_tab_id
            .map(|expected_tab_id| {
                self.launched_windows
                    .iter()
                    .filter_map(|(issue_number, window_id)| {
                        issue_monitor_qualified_window_id(window_id)
                            .filter(|(tab_id, _)| *tab_id != expected_tab_id)
                            .map(|_| *issue_number)
                    })
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let targets = absent_issues
            .union(&foreign_window_issues)
            .copied()
            .collect::<Vec<_>>();

        for issue_number in targets {
            self.clear_active_tracking(issue_number);
            if absent_issues.contains(&issue_number) {
                self.queue.retain(|queued| *queued != issue_number);
                self.inbox.retain(|item| item.issue.number != issue_number);
            } else {
                self.set_inbox_state(issue_number, MonitorInboxState::Queued);
            }
        }
    }

    fn set_inbox_state(&mut self, issue_number: u64, state: MonitorInboxState) {
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = state;
            item.launched_window_id = None;
            item.error_message = None;
            item.exclusion_reason = None;
        }
    }

    fn match_pending_launch_delivery(
        &self,
        issue_number: u64,
        delivery_id: &str,
    ) -> PendingLaunchDeliveryMatch {
        if let Some(index) = self.pending_launch_deliveries.iter().position(|delivery| {
            delivery.issue_number == issue_number && delivery.delivery_id == delivery_id
        }) {
            return PendingLaunchDeliveryMatch::Matched(index);
        }
        if self
            .pending_launch_deliveries
            .iter()
            .any(|delivery| delivery.issue_number == issue_number)
        {
            PendingLaunchDeliveryMatch::Mismatched
        } else {
            PendingLaunchDeliveryMatch::Missing
        }
    }

    /// Record one merged work delivery. Every merge frees its active slot, but
    /// only evidence that completes the current Issue generation creates the
    /// terminal `Merged` state. Incomplete work is re-queued, while unknown
    /// completion evidence fails closed without minting a terminal marker.
    pub fn record_merged(&mut self, issue_number: u64) {
        self.record_issue_completion(issue_number, IssueCompletionEvidence::WorkBranch);
    }

    fn record_linked_pr_completion(&mut self, issue_number: u64) -> bool {
        self.record_issue_completion(issue_number, IssueCompletionEvidence::LinkedPr)
    }

    fn record_issue_completion(
        &mut self,
        issue_number: u64,
        evidence: IssueCompletionEvidence,
    ) -> bool {
        let issue = self.inbox_item(issue_number).map(|item| item.issue.clone());
        // A merged branch/PR completes this delivery regardless of whether it
        // completes the Issue. Free the slot before evaluating Issue-wide
        // completion so ordinary Open work can return to the queue promptly.
        self.clear_active_tracking(issue_number);
        if issue
            .as_ref()
            .map(Self::issue_completion_decision)
            .unwrap_or(IssueCompletionDecision::Unknown)
            != IssueCompletionDecision::Complete
        {
            let updated_at = issue.as_ref().and_then(|issue| issue.updated_at.clone());
            self.transition_completion(
                issue_number,
                IssueCompletionState::Reopened,
                updated_at,
                evidence,
            );
            // The merged delivery finished this autonomous attempt even when
            // the ordinary Issue is still Open or the SPEC has remaining
            // tasks. Leaving `Delivering` here would replay the same PR forever
            // and prevent the queued successor attempt from starting.
            self.clear_autonomous_record(issue_number);
            if let Some(issue) = issue {
                self.set_inbox_state(issue_number, MonitorInboxState::Queued);
                self.record_candidate(issue);
            }
            self.require_fresh_launch_session(issue_number);
            return false;
        }
        // FR-034: notify the operator when an issue that went through the
        // autonomous loop completes (checked BEFORE the record is cleared).
        if self.autonomous_records.contains_key(&issue_number) {
            self.push_autonomous_notice(
                "done",
                issue_number,
                format!("Issue #{issue_number} merged autonomously"),
            );
        }
        self.transition_completion(
            issue_number,
            IssueCompletionState::Completed,
            issue.and_then(|issue| issue.updated_at),
            evidence,
        );
        self.apply_merged_terminal_state(issue_number);
        true
    }

    /// Apply persisted merge completion without emitting another autonomous
    /// notice. Cross-process rebase is state convergence, not a new event.
    fn apply_merged_terminal_state(&mut self, issue_number: u64) {
        let removed_failure_banner = self
            .failed_issues
            .get(&issue_number)
            .map(|message| format!("issue #{issue_number}: {message}"));
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.failed_issues.remove(&issue_number);
        self.released_failures.remove(&issue_number);
        self.merged_issues.insert(issue_number);
        self.set_inbox_state(issue_number, MonitorInboxState::Merged);
        // SPEC #3200 T-022: completion resets the autonomous lifecycle (attempts,
        // phase, snapshot, in-flight launch id) so a future reopen starts clean.
        self.clear_autonomous_record(issue_number);
        if removed_failure_banner
            .as_ref()
            .is_some_and(|banner| self.last_error.as_ref() == Some(banner))
        {
            self.last_error = self.first_failed_issue_banner();
        }
    }

    /// Record that the GitHub Issue for `issue_number` was closed (released).
    pub fn record_released(&mut self, issue_number: u64) {
        let current = self.closure_records.get(&issue_number);
        let generation = current
            .map(|record| record.generation.saturating_add(1))
            .unwrap_or(1);
        let (evidence, issue_updated_at) = current
            .map(|record| {
                (
                    if record.state == IssueClosureState::Closed {
                        record.evidence
                    } else {
                        IssueClosureEvidence::DirectRelease
                    },
                    record.issue_updated_at.clone(),
                )
            })
            .unwrap_or((IssueClosureEvidence::DirectRelease, None));
        self.closure_records.insert(
            issue_number,
            IssueClosureRecord {
                issue_number,
                generation,
                state: IssueClosureState::Closed,
                evidence,
                issue_updated_at,
            },
        );
        self.closure_reopen_tombstones.remove(&issue_number);
        self.clear_closed_issue_current_state(issue_number);
    }

    /// issue → work branch for every currently active (launched) Issue. Uses
    /// the stored launch branch, falling back to the inbox launch plan.
    pub fn active_launched_branches(&self) -> Vec<(u64, String)> {
        self.active_launches
            .iter()
            .filter_map(|number| {
                let branch = self.launched_branches.get(number).cloned().or_else(|| {
                    self.inbox_item(*number)
                        .and_then(|item| item.launch_plan.as_ref())
                        .map(|plan| plan.branch_name.clone())
                })?;
                Some((*number, branch))
            })
            .collect()
    }

    /// Reconcile active work branches that merged. Every match frees its slot;
    /// the shared completion policy decides whether the Issue is terminal or
    /// must return to the queue for remaining tasks.
    pub fn reconcile_merged_branches(&mut self, merged_branches: &BTreeSet<String>) -> Vec<u64> {
        let to_merge: Vec<u64> = self
            .active_launched_branches()
            .into_iter()
            .filter(|(_, branch)| merged_branches.contains(branch))
            .map(|(number, _)| number)
            .collect();
        for number in &to_merge {
            self.record_merged(*number);
        }
        to_merge
    }

    /// Issue #3645 AC-3/AC-4: issues whose completion the launch tracking
    /// cannot see, nominated from the merged-branch list alone.
    ///
    /// [`Self::reconcile_merged_branches`] resolves branches through
    /// `active_launched_branches`, so a launch the tracking lost — which is
    /// exactly what emptying `launched_issues` produces — can merge without the
    /// Monitor ever noticing. This nominates the same work from the other side:
    /// a non-terminal row that is not currently launched, whose work branch has
    /// merged.
    ///
    /// Nomination is deliberately not completion. A branch merged for a
    /// *previous* generation of a reopened issue is also in that list forever,
    /// so the caller decides with the generation-aware linked-PR probe. Keeping
    /// the cheap local filter and the authoritative remote check separate is
    /// what bounds the probe to branch matches instead of the whole queue.
    pub fn untracked_merged_branch_candidates(
        &self,
        merged_branches: &BTreeSet<String>,
    ) -> Vec<IssueMonitorIssue> {
        self.inbox
            .iter()
            .filter(|item| {
                !item.state.is_terminal()
                    && item.issue.state == IssueMonitorIssueState::Open
                    && !self.merged_issues.contains(&item.issue.number)
                    && !self.active_launches.contains(&item.issue.number)
            })
            .filter(|item| {
                item.launch_plan
                    .as_ref()
                    .is_some_and(|plan| merged_branches.contains(&plan.branch_name))
            })
            .map(|item| item.issue.clone())
            .collect()
    }

    /// Whether any row could be nominated by
    /// [`Self::untracked_merged_branch_candidates`] at all. Lets the caller skip
    /// the merged-branch query entirely on a monitor with nothing to reconcile.
    pub fn has_open_launch_plan_candidates(&self) -> bool {
        self.inbox.iter().any(|item| {
            !item.state.is_terminal()
                && item.issue.state == IssueMonitorIssueState::Open
                && item.launch_plan.is_some()
                && !self.merged_issues.contains(&item.issue.number)
                && !self.active_launches.contains(&item.issue.number)
        })
    }

    /// Record a completion that the launch tracking never saw, once the caller's
    /// generation-aware probe confirmed it. Returns `false` when the shared
    /// completion policy decides the Issue still has open work, in which case
    /// the Issue returns to the queue rather than becoming terminal.
    pub fn record_untracked_completion(&mut self, issue_number: u64) -> bool {
        self.record_issue_completion(issue_number, IssueCompletionEvidence::LinkedPr)
    }

    /// An agent window closed without the work completing. Frees the active
    /// slot and returns the Issue to pending (`Queued`) — never a fabricated
    /// "done" state. Terminal states (Merged/Released/failed) are preserved.
    /// Returns the affected Issue number when the window mapped to an active
    /// launch that was re-queued.
    /// SPEC-3431 FR-069: free the slot held by a launch whose provider hit its
    /// usage limit, and hold the issue until `resets_at`.
    ///
    /// Deliberately unlike [`Self::requeue_window_at`]: no attempt is consumed
    /// and the backoff is the provider's own reset instant rather than the
    /// retry ladder. Nothing is wrong with the work or the agent — the account
    /// simply ran out — so charging it against the issue's retry budget would
    /// eventually escalate healthy work to `needs_human` for someone else's
    /// billing cycle.
    ///
    /// This is the single stall the mechanism resolves on its own, because it
    /// is the single stall whose cause the provider states outright. Every
    /// other stall is reported and left to the PM (SPEC-3431 FR-069), since an approval
    /// prompt, a rate limit, and a hang are indistinguishable from elapsed
    /// time alone.
    pub fn release_rate_limited_launch(
        &mut self,
        window_id: &str,
        resets_at: &str,
        now: &str,
    ) -> Option<u64> {
        let issue_number = self.launched_window_issue(window_id)?;
        let provider = self.saved_launch_provider();
        self.hold_provider_usage_limit_core(
            issue_number,
            provider.as_deref(),
            None,
            Some(resets_at),
            Some(IssueMonitorProviderQuotaHoldEvidence {
                recorded_at: now.to_string(),
                source: "usage_poller".to_string(),
                issue_number: Some(issue_number),
                window_id: Some(window_id.to_string()),
                screen_text: None,
                poller_state: None,
                poller_limit_reached: Some(true),
                poller_windows: Vec::new(),
            }),
            now,
        )
        .then_some(issue_number)
    }

    /// Issue #3616: hold `issue_number` because the provider backing its agent
    /// ran out of quota, only when `source_window_id` is still the exact window
    /// bound to that launch.
    ///
    /// The identity guard mirrors
    /// [`Self::try_requeue_agent_resume_writer_conflict`]: an issue number alone
    /// would let a dying pane revoke the successor launch that replaced it.
    ///
    /// `reason` is stored on the autonomous record so
    /// [`Self::agent_status`] can say *why* a queued issue is not launching —
    /// the failure this fixes was a quota-dead pane that read as `launched` for
    /// 23 minutes and then as terminal `AgentFailed`.
    #[allow(clippy::too_many_arguments)]
    pub fn try_hold_provider_usage_limit(
        &mut self,
        issue_number: u64,
        source_window_id: &str,
        provider: &str,
        reason: impl Into<String>,
        resets_at: Option<&str>,
        evidence: Option<IssueMonitorProviderQuotaHoldEvidence>,
        now: &str,
    ) -> IssueMonitorProviderUsageLimitOutcome {
        let Some(provider) = normalize_issue_monitor_provider(provider) else {
            return IssueMonitorProviderUsageLimitOutcome::Rejected;
        };
        if !self
            .launched_windows
            .get(&issue_number)
            .is_some_and(|stored_window_id| {
                issue_monitor_window_ids_match(stored_window_id, source_window_id)
            })
        {
            return IssueMonitorProviderUsageLimitOutcome::Rejected;
        }
        if self.hold_provider_usage_limit_core(
            issue_number,
            Some(&provider),
            Some(reason.into()),
            resets_at,
            evidence,
            now,
        ) {
            IssueMonitorProviderUsageLimitOutcome::Held
        } else {
            IssueMonitorProviderUsageLimitOutcome::Rejected
        }
    }

    /// Free the slot and hold the issue until the provider recovers. Shared by
    /// the identity-checked entry point and the legacy usage-poller path so the
    /// two can never disagree about what a quota hold does.
    fn hold_provider_usage_limit_core(
        &mut self,
        issue_number: u64,
        provider: Option<&str>,
        reason: Option<String>,
        resets_at: Option<&str>,
        evidence: Option<IssueMonitorProviderQuotaHoldEvidence>,
        now: &str,
    ) -> bool {
        if self.merged_issues.contains(&issue_number) {
            return false;
        }
        if self
            .inbox_item(issue_number)
            .is_some_and(|item| item.state.is_terminal())
        {
            return false;
        }
        let candidate_deadline = concrete_provider_quota_deadline(resets_at, now);
        let provider = provider.and_then(normalize_issue_monitor_provider);
        let floor = provider
            .as_deref()
            .and_then(|provider| {
                merge_provider_quota_hold(
                    &mut self.provider_quota_holds,
                    provider,
                    &candidate_deadline,
                )
            })
            .unwrap_or(candidate_deadline);
        if let Some(provider) = provider.as_deref() {
            // Issue #3923 AC-2: every hold carries what it was formed from,
            // and the same facts go to gwt.log so a false hold can be traced
            // to its trigger.
            let mut evidence = evidence.unwrap_or_else(|| IssueMonitorProviderQuotaHoldEvidence {
                recorded_at: now.to_string(),
                source: "unrecorded".to_string(),
                issue_number: None,
                window_id: None,
                screen_text: None,
                poller_state: None,
                poller_limit_reached: None,
                poller_windows: Vec::new(),
            });
            if parse_rfc3339_utc(&evidence.recorded_at).is_none() {
                evidence.recorded_at = now.to_string();
            }
            evidence.issue_number.get_or_insert(issue_number);
            tracing::warn!(
                provider = %provider,
                reset_at = %floor,
                issue_number,
                source = %evidence.source,
                window_id = ?evidence.window_id,
                screen_text = ?evidence.screen_text,
                poller_state = ?evidence.poller_state,
                poller_limit_reached = ?evidence.poller_limit_reached,
                poller_windows = ?evidence.poller_windows,
                reason = ?reason,
                "Issue Monitor provider quota hold recorded (Issue #3923)"
            );
            self.provider_quota_hold_evidence
                .insert(provider.to_string(), evidence);
        }
        let held_provider_matches_profile = provider
            .clone()
            .zip(self.saved_launch_provider())
            .is_some_and(|(held, saved)| held == saved);
        self.release_provider_limited_source_tracking(issue_number);
        if held_provider_matches_profile {
            self.compensate_uncommitted_provider_claims();
        }
        let record = self.autonomous_record_mut(issue_number);
        record.retry_not_before = Some(floor);
        record.retry_hold_reason = reason;
        record.retry_hold_provider = provider;
        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.claim_id = None;
            item.launched_window_id = None;
        }
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
        true
    }

    fn release_provider_limited_source_tracking(&mut self, issue_number: u64) {
        self.active_launches
            .retain(|active| *active != issue_number);
        self.launching_claimed_at.remove(&issue_number);
        self.launched_windows.remove(&issue_number);
        self.launched_claims.remove(&issue_number);
        self.launched_branches.remove(&issue_number);
        self.failed_windows.remove(&issue_number);
        self.pending_launches
            .retain(|pending| pending.issue_number != issue_number);
        self.queued_launch_session_strategies.remove(&issue_number);
        self.pending_review_dispatches
            .retain(|pending| pending.issue_number != issue_number);
    }

    /// Revoke only claim work that has not acquired a materializer. Ambiguous
    /// Attempting tuples stay in the journal until their late result arrives;
    /// exact ReleaseClaim companions make that result harmless without
    /// invalidating unrelated effect authority.
    fn compensate_uncommitted_provider_claims(&mut self) {
        let attempting_claims = self
            .pending_effects
            .iter()
            .filter(|effect| {
                effect.state == IssueMonitorEffectState::Attempting
                    && matches!(
                        effect.payload,
                        IssueMonitorEffectPayload::AcquireClaim { .. }
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        self.pending_effects.retain(|effect| {
            effect.state != IssueMonitorEffectState::Prepared
                || !matches!(
                    effect.payload,
                    IssueMonitorEffectPayload::AcquireClaim { .. }
                )
        });
        for effect in attempting_claims {
            if let IssueMonitorEffectPayload::AcquireClaim {
                issue_number,
                claim_id,
                owner,
                ..
            } = &effect.payload
            {
                ensure_claim_release_effect(
                    &mut self.pending_effects,
                    self.effect_authority_epoch,
                    &effect.effect_id,
                    *issue_number,
                    claim_id,
                    owner,
                );
            }
        }

        let cancelled_deliveries = self
            .pending_launch_deliveries
            .iter()
            .filter(|delivery| delivery.materializer_id.is_none())
            .cloned()
            .collect::<Vec<_>>();
        self.pending_launch_deliveries
            .retain(|delivery| delivery.materializer_id.is_some());
        for delivery in cancelled_deliveries {
            ensure_claim_release_effect(
                &mut self.pending_effects,
                self.effect_authority_epoch,
                &delivery.delivery_id,
                delivery.issue_number,
                &delivery.claim_id,
                &delivery.claim_owner,
            );
            self.queued_launch_session_strategies
                .insert(delivery.issue_number, delivery.launch_session_strategy);

            let launch_still_live = self.launched_windows.contains_key(&delivery.issue_number)
                || self
                    .pending_launch_deliveries
                    .iter()
                    .any(|pending| pending.issue_number == delivery.issue_number);
            if launch_still_live {
                continue;
            }
            self.active_launches
                .retain(|active| *active != delivery.issue_number);
            self.launching_claimed_at.remove(&delivery.issue_number);
            self.set_active_launch_id(delivery.issue_number, None);
            if let Some(item) = self
                .inbox
                .iter_mut()
                .find(|item| item.issue.number == delivery.issue_number)
            {
                if !item.state.is_terminal() {
                    item.state = MonitorInboxState::Queued;
                    item.claim_id = None;
                    item.launched_window_id = None;
                }
            }
            if !self.queue.contains(&delivery.issue_number) {
                self.queue.push_back(delivery.issue_number);
            }
        }
        self.apply_priority_order_to_queue();
    }

    /// Issue #3616 AC-5: drop a retry hold so an explicit human/PM launch
    /// instruction is not silently ignored.
    ///
    /// A provider reset can be days out. Switching the launch profile to a
    /// healthy provider is the recovery, and it is useless if the hold still
    /// gates `retry_ready`. Returns whether a hold was actually cleared.
    pub fn clear_retry_hold(&mut self, issue_number: u64) -> bool {
        let Some(record) = self.autonomous_records.get_mut(&issue_number) else {
            return false;
        };
        let had_hold = record.retry_not_before.is_some()
            || record.retry_hold_reason.is_some()
            || record.retry_hold_provider.is_some();
        record.retry_not_before = None;
        record.retry_hold_reason = None;
        record.retry_hold_provider = None;
        had_hold
    }

    /// Issue #3832: release a stale completion hold without weakening a
    /// durable GitHub Closed observation.
    ///
    /// The higher-generation Reopened record is the recovery tombstone. It
    /// removes the compatibility `merged_issues` projection and fences an
    /// older writer from restoring it. A live row is queued immediately; a
    /// cold row remains cold until the next complete scan observes it.
    pub fn clear_completion_hold(&mut self, issue_number: u64) -> bool {
        if self.issue_is_closed(issue_number)
            || self
                .inbox_item(issue_number)
                .is_some_and(|item| item.issue.state == IssueMonitorIssueState::Closed)
        {
            return false;
        }
        let Some(completed) = self
            .completion_records
            .get(&issue_number)
            .filter(|record| record.state == IssueCompletionState::Completed)
            .cloned()
        else {
            return false;
        };
        let live_issue = self
            .inbox_item(issue_number)
            .map(|item| item.issue.clone())
            .filter(|issue| issue.state == IssueMonitorIssueState::Open);

        self.clear_active_tracking(issue_number);
        self.clear_autonomous_record(issue_number);
        self.transition_completion(
            issue_number,
            IssueCompletionState::Reopened,
            live_issue
                .as_ref()
                .and_then(|issue| issue.updated_at.clone())
                .or(completed.issue_updated_at),
            IssueCompletionEvidence::Legacy,
        );
        self.require_fresh_launch_session(issue_number);
        if let Some(issue) = live_issue {
            self.set_inbox_state(issue_number, MonitorInboxState::Queued);
            self.record_candidate(issue);
        }
        true
    }

    /// SPEC-3431 FR-033: the window currently bound to `issue_number`.
    pub fn launched_window_id(&self, issue_number: u64) -> Option<String> {
        self.launched_windows
            .get(&issue_number)
            .cloned()
            .or_else(|| {
                self.inbox_item(issue_number)
                    .and_then(|item| item.launched_window_id.clone())
            })
    }

    /// SPEC-3431 FR-033: the delivery still awaiting an ACK for `issue_number`.
    ///
    /// `None` once the GUI has ACKed the launch, because
    /// [`Self::complete_active_launch_delivery`] consumes the delivery at that
    /// point — a running agent is identified by its window, a materializing one
    /// by its delivery.
    pub fn pending_launch_delivery_id(&self, issue_number: u64) -> Option<String> {
        self.pending_launch_deliveries
            .iter()
            .find(|delivery| delivery.issue_number == issue_number)
            .map(|delivery| delivery.delivery_id.clone())
    }

    /// SPEC-3431 FR-033: stop one launch and hold its issue, without requeueing.
    ///
    /// This is the "stop" half of the Monitor-owned lifecycle, and it is
    /// deliberately not [`Self::requeue_window_at`]. A close says "this attempt
    /// failed, try again" and spends a retry; a stop says "do not run this
    /// now", so it consumes no attempt and puts nothing back in the queue.
    /// FR-029〜031's `failover_restart` is the other half and composes this
    /// stop with a requeue and a saved-profile launch; it is a separate
    /// operation precisely so the two results cannot be confused.
    ///
    /// `target` must reproduce the live identity exactly. Anything else — an
    /// old window id, a foreign claim, a delivery that was already ACKed — is
    /// refused with the monitor untouched, because the failure mode being
    /// prevented is killing the *wrong* agent, which no later compensation can
    /// undo.
    pub fn stop_only(
        &mut self,
        target: &IssueMonitorStopTarget,
        reason: &str,
        now: &str,
    ) -> IssueMonitorStopOutcome {
        let issue_number = target.issue_number;

        // An issue already stopped by this operation answers the same request
        // idempotently. Checked before liveness, because the stop already
        // released the slot.
        if self.stop_only_reason(issue_number).is_some() {
            return IssueMonitorStopOutcome::AlreadyStopped;
        }

        let live_window = match self.resolve_exact_launch(target) {
            Ok(window_id) => window_id,
            Err(mismatch) => return self.refuse_stop(issue_number, mismatch),
        };

        // Revoke first: an effect that lands after the stop must not be able to
        // claim authority it no longer has.
        self.advance_effect_authority_epoch();
        self.record_autonomous_heartbeat(issue_number, now);
        // An operator stop is the operator's own decision; what happens next
        // is the operator's choice, so the row parks under that kind.
        self.escalate_to_needs_human(
            issue_number,
            NeedsHumanKind::UserChoiceRequired,
            format!("{STOP_ONLY_REASON_PREFIX}{reason}"),
        );

        IssueMonitorStopOutcome::Stopped {
            window_id: live_window.unwrap_or_default(),
        }
    }

    /// SPEC-3431 FR-033: the reason this issue was stopped, if it was.
    ///
    /// Read from `failed_issues`, which is persisted, so the hold survives the
    /// prefs roundtrip that happens between every `gwtd` invocation. The inbox
    /// error message says the same thing but only in a process that scanned.
    pub fn stop_only_reason(&self, issue_number: u64) -> Option<String> {
        self.failed_issues
            .get(&issue_number)
            .and_then(|message| message.strip_prefix(STOP_ONLY_REASON_PREFIX))
            .map(str::to_string)
    }

    /// SPEC-3431 FR-033: the claim backing the live launch for `issue_number`.
    ///
    /// The pending delivery carries it durably while the agent materializes;
    /// once the GUI ACKs, the delivery is consumed and only a scanned inbox
    /// row still knows it. Both are consulted so the answer is the same in the
    /// daemon and in a bare `gwtd` process.
    pub fn live_claim_id(&self, issue_number: u64) -> Option<String> {
        self.launched_claims
            .get(&issue_number)
            .cloned()
            .or_else(|| {
                self.pending_launch_deliveries
                    .iter()
                    .find(|delivery| delivery.issue_number == issue_number)
                    .map(|delivery| delivery.claim_id.clone())
            })
            .or_else(|| {
                self.inbox_item(issue_number)
                    .and_then(|item| item.claim_id.clone())
            })
    }

    /// SPEC-3431 FR-030/FR-033: resolve `target` against the one live launch it
    /// claims to name, or say which component disagreed.
    ///
    /// Shared by `stop_only` and `failover_restart` so the two can never drift
    /// into different notions of "the same agent" — the whole point of the
    /// exact match is that both operations refuse the same stale requests.
    ///
    /// Liveness comes from the durable launch accounting, not the inbox.
    /// [`Self::with_prefs`] restores `active_launches` but performs no
    /// candidate scan, so a short-lived process (every `gwtd` invocation) has
    /// an empty inbox for a running agent; reading liveness off the inbox would
    /// answer `UnknownIssue` in exactly the process the PM calls from.
    fn resolve_exact_launch(
        &self,
        target: &IssueMonitorStopTarget,
    ) -> Result<Option<String>, IssueMonitorStopMismatch> {
        let issue_number = target.issue_number;
        let inbox_state = self.inbox_item(issue_number).map(|item| item.state);
        if !self.active_launches.contains(&issue_number) {
            // Distinguish "never heard of it" from "not currently running", so
            // the PM can tell a typo apart from a race it lost.
            let known = inbox_state.is_some()
                || self.failed_issues.contains_key(&issue_number)
                || self.merged_issues.contains(&issue_number);
            return Err(if known {
                IssueMonitorStopMismatch::NotRunning
            } else {
                IssueMonitorStopMismatch::UnknownIssue
            });
        }
        // A terminal row still holding a slot is being reconciled elsewhere;
        // relabelling it would overwrite that outcome.
        if inbox_state.is_some_and(|state| state.is_terminal())
            || self.merged_issues.contains(&issue_number)
        {
            return Err(IssueMonitorStopMismatch::NotRunning);
        }

        if target.claim_id != self.live_claim_id(issue_number) {
            return Err(IssueMonitorStopMismatch::ClaimMismatch);
        }
        if target.delivery_id != self.pending_launch_delivery_id(issue_number) {
            return Err(IssueMonitorStopMismatch::DeliveryMismatch);
        }
        let live_window = self.launched_window_id(issue_number);
        match (target.window_id.as_deref(), live_window.as_deref()) {
            (Some(requested), Some(live)) if issue_monitor_window_ids_match(live, requested) => {}
            // A `Launching` issue has no window yet, so both sides must agree
            // that there is none.
            (None, None) => {}
            _ => return Err(IssueMonitorStopMismatch::WindowMismatch),
        }
        Ok(live_window)
    }

    /// SPEC-3431 FR-029〜031: stop one launch and put its issue back in line for
    /// the currently saved launch profile.
    ///
    /// The other half of the Monitor-owned lifecycle. Where `stop_only` holds
    /// the issue, this one releases it back to the queue head so the ordinary
    /// claim/slot path relaunches it — which is what makes it a *provider
    /// failover*: the profile the next launch reads is whatever is saved now,
    /// so switching provider is a profile edit followed by this call.
    ///
    /// It consumes no retry attempt, deliberately. The work did not fail; an
    /// operator (or a rate limit) decided a different provider should run it,
    /// and charging that to the issue's budget would escalate healthy work to
    /// `needs_human` for reasons that have nothing to do with the work.
    ///
    /// The old session is not reused: the launch identity is cleared, so the
    /// relaunch materializes a fresh conversation rather than resuming the
    /// provider that was just abandoned.
    pub fn failover_restart(
        &mut self,
        target: &IssueMonitorStopTarget,
        reason: &str,
        now: &str,
    ) -> IssueMonitorFailoverOutcome {
        let issue_number = target.issue_number;
        let live_window = match self.resolve_exact_launch(target) {
            Ok(window_id) => window_id,
            Err(mismatch) => {
                self.refuse_stop(issue_number, mismatch);
                return IssueMonitorFailoverOutcome::Mismatch(mismatch);
            }
        };

        // Revoke before requeueing: an effect still in flight for the old
        // provider must not be able to act once the issue belongs to the next
        // launch.
        if self.advance_effect_authority_epoch().is_none() {
            return IssueMonitorFailoverOutcome::AuthorityExhausted;
        }
        self.clear_active_tracking(issue_number);
        self.require_fresh_launch_session(issue_number);
        self.set_autonomous_phase(issue_number, AutonomousPhase::Idle);
        self.set_active_launch_id(issue_number, None);
        self.record_autonomous_heartbeat(issue_number, now);
        // A failover is not a failure, so any earlier failure marker for this
        // issue must not survive to hold it out of the queue.
        self.failed_issues.remove(&issue_number);
        self.failed_windows.remove(&issue_number);
        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.launched_window_id = None;
            item.claim_id = None;
            item.error_message = None;
        }
        // Head of the queue: the operator asked for this issue to run next, not
        // eventually.
        self.priority_order
            .retain(|existing| *existing != issue_number);
        self.priority_order.insert(0, issue_number);
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
        }
        self.apply_priority_order_to_queue();
        self.push_autonomous_notice(
            "info",
            issue_number,
            format!("Issue #{issue_number} failover: {reason}"),
        );

        IssueMonitorFailoverOutcome::Restarting {
            stopped_window_id: live_window,
        }
    }

    /// Issue #3645 / #3628: release the failure holding one issue out of the
    /// queue, without a live launch to name.
    ///
    /// [`Self::stop_only`] and [`Self::failover_restart`] both resolve an exact
    /// live launch first, which is right for them and fatal here: a row that
    /// reached `AgentFailed` has *already* lost its launch, so every path that
    /// demands one refuses it and the only remaining recovery is a hand edit of
    /// the state file. This operation is deliberately separate rather than a
    /// relaxation of the failover gate, because relaxing that gate would make
    /// one call sometimes kill a running agent and sometimes resurrect a dead
    /// row — the exact ambiguity the exact-identity match exists to prevent.
    ///
    /// It consumes no new retry attempt: nothing about the work failed, an
    /// operator decided the hold was wrong. Instead it resets the persisted
    /// count so an exhausted row starts one fresh bounded retry cycle.
    pub fn requeue_failed_issue(
        &mut self,
        issue_number: u64,
        reason: &str,
        now: &str,
    ) -> IssueMonitorRequeueOutcome {
        // Fail closed on anything a launch still owns. `record_failed_issue`
        // clears active tracking, so a genuinely failed row never trips this;
        // a row that does trip it is live, and killing a running agent is the
        // one mistake no later compensation can undo.
        if self.active_launches.contains(&issue_number)
            || self.launched_windows.contains_key(&issue_number)
            || self
                .pending_launch_deliveries
                .iter()
                .any(|delivery| delivery.issue_number == issue_number)
        {
            return IssueMonitorRequeueOutcome::LaunchLive;
        }
        if !self.failed_issues.contains_key(&issue_number) {
            return IssueMonitorRequeueOutcome::NotHeld;
        }

        let stale_window_id = self.failed_windows.get(&issue_number).cloned();
        let attempts_before = self.attempt_count(issue_number);
        self.failure_release_version += 1;
        let release = IssueMonitorReleasedFailure {
            issue_number,
            release_version: self.failure_release_version,
            released_at: now.to_string(),
            reason: reason.to_string(),
            attempts_before,
            attempts_after: 0,
        };
        self.released_failures.insert(issue_number, release.clone());
        self.merge_requeue_audit(std::iter::once(release));
        self.apply_failure_release(issue_number, true);
        self.push_autonomous_notice(
            "info",
            issue_number,
            format!("Issue #{issue_number} requeued by operator: {reason}"),
        );
        IssueMonitorRequeueOutcome::Requeued {
            stale_window_id,
            attempts_before,
            attempts_after: self.attempt_count(issue_number),
        }
    }

    /// Issue #3683 (AC-3): release a `BlockedByClaim` hold an operator decided
    /// is wrong, e.g. one anchored to a stale claim from a pre-identity-
    /// migration owner.
    ///
    /// The hold itself lives only in the driving process's inbox, which this
    /// process cannot see, so the release is published through the same
    /// versioned [`IssueMonitorPrefs::released_failures`] machinery as
    /// [`Self::requeue_failed_issue`]: the holding driver adopts it on its next
    /// prefs rebase and returns the issue to its queue. Verifying that the hold
    /// actually exists is the caller's job (via the daemon status projection);
    /// an unfounded release converges to a no-op, because the claim layer
    /// re-validates against the live GitHub claims on the next acquire and
    /// simply re-records the block while a foreign claim is genuinely active.
    pub fn release_claim_block(
        &mut self,
        issue_number: u64,
        reason: &str,
        now: &str,
    ) -> IssueMonitorRequeueOutcome {
        // The same fail-closed launch gate as `requeue_failed_issue`: releasing
        // a claim out from under a live launch would let a second agent claim
        // the same issue.
        if self.active_launches.contains(&issue_number)
            || self.launched_windows.contains_key(&issue_number)
            || self
                .pending_launch_deliveries
                .iter()
                .any(|delivery| delivery.issue_number == issue_number)
        {
            return IssueMonitorRequeueOutcome::LaunchLive;
        }

        let attempts_before = self.attempt_count(issue_number);
        self.failure_release_version += 1;
        let release = IssueMonitorReleasedFailure {
            issue_number,
            release_version: self.failure_release_version,
            released_at: now.to_string(),
            reason: reason.to_string(),
            attempts_before,
            attempts_after: 0,
        };
        self.released_failures.insert(issue_number, release.clone());
        self.merge_requeue_audit(std::iter::once(release));
        self.apply_failure_release(issue_number, true);
        self.push_autonomous_notice(
            "info",
            issue_number,
            format!("Issue #{issue_number} claim block released by operator: {reason}"),
        );
        IssueMonitorRequeueOutcome::Requeued {
            stale_window_id: None,
            attempts_before,
            attempts_after: self.attempt_count(issue_number),
        }
    }

    /// Apply one released hold. Shared by [`Self::requeue_failed_issue`] and by
    /// the cross-process adoption of a release another process committed, so a
    /// converged process cannot land in a different state than the one that
    /// issued the recovery.
    fn apply_failure_release(&mut self, issue_number: u64, revoke_claims: bool) {
        let removed_banner = self
            .failed_issues
            .get(&issue_number)
            .map(|message| format!("issue #{issue_number}: {message}"));
        self.failed_issues.remove(&issue_number);
        self.failed_windows.remove(&issue_number);
        self.clear_active_tracking(issue_number);
        if revoke_claims {
            revoke_uncommitted_claims_for_issue(
                &mut self.pending_effects,
                self.effect_authority_epoch,
                issue_number,
            );
        }
        // The abandoned conversation is what stranded this issue; resuming it
        // would reproduce the failure the recovery is undoing.
        self.require_fresh_launch_session(issue_number);
        if let Some(record) = self.autonomous_records.get_mut(&issue_number) {
            record.attempts = 0;
            record.retry_not_before = None;
            record.retry_hold_reason = None;
            record.retry_hold_provider = None;
            record.active_launch_id = None;
            record.last_heartbeat = None;
            record.phase = AutonomousPhase::Idle;
            record.needs_human_kind = None;
            record.steering = None;
        }
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Queued;
            item.claim_id = None;
            item.launched_window_id = None;
            item.error_message = None;
        }
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
        }
        self.apply_priority_order_to_queue();
        if removed_banner
            .as_ref()
            .is_some_and(|banner| self.last_error.as_ref() == Some(banner))
        {
            self.last_error = self.first_failed_issue_banner();
        }
    }

    /// Converge on operator recoveries committed by another process.
    ///
    /// Only releases issued *after* the snapshot this process already observed
    /// are adopted. Without that watermark a release would also erase failures
    /// recorded after it — the process that just discovered a real failure would
    /// undo its own finding on the next rebase.
    fn adopt_failure_releases_from_prefs(&mut self, disk: &IssueMonitorPrefs) {
        let observed = self.failure_release_version;
        for release in &disk.released_failures {
            if release.release_version <= observed {
                continue;
            }
            // A completed issue is terminal; re-queueing it would relaunch
            // delivered work.
            if self.merged_issues.contains(&release.issue_number) {
                continue;
            }
            self.released_failures
                .insert(release.issue_number, release.clone());
            self.merge_requeue_audit(std::iter::once(release.clone()));
            self.apply_failure_release(release.issue_number, true);
        }
        self.failure_release_version = observed.max(disk.failure_release_version);
    }

    /// Reapply active recovery fences after load and after every companion
    /// merge. Adoption watermarks deduplicate release events, but cannot stop a
    /// union merge from reintroducing the abandoned launch later.
    fn enforce_failure_release_fences(&mut self) {
        let issue_numbers = self.released_failures.keys().copied().collect::<Vec<_>>();
        for issue_number in issue_numbers {
            let terminal = self.merged_issues.contains(&issue_number)
                || self.inbox_item(issue_number).is_some_and(|item| {
                    matches!(
                        item.state,
                        MonitorInboxState::Merged | MonitorInboxState::Released
                    )
                });
            if terminal {
                self.released_failures.remove(&issue_number);
                continue;
            }
            // Do not revoke Prepared claims repeatedly: a claim planned after
            // the release is the legitimate fresh lifecycle. Newly created or
            // newly adopted releases use `true` and revoke old claims once.
            self.apply_failure_release(issue_number, false);
        }
    }

    /// Consume a local fence only from positive disk evidence produced by the
    /// exact confirmed-claim transaction. The disk watermark proves it had
    /// observed the release; a durable active companion plus absence of both
    /// release and failure then proves a fresh lifecycle superseded it.
    fn consume_releases_superseded_by_disk_launches(&mut self, disk: &IssueMonitorPrefs) {
        let disk_active = disk
            .launched_issues
            .iter()
            .map(|launch| launch.issue_number)
            .chain(
                disk.launching_issues
                    .iter()
                    .map(|launch| launch.issue_number),
            )
            .chain(
                disk.pending_launch_deliveries
                    .iter()
                    .map(|delivery| delivery.issue_number),
            )
            .collect::<BTreeSet<_>>();
        self.released_failures.retain(|issue_number, release| {
            let disk_keeps_release = disk
                .released_failures
                .iter()
                .any(|disk_release| disk_release.issue_number == *issue_number);
            let disk_has_failure = disk
                .failed_issues
                .iter()
                .any(|failed| failed.issue_number == *issue_number);
            let exact_fresh_launch = disk.failure_release_version >= release.release_version
                && disk_active.contains(issue_number)
                && !disk_keeps_release
                && !disk_has_failure;
            !exact_fresh_launch
        });
    }

    /// Merge operator reset evidence by release sequence and retain only the
    /// newest bounded window. Disk entries win a same-version conflict; the
    /// release counter is serialized under the prefs transaction lock, so such
    /// a conflict can only come from corrupted or legacy input.
    fn merge_requeue_audit(
        &mut self,
        incoming: impl IntoIterator<Item = IssueMonitorReleasedFailure>,
    ) {
        let mut by_version = self
            .requeue_audit
            .drain(..)
            .map(|entry| (entry.release_version, entry))
            .collect::<BTreeMap<_, _>>();
        for entry in incoming {
            by_version.insert(entry.release_version, entry);
        }
        let drop_count = by_version.len().saturating_sub(REQUEUE_AUDIT_CAP);
        self.requeue_audit = by_version.into_values().skip(drop_count).collect();
    }

    /// Restore the invariant that one issue is never both failed and released.
    /// Called after every path that can adopt a failure from disk, so a stale
    /// release can never be republished alongside the failure that superseded
    /// it.
    fn drop_releases_for_failed_issues(&mut self) {
        self.released_failures
            .retain(|issue_number, _| !self.failed_issues.contains_key(issue_number));
    }

    /// Recover an AgentFailed writer conflict only when the reporting window
    /// is still the exact launched window for the hinted issue.
    pub fn requeue_agent_resume_writer_conflict(
        &mut self,
        issue_number: u64,
        source_window_id: &str,
        message: impl Into<String>,
        holder_window_id: Option<&str>,
    ) -> bool {
        self.try_requeue_agent_resume_writer_conflict(
            issue_number,
            source_window_id,
            message,
            holder_window_id,
        ) == IssueMonitorResumeWriterConflictOutcome::Requeued
    }

    pub fn try_requeue_agent_resume_writer_conflict(
        &mut self,
        issue_number: u64,
        source_window_id: &str,
        message: impl Into<String>,
        holder_window_id: Option<&str>,
    ) -> IssueMonitorResumeWriterConflictOutcome {
        if !self
            .launched_windows
            .get(&issue_number)
            .is_some_and(|stored_window_id| {
                issue_monitor_window_ids_match(stored_window_id, source_window_id)
            })
        {
            return IssueMonitorResumeWriterConflictOutcome::Rejected;
        }
        self.requeue_resume_writer_conflict_core(issue_number, message, holder_window_id)
    }

    /// Recover a LaunchFailed writer conflict only when the reporting
    /// materializer still owns the exact pending delivery for this issue.
    pub fn requeue_launch_resume_writer_conflict(
        &mut self,
        issue_number: u64,
        source_delivery_id: &str,
        source_materializer_id: &str,
        message: impl Into<String>,
        holder_window_id: Option<&str>,
    ) -> bool {
        self.try_requeue_launch_resume_writer_conflict(
            issue_number,
            source_delivery_id,
            source_materializer_id,
            message,
            holder_window_id,
        ) == IssueMonitorResumeWriterConflictOutcome::Requeued
    }

    pub fn try_requeue_launch_resume_writer_conflict(
        &mut self,
        issue_number: u64,
        source_delivery_id: &str,
        source_materializer_id: &str,
        message: impl Into<String>,
        holder_window_id: Option<&str>,
    ) -> IssueMonitorResumeWriterConflictOutcome {
        let source_matches = self.pending_launch_deliveries.iter().any(|delivery| {
            delivery.issue_number == issue_number
                && delivery.delivery_id == source_delivery_id
                && delivery.materializer_id.as_deref() == Some(source_materializer_id)
        });
        if !source_matches {
            return IssueMonitorResumeWriterConflictOutcome::Rejected;
        }
        self.requeue_resume_writer_conflict_core(issue_number, message, holder_window_id)
    }

    /// Apply the shared mutation only after a public entry point validates the
    /// exact source identity. Keeping this private prevents an issue hint alone
    /// from revoking a newer successor launch.
    fn requeue_resume_writer_conflict_core(
        &mut self,
        issue_number: u64,
        message: impl Into<String>,
        holder_window_id: Option<&str>,
    ) -> IssueMonitorResumeWriterConflictOutcome {
        let inbox_state = self.inbox_item(issue_number).map(|item| item.state);
        if self.merged_issues.contains(&issue_number)
            || self.failed_issues.contains_key(&issue_number)
            || inbox_state.is_some_and(MonitorInboxState::is_terminal)
        {
            return IssueMonitorResumeWriterConflictOutcome::Rejected;
        }

        // Revoke before dropping the old delivery/tracking. If the epoch
        // cannot advance, leave the complete transition unapplied.
        if self.advance_effect_authority_epoch().is_none() {
            return IssueMonitorResumeWriterConflictOutcome::AuthorityExhausted;
        }
        self.clear_active_tracking(issue_number);
        self.require_fresh_launch_session(issue_number);
        if let Some(record) = self.autonomous_records.get_mut(&issue_number) {
            record.phase = AutonomousPhase::Idle;
            record.active_launch_id = None;
        }
        self.queue.retain(|queued| *queued != issue_number);
        self.queue.push_back(issue_number);
        self.apply_priority_order_to_queue();

        let message = message.into();
        let diagnostic = holder_window_id
            .map(str::trim)
            .filter(|holder| !holder.is_empty())
            .map(|holder| format!("{message} (writer held by window {holder})"))
            .unwrap_or(message);
        self.last_error = Some(format!("issue #{issue_number}: {diagnostic}"));
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Queued;
            item.claim_id = None;
            item.blocked_by_owner = None;
            item.claim_expires_at = None;
            item.launched_window_id = None;
            item.error_message = Some(diagnostic);
            item.exclusion_reason = None;
        }
        IssueMonitorResumeWriterConflictOutcome::Requeued
    }

    /// SPEC-3431 FR-033: refuse a stop without mutating any launch state.
    ///
    /// The reason is recorded as a diagnostic (FR-031) only; nothing is closed,
    /// requeued, or relabelled.
    fn refuse_stop(
        &mut self,
        issue_number: u64,
        mismatch: IssueMonitorStopMismatch,
    ) -> IssueMonitorStopOutcome {
        self.last_error = Some(format!(
            "issue #{issue_number}: stop refused ({mismatch:?}) — identity did not match the live launch"
        ));
        IssueMonitorStopOutcome::Mismatch(mismatch)
    }

    pub fn requeue_window(&mut self, window_id: &str) -> Option<u64> {
        self.requeue_window_at(
            window_id,
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
    }

    /// Requeue only the exact persisted launch generation named by `target`.
    ///
    /// A detached close may finish after a successor has reused both the issue
    /// number and window id. The claim id is the durable launch generation, so
    /// the stale close becomes an inert CAS miss instead of revoking that
    /// successor.
    pub fn requeue_exact_window(&mut self, target: &IssueMonitorStopTarget) -> Option<u64> {
        let window_id = self.resolve_exact_launch(target).ok().flatten()?;
        self.requeue_window(&window_id)
            .filter(|issue_number| *issue_number == target.issue_number)
    }

    /// Issue #3927 (SPEC #3340 FR-044): report the durable facts the runtime's
    /// terminal predicate consumes for `window_id` linked to `issue_number`.
    pub fn terminal_window_facts(
        &self,
        issue_number: u64,
        window_id: &str,
    ) -> IssueMonitorTerminalWindowFacts {
        let bound_window = self.launched_window_id(issue_number);
        let binds_this_window = bound_window
            .as_deref()
            .is_some_and(|bound| issue_monitor_window_ids_match(bound, window_id));
        IssueMonitorTerminalWindowFacts {
            binds_this_window,
            binds_other_window: bound_window.is_some() && !binds_this_window,
            issue_closed: self.issue_is_closed(issue_number)
                || self.merged_issues.contains(&issue_number),
            needs_human: self
                .inbox_item(issue_number)
                .is_some_and(|item| item.state == MonitorInboxState::NeedsHuman)
                || self
                    .autonomous_records
                    .get(&issue_number)
                    .is_some_and(|record| record.phase == AutonomousPhase::NeedsHuman),
            failure_hold: self.failed_issues.contains_key(&issue_number),
        }
    }

    /// Issue #3927 (SPEC #3340 Phase 10S, PM ruling `1f7fdc9e`): settle one
    /// successful exact terminal delivery.
    ///
    /// The runtime observed that the launched window's Work is canonically
    /// terminal (settled execution record or durable closed Issue) and is
    /// about to close the window itself. Unlike [`Self::requeue_exact_window`]
    /// — the `window_closed` contract, which says "this attempt failed, try
    /// again" — a settled delivery is a success: the active slot is released
    /// in this mutation, no retry attempt is spent, and the row stays
    /// `Launched` out of the queue until the ordinary completion probe (merged
    /// PR / closed Issue) ends it. The window is unbound so the later
    /// vanished-window reconciliation cannot mistake the runtime's own close
    /// for a failed attempt.
    ///
    /// `target` must reproduce the live identity exactly, including the
    /// window; a `Launching` row without a window has delivered nothing.
    pub fn settle_exact_terminal_delivery(
        &mut self,
        target: &IssueMonitorStopTarget,
    ) -> Result<u64, IssueMonitorStopMismatch> {
        let issue_number = target.issue_number;
        if self.resolve_exact_launch(target)?.is_none() {
            return Err(IssueMonitorStopMismatch::WindowMismatch);
        }
        self.clear_active_tracking(issue_number);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.launched_window_id = None;
        }
        self.queue.retain(|queued| *queued != issue_number);
        Ok(issue_number)
    }

    /// [`Self::requeue_window`] with an injected clock for the backoff floor.
    ///
    /// SPEC-3431 FR-066: a close is a bounded retry, not a free one. It used to
    /// consume no attempt and set no backoff, so under autonomous mode the
    /// closed issue went straight back to the queue head and the next scan
    /// relaunched it immediately — closing a pane restarted it instead of
    /// stopping it.
    ///
    /// The PM's contract worked around that by forbidding pane closes, but the
    /// constraint protected nobody: the only callers are the `WindowClosed`
    /// control, so a person closing a window by hand hit the same loop. Bound
    /// it here, where the loop is, and the capability is safe for everyone.
    ///
    /// The attempt ladder, backoff, and `NeedsHuman` escalation already exist
    /// for autonomous failures; a close is a failed attempt by the same
    /// definition ("closed without the work completing"), so it shares that
    /// budget rather than introducing a second one that could disagree.
    pub fn requeue_window_at(&mut self, window_id: &str, now: &str) -> Option<u64> {
        let issue_number = self.launched_window_issue(window_id)?;
        if self.merged_issues.contains(&issue_number) {
            return None;
        }
        if self
            .inbox_item(issue_number)
            .is_some_and(|item| item.state.is_terminal())
        {
            return None;
        }
        let attempt = self.record_attempt(issue_number);
        let max = self.autonomous_tuning.max_attempts;
        // Issue #3944 AC-6: the human-gated flow keeps its cap-park unchanged.
        // AC-2: under autonomous mode a closed window is a dead launch — it is
        // requeued at the capped backoff with a steering request, never parked.
        if attempt >= max && !self.autonomous_mode {
            self.clear_active_tracking(issue_number);
            self.escalate_to_needs_human(
                issue_number,
                NeedsHumanKind::UserChoiceRequired,
                format!("closed without completing ({attempt}/{max} attempts used)"),
            );
            return Some(issue_number);
        }
        let backoff = autonomous_retry_backoff_secs(
            attempt,
            self.autonomous_tuning.retry_backoff_base_secs,
            self.autonomous_tuning.retry_backoff_cap_secs,
        );
        self.clear_active_tracking(issue_number);
        let record = self.autonomous_record_mut(issue_number);
        record.retry_not_before = rfc3339_plus_secs(now, backoff);
        record.retry_hold_reason = None;
        record.retry_hold_provider = None;
        if attempt >= max {
            self.request_autonomous_steering(
                issue_number,
                format!(
                    "closed without completing ({attempt}/{max} attempts used); retrying after backoff"
                ),
                now,
            );
        }
        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
        Some(issue_number)
    }

    fn record_failed_issue(
        &mut self,
        issue_number: u64,
        message: impl Into<String>,
        state: MonitorInboxState,
    ) {
        let message = message.into();
        // Issue #3941 AC-3: a launch aborted by transient infrastructure (exact
        // package probe timeout with no cached version, a remote-tracking ref
        // race between concurrent fetches) is neither an agent failure nor an
        // attempt: it is requeued behind a short backoff in every mode.
        if gwt_agent::is_transient_launch_failure(&message) {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            self.record_transient_launch_retry(issue_number, message, &now);
            return;
        }
        // SPEC #3200 (review follow-up): a failure for an in-flight autonomous
        // issue (e.g. the independent review agent could not spawn, leaving the
        // record in `Reviewing`) must funnel through the autonomous
        // retry/backoff/escalation machinery — otherwise the record strands in a
        // non-`Idle` phase forever and the daemon waits for a verdict that will
        // never arrive. The plain human-gated `LaunchFailed`/`AgentFailed` path
        // below is preserved for every non-autonomous issue.
        if self.autonomous_mode && self.is_autonomous_in_flight(issue_number) {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            self.record_autonomous_failure(issue_number, message, &now);
            return;
        }
        self.active_launches
            .retain(|active| *active != issue_number);
        // #3165 error-window lifecycle: retain the stale agent window id so an
        // explicit Launch Now can close it before relaunching. Prefer the
        // tracked launched window; fall back to the inbox item's window id.
        self.launched_claims.remove(&issue_number);
        let stale_window = self.launched_windows.remove(&issue_number).or_else(|| {
            self.inbox
                .iter()
                .find(|item| item.issue.number == issue_number)
                .and_then(|item| item.launched_window_id.clone())
        });
        if let Some(window_id) = stale_window {
            self.failed_windows.insert(issue_number, window_id);
        }
        self.failed_issues.insert(issue_number, message.clone());
        // A newer failure supersedes the recovery that preceded it, or the
        // stale release would erase this hold on the next cross-process rebase.
        self.released_failures.remove(&issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.pending_launches
            .retain(|pending| pending.issue_number != issue_number);
        self.queued_launch_session_strategies.remove(&issue_number);
        self.launch_auth_required = false;
        self.last_error = Some(format!("issue #{issue_number}: {message}"));
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = state;
            item.launched_window_id = None;
            item.error_message = Some(message);
        }
    }

    fn upsert_inbox(&mut self, item: IssueMonitorInboxItem) {
        if let Some(existing) = self
            .inbox
            .iter_mut()
            .find(|existing| existing.issue.number == item.issue.number)
        {
            *existing = item;
        } else {
            self.inbox.push(item);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorScanSummary {
    pub scanned: usize,
    pub claimed: usize,
    pub blocked: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub fn scan_issue_monitor_candidates(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    now: &str,
) -> IssueMonitorScanSummary {
    let mut summary = IssueMonitorScanSummary::default();
    monitor.last_scan_at = Some(now.to_string());
    monitor.last_error = None;
    monitor.launch_auth_required = false;

    for issue in issues {
        summary.scanned += 1;
        if issue.state == IssueMonitorIssueState::Closed {
            monitor.transition_issue_closure(
                issue.number,
                IssueClosureState::Closed,
                IssueClosureEvidence::ExplicitRevision,
                issue.updated_at.clone(),
            );
            summary.skipped += 1;
            continue;
        }
        if monitor.issue_is_closed(issue.number) {
            // Cache and LiveIncomplete inputs cannot supersede a durable close.
            // A complete Live observation transitions the fact before reaching
            // this shared scan loop.
            summary.skipped += 1;
            continue;
        }
        if !is_auto_improve_candidate(issue, &monitor.config) {
            summary.skipped += 1;
            continue;
        }

        monitor.record_candidate(issue.clone());
        if monitor.inbox_item(issue.number).is_some_and(|item| {
            matches!(
                item.state,
                MonitorInboxState::NotReady | MonitorInboxState::HoldExcluded
            )
        }) {
            summary.skipped += 1;
        }
    }

    summary
}

/// Canonical candidate scan for provenance-aware loaders. A historical
/// persisted failure is reconciled only for an unapplied project receiving a
/// complete live snapshot and only after the shared Launch Agent resolver
/// proves that the project now has a usable base branch. The resolved branch is
/// intentionally only a gate; this transition never changes branches and never
/// creates queue/claim/launch work directly.
pub fn scan_issue_monitor_candidates_with_provenance(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    source: IssueMonitorCandidateSource,
    project_root: &Path,
    now: &str,
) -> IssueMonitorScanSummary {
    scan_issue_monitor_candidates_for_project_tab_with_provenance(
        monitor,
        issues,
        source,
        project_root,
        None,
        now,
    )
}

/// Provenance-aware scan for a GUI observer bound to one project tab.
///
/// Destructive reconciliation is authorized only by a complete live snapshot.
/// The optional tab id lets the GUI positively identify qualified window ids
/// from another open project without guessing about legacy bare ids.
pub fn scan_issue_monitor_candidates_for_project_tab_with_provenance(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    source: IssueMonitorCandidateSource,
    project_root: &Path,
    expected_project_tab_id: Option<&str>,
    now: &str,
) -> IssueMonitorScanSummary {
    if monitor.legacy_git_launch_failure_migration_version
        < LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        && source == IssueMonitorCandidateSource::Live
        && crate::start_work::resolve_launch_agent_base_branch(project_root).is_ok()
    {
        monitor.apply_legacy_git_launch_failure_migration(project_root);
    }
    if source == IssueMonitorCandidateSource::Live {
        let mut tracked_issue_numbers = monitor.current_action_issue_numbers();
        tracked_issue_numbers.extend(monitor.closure_records.keys().copied());
        let observed_issue_numbers = issues
            .iter()
            .map(|issue| issue.number)
            .collect::<BTreeSet<_>>();
        for issue in issues
            .iter()
            .filter(|issue| issue.state == IssueMonitorIssueState::Open)
        {
            monitor.transition_issue_closure(
                issue.number,
                IssueClosureState::Reopened,
                IssueClosureEvidence::ExplicitRevision,
                issue.updated_at.clone(),
            );
        }
        for issue_number in tracked_issue_numbers.difference(&observed_issue_numbers) {
            monitor.transition_issue_closure(
                *issue_number,
                IssueClosureState::Closed,
                IssueClosureEvidence::CompleteLiveAbsence,
                None,
            );
        }
        monitor.reconcile_live_completion_records(issues);
        let live_issue_numbers = issues
            .iter()
            .map(|issue| issue.number)
            .collect::<BTreeSet<_>>();
        monitor.prune_inflight_launches_for_live_snapshot(
            &live_issue_numbers,
            expected_project_tab_id,
        );
    }
    scan_issue_monitor_candidates(monitor, issues, now)
}

fn expiry_from_now_lexical(now: &str, ttl_secs: u64) -> String {
    chrono::DateTime::parse_from_rfc3339(now)
        .map(|time| {
            (time + chrono::Duration::seconds(ttl_secs as i64))
                .to_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|_| now.to_string())
}

fn issue_monitor_window_ids_match(stored: &str, incoming: &str) -> bool {
    if stored == incoming {
        return true;
    }
    let stored_qualified = issue_monitor_qualified_window_id(stored);
    let incoming_qualified = issue_monitor_qualified_window_id(incoming);
    if let (Some((stored_tab, stored_raw)), Some((incoming_tab, incoming_raw))) =
        (stored_qualified, incoming_qualified)
    {
        return stored_tab == incoming_tab && stored_raw == incoming_raw;
    }
    let stored_raw = stored_qualified.map(|(_, raw)| raw).unwrap_or(stored);
    let incoming_raw = incoming_qualified.map(|(_, raw)| raw).unwrap_or(incoming);
    !stored_raw.is_empty() && stored_raw == incoming_raw
}

fn issue_monitor_qualified_window_id(window_id: &str) -> Option<(&str, &str)> {
    let (project_tab_id, raw_window_id) = window_id.split_once("::")?;
    (!project_tab_id.is_empty() && !raw_window_id.is_empty())
        .then_some((project_tab_id, raw_window_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #3676 AC-2: the auth verdict must be definitive before it may
    /// block a launch — every ambiguous shape stays fail-open (`Unknown`).
    #[test]
    fn provider_auth_state_codex_verdicts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        fs::create_dir_all(&codex_home).expect("create codex home");
        fn probe<'facts>(
            home: Option<&'facts Path>,
            key: Option<&'facts str>,
        ) -> ProviderAuthProbe<'facts> {
            ProviderAuthProbe {
                codex_home: home,
                claude_home: None,
                openai_api_key: key,
                anthropic_api_key: None,
            }
        }

        // No auth.json: the CLI would boot into its login screen.
        assert_eq!(
            provider_auth_state("codex", &probe(Some(&codex_home), None)),
            ProviderAuthState::Unauthenticated,
        );
        // An API key in the environment authenticates without auth.json.
        assert_eq!(
            provider_auth_state("codex", &probe(Some(&codex_home), Some("sk-test"))),
            ProviderAuthState::Authenticated,
        );
        // Whitespace-only keys are not credentials.
        assert_eq!(
            provider_auth_state("codex", &probe(Some(&codex_home), Some("  "))),
            ProviderAuthState::Unauthenticated,
        );
        // Unknown home: nothing definitive to say.
        assert_eq!(
            provider_auth_state("codex", &probe(None, None)),
            ProviderAuthState::Unknown,
        );

        let auth_path = codex_home.join("auth.json");
        fs::write(&auth_path, "{corrupt").expect("write corrupt auth");
        assert_eq!(
            provider_auth_state("codex", &probe(Some(&codex_home), None)),
            ProviderAuthState::Unauthenticated,
        );
        fs::write(&auth_path, r#"{"auth_mode":"chatgpt","tokens":{}}"#).expect("write empty auth");
        assert_eq!(
            provider_auth_state("codex", &probe(Some(&codex_home), None)),
            ProviderAuthState::Unauthenticated,
        );
        fs::write(
            &auth_path,
            r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"x","access_token":"tok","refresh_token":"ref","account_id":"acct"}}"#,
        )
        .expect("write chatgpt auth");
        assert_eq!(
            provider_auth_state("codex", &probe(Some(&codex_home), None)),
            ProviderAuthState::Authenticated,
        );
        fs::write(
            &auth_path,
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-file"}"#,
        )
        .expect("write apikey auth");
        assert_eq!(
            provider_auth_state("codex", &probe(Some(&codex_home), None)),
            ProviderAuthState::Authenticated,
        );
        // Agent id casing/whitespace must not change the verdict.
        assert_eq!(
            provider_auth_state(" Codex ", &probe(Some(&codex_home), None)),
            ProviderAuthState::Authenticated,
        );
    }

    /// Issue #3676 AC-2: Claude Code may keep credentials in the macOS
    /// Keychain, so a missing credentials file is `Unknown`, never a blocker.
    #[test]
    fn provider_auth_state_claude_fails_open() {
        let temp = tempfile::tempdir().expect("tempdir");
        let claude_home = temp.path().join("claude-home");
        fs::create_dir_all(&claude_home).expect("create claude home");
        fn probe<'facts>(
            home: &'facts Path,
            key: Option<&'facts str>,
        ) -> ProviderAuthProbe<'facts> {
            ProviderAuthProbe {
                codex_home: None,
                claude_home: Some(home),
                openai_api_key: None,
                anthropic_api_key: key,
            }
        }

        assert_eq!(
            provider_auth_state("claude", &probe(&claude_home, None)),
            ProviderAuthState::Unknown,
        );
        assert_eq!(
            provider_auth_state("claude", &probe(&claude_home, Some("sk-ant"))),
            ProviderAuthState::Authenticated,
        );
        fs::write(claude_home.join(".credentials.json"), "{}").expect("write credentials");
        assert_eq!(
            provider_auth_state("claude", &probe(&claude_home, None)),
            ProviderAuthState::Authenticated,
        );
        assert_eq!(
            provider_auth_state("claude-code", &probe(&claude_home, None)),
            ProviderAuthState::Authenticated,
        );
        // Unmapped agents have no observable credential surface.
        assert_eq!(
            provider_auth_state("opencode", &ProviderAuthProbe::default()),
            ProviderAuthState::Unknown,
        );
    }

    #[test]
    fn provider_unauthenticated_message_is_identifiable() {
        let message = provider_unauthenticated_message("codex");
        assert!(message.starts_with("provider_unauthenticated:"));
        assert!(message.contains("codex"));
    }

    fn issue(number: u64) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: Vec::new(),
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: IssueMonitorReadiness::NotApplicable,
            updated_at: Some("2026-08-01T00:00:00Z".to_string()),
        }
    }

    #[test]
    fn queue_and_active_issue_numbers_are_exposed_in_runtime_order() {
        let prefs = IssueMonitorPrefs {
            enabled: true,
            priority_order: vec![2, 1],
            launching_issues: vec![IssueMonitorLaunchingIssue {
                issue_number: 9,
                claimed_at: Some("2026-08-03T00:00:00Z".to_string()),
            }],
            ..IssueMonitorPrefs::default()
        };
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        let mut first = issue(1);
        first.labels.push("auto-improve".to_string());
        let mut second = issue(2);
        second.labels.push("auto-improve".to_string());

        scan_issue_monitor_candidates(&mut monitor, &[first, second], "2026-08-03T00:00:00Z");

        assert_eq!(monitor.queued_issue_numbers(), vec![2, 1]);
        assert_eq!(monitor.active_issue_numbers(), vec![9]);
    }

    #[test]
    fn agent_status_projects_blocked_claim_from_one_live_state() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 3,
            ..IssueMonitorConfig::default()
        });
        let mut candidate = issue(42);
        candidate.labels.push("auto-improve".to_string());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&candidate),
            "2026-08-03T00:00:00Z",
        );
        monitor.record_blocked_by_claim(candidate, "other-agent", "2026-08-03T00:05:00Z");

        assert_eq!(
            monitor.agent_status(),
            IssueMonitorAgentStatus {
                queue: Vec::new(),
                active_launches: Vec::new(),
                max_active: 3,
                enabled: true,
                autonomous_mode: false,
                has_launch_profile: false,
                quota_hold: None,
                provider_quota_holds: Vec::new(),
                needs_human: Vec::new(),
                inbox: vec![IssueMonitorInboxSummary {
                    issue_number: 42,
                    state: MonitorInboxState::BlockedByClaim,
                    github_state: IssueMonitorIssueState::Open,
                    issue_updated_at: Some("2026-08-01T00:00:00Z".to_string()),
                    readiness: IssueMonitorReadiness::NotApplicable,
                    recoverable_merged: false,
                    completion_reason: None,
                    blocked_by_owner: Some("other-agent".to_string()),
                    launched_window_id: None,
                    error_message: None,
                    // Never launched, so no activity clock was ever started
                    // and there is no launch identity to stop.
                    last_activity_at: None,
                    retry_not_before: None,
                    retry_hold_reason: None,
                    claim_id: None,
                    delivery_id: None,
                    waiting: None,
                    steering: None,
                }],
                last_error: None,
                last_scan_at: Some("2026-08-03T00:00:00Z".to_string()),
                // `agent_status` reports the queue; the scan-cadence check is
                // applied by `agent_status_at`, which needs a clock.
                scan_stall: None,
            }
        );
    }

    /// SPEC-3431 FR-024: everything the PM's reconcile loop acts on must be in
    /// one snapshot. `needs_human`, the inbox rows, and `last_error` used to
    /// exist only on the daemon's capacity-64 broadcast ring, so an escalation
    /// a lagging subscriber missed was unrecoverable — and the PM contract
    /// told the PM to read an `inbox` operation that does not exist.
    #[test]
    fn agent_status_snapshot_carries_needs_human_and_inbox() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..IssueMonitorConfig::default()
        });
        let mut escalated = issue(7);
        escalated.labels.push("auto-improve".to_string());
        let mut blocked = issue(8);
        blocked.labels.push("auto-improve".to_string());
        scan_issue_monitor_candidates(
            &mut monitor,
            &[escalated.clone(), blocked.clone()],
            "2026-08-05T00:00:00Z",
        );
        monitor.escalate_to_needs_human(
            7,
            NeedsHumanKind::UserChoiceRequired,
            "review exhausted its retries",
        );
        monitor.record_blocked_by_claim(blocked, "other-agent", "2026-08-05T00:05:00Z");

        let status = monitor.agent_status();

        assert_eq!(
            status.needs_human,
            vec![7],
            "an escalated issue must be visible without subscribing"
        );
        let escalated_row = status
            .inbox
            .iter()
            .find(|row| row.issue_number == 7)
            .expect("escalated issue has an inbox row");
        assert_eq!(escalated_row.state, MonitorInboxState::NeedsHuman);
        assert_eq!(
            escalated_row.error_message.as_deref(),
            Some("review exhausted its retries")
        );
        let blocked_row = status
            .inbox
            .iter()
            .find(|row| row.issue_number == 8)
            .expect("blocked issue has an inbox row");
        assert_eq!(blocked_row.blocked_by_owner.as_deref(), Some("other-agent"));
    }

    #[test]
    fn rejected_blocked_claim_removes_a_stale_nonqueued_queue_entry() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        let candidate = issue(42);
        monitor.record_candidate(candidate.clone());
        monitor.set_inbox_state(42, MonitorInboxState::HoldExcluded);

        assert!(!monitor.record_blocked_by_claim(candidate, "other-agent", "2026-08-05T10:30:00Z",));
        assert!(
            monitor.queued_issue_numbers().is_empty(),
            "a rejected late claim result must still guarantee synchronous loop progress"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::HoldExcluded)
        );
    }

    fn pending_arm_effect(
        effect_id: &str,
        authority_epoch: u64,
        attempt: u32,
        state: IssueMonitorEffectState,
    ) -> PendingIssueMonitorEffect {
        PendingIssueMonitorEffect {
            effect_id: effect_id.to_string(),
            authority_epoch,
            attempt,
            state,
            payload: IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 7,
                pr_number: 99,
                reviewed_sha: "abc123".to_string(),
            },
        }
    }

    fn effect_attempt_key(
        effect_id: &str,
        authority_epoch: u64,
        attempt: u32,
    ) -> IssueMonitorEffectAttemptKey {
        IssueMonitorEffectAttemptKey {
            effect_id: effect_id.to_string(),
            authority_epoch,
            attempt,
        }
    }

    #[test]
    fn launch_plan_uses_unified_execute_prompt_and_work_issue_branch() {
        let mut spec_issue = issue(3164);
        spec_issue.labels.push("gwt-spec".to_string());
        let spec_plan = issue_monitor_launch_plan(&spec_issue);

        assert_eq!(spec_plan.branch_name, "work/issue-3164");
        assert_eq!(spec_plan.prompt, "$gwt-execute #3164");
        assert_eq!(spec_plan.linked_issue_kind, LinkedIssueKind::Spec);

        let plain_plan = issue_monitor_launch_plan(&issue(42));
        assert_eq!(plain_plan.branch_name, "work/issue-42");
        assert_eq!(
            plain_plan.prompt,
            "$gwt-execute #42\n\nBefore changing code, evaluate every remaining acceptance criterion. If all criteria are already satisfied, close Issue #42 and finish without creating another implementation PR. Otherwise, implement only the remaining criteria."
        );
        assert_eq!(plain_plan.linked_issue_kind, LinkedIssueKind::Issue);
    }

    #[test]
    fn scan_keeps_queue_visible_when_processing_is_stopped() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: false,
            ..IssueMonitorConfig::default()
        });

        let summary = scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-06-23T10:00:00Z",
        );

        assert_eq!(summary.scanned, 2);
        assert_eq!(monitor.queue_len(), 2);
        assert_eq!(monitor.status_view().state, "disabled");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
    }

    #[test]
    fn launched_issue_from_prefs_stays_active_and_is_not_requeued() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 1,
                launched_issues: vec![IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                }],
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);

        let summary = scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-06-23T10:00:00Z",
        );

        assert_eq!(summary.scanned, 2);
        assert_eq!(monitor.status_view().state, "active");
        assert_eq!(monitor.active_count(), 1);
        assert_eq!(monitor.queue_len(), 1);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Launched)
        );
        assert_eq!(
            monitor
                .inbox_item(42)
                .and_then(|item| item.launched_window_id.as_deref()),
            Some("tab-1::agent-1")
        );
        assert!(
            monitor
                .next_launch_request("2026-07-02T00:00:00Z")
                .is_none(),
            "max_active=1 must keep the next queued issue waiting while launched work is active"
        );
    }

    fn launched_monitor(number: u64, window_id: &str) -> IssueMonitorState {
        launched_monitor_for_issue(issue(number), window_id)
    }

    fn launched_monitor_for_issue(
        candidate: IssueMonitorIssue,
        window_id: &str,
    ) -> IssueMonitorState {
        let number = candidate.number;
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&candidate),
            "2026-06-26T00:00:00Z",
        );
        monitor.complete_active_launch(number, window_id);
        assert_eq!(monitor.active_count(), 1);
        monitor
    }

    fn test_launch_profile(agent_id: &str) -> IssueMonitorLaunchProfile {
        IssueMonitorLaunchProfile {
            agent_id: agent_id.to_string(),
            model: None,
            reasoning: None,
            version: None,
            session_mode: Default::default(),
            skip_permissions: false,
            codex_fast_mode: false,
            runtime_target: Default::default(),
            docker_service: None,
            docker_lifecycle_intent: Default::default(),
            windows_shell: None,
        }
    }

    fn assert_manual_relaunch_accepts_fresh_lifecycle_failures(base: &IssueMonitorState) {
        for (agent_failure, expected_state) in [
            (false, MonitorInboxState::LaunchFailed),
            (true, MonitorInboxState::AgentFailed),
        ] {
            let mut relaunched = base.clone();
            relaunched.complete_active_launch(7, "tab-1::manual-relaunch-7");
            assert_eq!(relaunched.active_count(), 1, "manual relaunch is active");
            assert_eq!(
                relaunched.inbox_item(7).map(|item| item.state),
                Some(MonitorInboxState::Launched)
            );

            if agent_failure {
                relaunched.record_agent_issue_failed(7, "fresh manual agent failure");
            } else {
                relaunched.record_launch_failed(7, "fresh manual launch failure");
            }

            assert_eq!(
                relaunched.inbox_item(7).map(|item| item.state),
                Some(expected_state),
                "new launch tracking distinguishes a fresh failure from receipt replay"
            );
            assert_eq!(relaunched.active_count(), 0);
            assert!(
                relaunched
                    .prefs()
                    .failed_issues
                    .iter()
                    .any(|failed| failed.issue_number == 7),
                "fresh manual relaunch failure is persisted"
            );
        }
    }

    // SPEC-3431 T-023 (AS2 / FR-006): the PM's launch_now writes the target to
    // the head of priority_order and asks for a scan. This pins the two
    // properties that make it safe: the reordered issue is the one the very
    // next scan claims, and re-scanning (which an immediate ScanNow racing the
    // interval tick can cause) never yields a second launch for it.
    #[test]
    fn launch_now_priority_head_claims_the_target_once_across_rescans() {
        let prefs = IssueMonitorPrefs {
            // exactly what run_monitor_launch_now writes for issue 7
            priority_order: vec![7],
            ..IssueMonitorPrefs::default()
        };
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        monitor.set_gui_connected(true);
        scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(7), issue(9)],
            "2026-08-04T00:00:00Z",
        );

        assert_eq!(
            monitor.queued_issue_numbers().first().copied(),
            Some(7),
            "launch_now's priority head must be first in the scan queue"
        );

        let request = monitor
            .next_launch_request("2026-08-04T00:00:00Z")
            .expect("the head issue is claimed");
        assert_eq!(request.issue_number, 7);

        // A second scan (immediate ScanNow landing next to the interval tick)
        // must not re-queue the in-flight issue, and no further launch for it
        // may be claimed.
        scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(7), issue(9)],
            "2026-08-04T00:00:05Z",
        );
        assert!(
            !monitor.queued_issue_numbers().contains(&7),
            "an in-flight launch must not be re-queued by a rescan"
        );

        let mut claimed = vec![request.issue_number];
        while let Some(next) = monitor.next_launch_request("2026-08-04T00:00:05Z") {
            claimed.push(next.issue_number);
        }
        assert_eq!(
            claimed.iter().filter(|number| **number == 7).count(),
            1,
            "issue 7 must be claimed exactly once: {claimed:?}"
        );
    }

    #[test]
    fn launching_claims_survive_prefs_roundtrip_and_are_not_reclaimed() {
        // Issue #3222: a claimed-but-not-yet-acked launch (Launching, no window
        // yet) must survive the prefs roundtrip. The GUI rebuilds the monitor
        // from disk on every handler call, so an unpersisted claim was invisible
        // to the next handler / the launch ACK's rescan, which re-claimed the
        // same issue (same-owner renewal) and spawned a DUPLICATE agent window
        // (observed live: Max 5 ⇒ 10 windows).
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-07-02T00:00:00Z");
        monitor.set_gui_connected(true);
        let request = monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("claimed for launch");
        assert_eq!(request.issue_number, 42);
        assert_eq!(monitor.active_count(), 1, "claim holds an active slot");

        // Reload from prefs (what every GUI handler does).
        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        restored.set_gui_connected(true);
        assert_eq!(
            restored.active_count(),
            1,
            "in-flight Launching claim survives the roundtrip"
        );
        // A rescan must not re-queue the in-flight issue…
        scan_issue_monitor_candidates(&mut restored, &[issue(42)], "2026-07-02T00:00:30Z");
        assert_eq!(
            restored.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Launching),
            "rescan shows the in-flight claim as Launching, not Queued"
        );
        assert_eq!(restored.queue_len(), 0, "not re-queued");
        // …and must not hand out a second launch request for it.
        assert!(
            restored
                .next_launch_request("2026-07-02T00:00:00Z")
                .is_none(),
            "no duplicate launch request for an in-flight claim"
        );
    }

    #[test]
    fn launching_prefs_accept_legacy_bare_ids_and_timestamped_entries() {
        // #3223 follow-up (codex P2): the launching entries gained a
        // `claimed_at` anchor. Files written by the first shipped shape (bare
        // ids) must still parse — a parse failure would `unwrap_or_default()`
        // into a FULL prefs wipe.
        let legacy = r#"{"enabled":true,"max_active_agents":2,"priority_order":[],"launching_issues":[42,43]}"#;
        let prefs: IssueMonitorPrefs = serde_json::from_str(legacy).expect("legacy parses");
        assert_eq!(
            prefs
                .launching_issues
                .iter()
                .map(|entry| entry.issue_number)
                .collect::<Vec<_>>(),
            vec![42, 43]
        );
        let timed = r#"{"enabled":true,"max_active_agents":2,"priority_order":[],"launching_issues":[{"issue_number":7,"claimed_at":"2026-07-02T00:00:00Z"}]}"#;
        let prefs: IssueMonitorPrefs = serde_json::from_str(timed).expect("timed parses");
        assert_eq!(prefs.launching_issues[0].issue_number, 7);
        assert_eq!(
            prefs.launching_issues[0].claimed_at.as_deref(),
            Some("2026-07-02T00:00:00Z")
        );
    }

    #[test]
    fn qualified_window_ids_require_the_same_project_tab_prefix() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                launched_issues: vec![IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "project-a::agent-1".to_string(),
                }],
                ..IssueMonitorPrefs::default()
            },
        );

        assert_eq!(
            monitor.launched_window_issue("project-a::agent-1"),
            Some(42),
            "the exact qualified id still matches"
        );
        assert_eq!(
            monitor.launched_window_issue("agent-1"),
            Some(42),
            "a legacy bare id remains compatible"
        );
        assert_eq!(
            monitor.launched_window_issue("project-b::agent-1"),
            None,
            "the same raw id in another project tab must not match"
        );
        assert_eq!(
            monitor.record_agent_window_failed("project-b::agent-1", "foreign failure"),
            None,
            "failure reverse lookup must not consume another project tab's launch"
        );
        assert_eq!(
            monitor.requeue_window("project-b::agent-1"),
            None,
            "close reverse lookup must not consume another project tab's launch"
        );
        assert_eq!(monitor.active_count(), 1);
    }

    #[test]
    fn live_candidate_snapshot_prunes_repo_absent_inflight_launches_but_cache_does_not() {
        let stale_prefs = IssueMonitorPrefs {
            // Both in-flight entries must fit under the cap: Issue #3627 makes
            // restore drop persisted launches beyond `max_active`, and this
            // fixture is about snapshot provenance, not slot accounting.
            max_active_agents: 2,
            launched_issues: vec![IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "project-a::agent-1".to_string(),
            }],
            launching_issues: vec![IssueMonitorLaunchingIssue {
                issue_number: 43,
                claimed_at: Some("2026-07-02T00:00:00Z".to_string()),
            }],
            ..IssueMonitorPrefs::default()
        };
        let project_root = Path::new(".");

        let mut live =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), stale_prefs.clone());
        scan_issue_monitor_candidates_with_provenance(
            &mut live,
            &[issue(99)],
            IssueMonitorCandidateSource::Live,
            project_root,
            "2026-07-28T00:00:00Z",
        );
        assert_eq!(
            live.active_count(),
            0,
            "a complete live snapshot frees both slots"
        );
        assert!(live.prefs().launched_issues.is_empty());
        assert!(live.prefs().launching_issues.is_empty());

        let mut cache = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), stale_prefs);
        scan_issue_monitor_candidates_with_provenance(
            &mut cache,
            &[issue(99)],
            IssueMonitorCandidateSource::Cache,
            project_root,
            "2026-07-28T00:00:00Z",
        );
        assert_eq!(
            cache.active_count(),
            2,
            "a partial cache snapshot must never authorize destructive pruning"
        );
    }

    #[test]
    fn live_gui_snapshot_prunes_foreign_qualified_windows_after_disk_rebase() {
        let stale_prefs = IssueMonitorPrefs {
            // Both launches must fit under the cap: Issue #3627 makes restore
            // drop persisted launches beyond `max_active`, and this fixture is
            // about window provenance, not slot accounting.
            max_active_agents: 2,
            launched_issues: vec![
                IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "project-b::agent-1".to_string(),
                },
                IssueMonitorLaunchedIssue {
                    issue_number: 43,
                    window_id: "legacy-agent-1".to_string(),
                },
            ],
            ..IssueMonitorPrefs::default()
        };
        let issues = [issue(42), issue(43)];
        let project_root = Path::new(".");
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), stale_prefs.clone());

        scan_issue_monitor_candidates_for_project_tab_with_provenance(
            &mut monitor,
            &issues,
            IssueMonitorCandidateSource::Live,
            project_root,
            Some("project-a"),
            "2026-07-28T00:00:00Z",
        );
        assert_eq!(monitor.launched_window_issue("project-b::agent-1"), None);
        assert_eq!(
            monitor.launched_window_issue("legacy-agent-1"),
            Some(43),
            "bare legacy ids have no positive foreign-project provenance"
        );

        monitor.rebase_gui_observer_prefs(&stale_prefs);
        scan_issue_monitor_candidates_for_project_tab_with_provenance(
            &mut monitor,
            &issues,
            IssueMonitorCandidateSource::Live,
            project_root,
            Some("project-a"),
            "2026-07-28T00:00:01Z",
        );
        assert_eq!(
            monitor.launched_window_issue("project-b::agent-1"),
            None,
            "the final post-rebase scan prevents union merge resurrection"
        );
        assert_eq!(monitor.active_count(), 1);

        let mut cache = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), stale_prefs);
        scan_issue_monitor_candidates_for_project_tab_with_provenance(
            &mut cache,
            &issues,
            IssueMonitorCandidateSource::Cache,
            project_root,
            Some("project-a"),
            "2026-07-28T00:00:02Z",
        );
        assert_eq!(
            cache.launched_window_issue("project-b::agent-1"),
            Some(42),
            "cache provenance cannot authorize foreign-prefix pruning"
        );
    }

    #[test]
    fn stale_unbound_launching_claims_expire_after_claim_ttl() {
        // #3223 follow-up (codex P2 / coderabbit): a crash between the
        // claim-save and the launch ACK leaves a restored `Launching` claim
        // with no window. Without an expiry it holds a max-active slot
        // forever. After claim_ttl_secs it must be released so the next scan
        // can re-queue and relaunch the issue.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-07-02T00:00:00Z");
        monitor.set_gui_connected(true);
        assert!(monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .is_some());
        assert_eq!(monitor.active_count(), 1);

        // Restart (roundtrip) mid-launch: claim restored, still unbound.
        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        restored.set_gui_connected(true);
        assert_eq!(restored.active_count(), 1);

        // Before the TTL: retained.
        let expired = restored.expire_stale_unbound_launches("2026-07-02T00:10:00Z");
        assert!(expired.is_empty(), "not expired before claim_ttl_secs");
        assert_eq!(restored.active_count(), 1);

        // After the TTL (default 1800s): released and re-queueable.
        let expired = restored.expire_stale_unbound_launches("2026-07-02T00:31:00Z");
        assert_eq!(expired, vec![42], "stale unbound claim expires");
        assert_eq!(restored.active_count(), 0, "slot released");
        scan_issue_monitor_candidates(&mut restored, &[issue(42)], "2026-07-02T00:31:10Z");
        assert!(
            restored
                .next_launch_request("2026-07-02T00:31:20Z")
                .is_some(),
            "the issue is claimable again after expiry"
        );
        // Bound launches never expire this way.
        monitor.complete_active_launch(42, "tab-1::agent-1");
        assert!(monitor
            .expire_stale_unbound_launches("2026-07-03T00:00:00Z")
            .is_empty());
    }

    #[test]
    fn merge_inflight_launches_from_disk_unifies_cross_process_accounting() {
        // #3223 follow-up (codex P1): the daemon only refreshed profile/tuning
        // from disk, so GUI-written launching/launched claims were invisible —
        // the daemon saw free slots (over-cap claims) and its next persist
        // dropped the GUI's in-flight entries. The merge must absorb both.
        let mut daemon = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 2,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 2,
                ..IssueMonitorPrefs::default()
            },
        );
        let disk = IssueMonitorPrefs {
            launching_issues: vec![IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: Some("2026-07-02T00:00:00Z".to_string()),
            }],
            launched_issues: vec![IssueMonitorLaunchedIssue {
                issue_number: 43,
                window_id: "tab-1::agent-2".to_string(),
            }],
            ..IssueMonitorPrefs::default()
        };
        daemon.merge_inflight_launches_from_disk(&disk);
        assert_eq!(daemon.active_count(), 2, "both in-flight claims absorbed");
        assert!(daemon.launched_window_issue("tab-1::agent-2").is_some());
        // The daemon persist now round-trips them instead of dropping them.
        let prefs = daemon.prefs();
        assert!(prefs
            .launching_issues
            .iter()
            .any(|entry| entry.issue_number == 42
                && entry.claimed_at.as_deref() == Some("2026-07-02T00:00:00Z")));
        assert!(prefs
            .launched_issues
            .iter()
            .any(|entry| entry.issue_number == 43));
    }

    #[test]
    fn merge_from_disk_keeps_stale_claim_away_from_replaced_window() {
        // PR #3787 review: a delivery completion can clear the local claim
        // while retaining the (newer) window. A later disk rebase must not
        // pair that window with the stale claim of the previous generation —
        // the imported claim would invalidate the exact-identity fence used
        // by live_claim_id / stop_only / failover_restart / requeue.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs::default(),
        );
        monitor.complete_active_launch(42, "tab-1::agent-new");
        let disk = IssueMonitorPrefs {
            launched_issues: vec![IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-old".to_string(),
            }],
            launched_claims: BTreeMap::from([(42, "claim-old".to_string())]),
            ..IssueMonitorPrefs::default()
        };
        monitor.merge_inflight_launches_from_disk(&disk);
        assert_eq!(
            monitor.live_claim_id(42),
            None,
            "a stale disk claim must not attach to a replaced window"
        );
        assert_eq!(monitor.launched_window_issue("tab-1::agent-new"), Some(42));

        // Control: with no local window the disk row imports window and claim.
        let mut fresh = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs::default(),
        );
        fresh.merge_inflight_launches_from_disk(&disk);
        assert_eq!(fresh.live_claim_id(42).as_deref(), Some("claim-old"));
        assert_eq!(fresh.launched_window_issue("tab-1::agent-old"), Some(42));
    }

    #[test]
    fn cross_process_rebase_unions_equal_marker_failures_with_terminal_precedence() {
        let local_failure = IssueMonitorFailedIssue {
            issue_number: 45,
            message: "local explicit failure".to_string(),
            window_id: None,
        };
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                merged_issues: vec![44],
                failed_issues: vec![local_failure.clone()],
                ..IssueMonitorPrefs::default()
            },
        );
        let profile = IssueMonitorLaunchProfile {
            agent_id: "claude".to_string(),
            model: Some("sonnet".to_string()),
            reasoning: None,
            version: None,
            session_mode: Default::default(),
            skip_permissions: false,
            codex_fast_mode: false,
            runtime_target: Default::default(),
            docker_service: None,
            docker_lifecycle_intent: Default::default(),
            windows_shell: None,
        };
        let autonomous_record = |issue_number, phase| AutonomousIssueRecord {
            issue_number,
            phase,
            active_launch_id: None,
            attempts: 2,
            acceptance_snapshot: None,
            retry_not_before: None,
            retry_hold_reason: None,
            retry_hold_provider: None,
            last_heartbeat: None,
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
            wait: None,
            needs_human_kind: None,
            steering: None,
        };
        let disk = IssueMonitorPrefs {
            launch_profile: Some(profile.clone()),
            launched_issues: vec![IssueMonitorLaunchedIssue {
                issue_number: 43,
                window_id: "tab-1::agent-43".to_string(),
            }],
            launching_issues: vec![IssueMonitorLaunchingIssue {
                issue_number: 100,
                claimed_at: Some("2026-07-21T00:00:00Z".to_string()),
            }],
            failed_issues: vec![
                IssueMonitorFailedIssue {
                    issue_number: 43,
                    message: "failure cannot beat a real launch".to_string(),
                    window_id: None,
                },
                IssueMonitorFailedIssue {
                    issue_number: 44,
                    message: "failure cannot beat merged state".to_string(),
                    window_id: None,
                },
                IssueMonitorFailedIssue {
                    issue_number: 45,
                    message: "disk cannot replace a local explicit failure".to_string(),
                    window_id: Some("tab-1::agent-45".to_string()),
                },
                IssueMonitorFailedIssue {
                    issue_number: 99,
                    message: "disk-only unrelated failure".to_string(),
                    window_id: Some("tab-1::agent-99".to_string()),
                },
                IssueMonitorFailedIssue {
                    issue_number: 100,
                    message: "failure beats an unbound claim".to_string(),
                    window_id: None,
                },
            ],
            autonomous_records: vec![
                autonomous_record(43, AutonomousPhase::NeedsHuman),
                autonomous_record(44, AutonomousPhase::NeedsHuman),
                autonomous_record(45, AutonomousPhase::NeedsHuman),
                autonomous_record(100, AutonomousPhase::Implementing),
            ],
            autonomous_tuning: AutonomousTuning {
                max_attempts: 9,
                ..AutonomousTuning::default()
            },
            ..IssueMonitorPrefs::default()
        };

        monitor.rebase_daemon_driver_prefs(&disk);

        let prefs = monitor.prefs();
        assert_eq!(prefs.launch_profile, Some(profile));
        assert_eq!(prefs.autonomous_tuning.max_attempts, 9);
        assert_eq!(monitor.launched_window_issue("tab-1::agent-43"), Some(43));
        assert!(prefs
            .failed_issues
            .iter()
            .all(|failed| failed.issue_number != 43 && failed.issue_number != 44));
        assert_eq!(
            prefs
                .failed_issues
                .iter()
                .find(|failed| failed.issue_number == 45),
            Some(&local_failure)
        );
        assert_eq!(
            prefs
                .failed_issues
                .iter()
                .find(|failed| failed.issue_number == 99)
                .map(|failed| (failed.message.as_str(), failed.window_id.as_deref())),
            Some(("disk-only unrelated failure", Some("tab-1::agent-99")))
        );
        assert_eq!(
            prefs
                .failed_issues
                .iter()
                .find(|failed| failed.issue_number == 100)
                .map(|failed| failed.message.as_str()),
            Some("failure beats an unbound claim")
        );
        assert!(prefs
            .launching_issues
            .iter()
            .all(|launching| launching.issue_number != 100));
        assert_eq!(
            prefs.autonomous_records,
            vec![autonomous_record(100, AutonomousPhase::Implementing)],
            "disk-only records are absorbed, but rejected terminal failure companions are not"
        );
    }

    #[test]
    fn cross_process_rebase_keeps_needs_human_record_with_disk_only_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("issue-monitor.json");
        let failure = IssueMonitorFailedIssue {
            issue_number: 99,
            message: "human review required".to_string(),
            window_id: Some("tab-1::agent-99".to_string()),
        };
        let needs_human = AutonomousIssueRecord {
            issue_number: 99,
            phase: AutonomousPhase::NeedsHuman,
            active_launch_id: None,
            attempts: 6,
            acceptance_snapshot: None,
            retry_not_before: None,
            retry_hold_reason: None,
            retry_hold_provider: None,
            last_heartbeat: Some("2026-07-20T00:00:00Z".to_string()),
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
            wait: None,
            needs_human_kind: None,
            steering: None,
        };
        save_issue_monitor_prefs(
            &path,
            &IssueMonitorPrefs {
                failed_issues: vec![failure.clone()],
                autonomous_records: vec![needs_human.clone()],
                ..IssueMonitorPrefs::default()
            },
        )
        .expect("seed equal-marker NeedsHuman state");
        let mut stale = IssueMonitorState::new(IssueMonitorConfig::default());

        mutate_issue_monitor_prefs(&path, |disk| {
            stale.rebase_daemon_driver_prefs(disk);
            *disk = stale.prefs();
        })
        .expect("rebase and save stale writer");

        let persisted = load_issue_monitor_prefs(&path).expect("reload committed prefs");
        assert_eq!(persisted.failed_issues, vec![failure]);
        assert_eq!(persisted.autonomous_records, vec![needs_human]);
        let mut restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), persisted);
        restored.record_candidate(issue(99));
        assert_eq!(
            restored.inbox_item(99).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
    }

    #[test]
    fn older_marker_rebase_rejects_terminal_companion_for_local_failure() {
        let local_failure = IssueMonitorFailedIssue {
            issue_number: 45,
            message: "local current failure".to_string(),
            window_id: None,
        };
        let mut current = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                failed_issues: vec![local_failure.clone()],
                ..IssueMonitorPrefs::default()
            },
        );
        let older = IssueMonitorPrefs {
            legacy_git_launch_failure_migration_version: 0,
            failed_issues: vec![IssueMonitorFailedIssue {
                issue_number: 45,
                message: "older disk failure".to_string(),
                window_id: None,
            }],
            autonomous_records: vec![AutonomousIssueRecord {
                issue_number: 45,
                phase: AutonomousPhase::NeedsHuman,
                active_launch_id: None,
                attempts: 6,
                acceptance_snapshot: None,
                retry_not_before: None,
                retry_hold_reason: None,
                retry_hold_provider: None,
                last_heartbeat: None,
                pr_number: None,
                reviewed_sha: None,
                review_passed: None,
                wait: None,
                needs_human_kind: None,
                steering: None,
            }],
            ..IssueMonitorPrefs::default()
        };

        let mut gui = current.clone();
        gui.rebase_gui_observer_prefs(&older);
        current.rebase_daemon_driver_prefs(&older);

        for monitor in [&mut gui, &mut current] {
            let prefs = monitor.prefs();
            assert_eq!(prefs.failed_issues, vec![local_failure.clone()]);
            assert!(
                prefs.autonomous_records.is_empty(),
                "an older terminal companion cannot change a current local failure"
            );
            monitor.record_candidate(issue(45));
            assert_eq!(
                monitor.inbox_item(45).map(|item| item.state),
                Some(MonitorInboxState::AgentFailed)
            );
        }
    }

    #[test]
    fn older_marker_rebase_keeps_unrelated_failure_but_not_migrated_fingerprint() {
        let project_root = Path::new("/tmp/gwt-issue-3314-older-marker");
        let migrated_message = format!(
            "{LEGACY_GIT_LAUNCH_FAILURE_PREFIX}{}",
            project_root.display()
        );
        let mut migrated = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                legacy_git_launch_failure_migration_version: 0,
                failed_issues: vec![IssueMonitorFailedIssue {
                    issue_number: 43,
                    message: migrated_message.clone(),
                    window_id: None,
                }],
                ..IssueMonitorPrefs::default()
            },
        );
        migrated.apply_legacy_git_launch_failure_migration(project_root);
        assert!(migrated.prefs().failed_issues.is_empty());

        let unrelated_failure = IssueMonitorFailedIssue {
            issue_number: 99,
            message: "unrelated fresh failure".to_string(),
            window_id: Some("tab-1::agent-99".to_string()),
        };
        let needs_human = AutonomousIssueRecord {
            issue_number: 99,
            phase: AutonomousPhase::NeedsHuman,
            active_launch_id: None,
            attempts: 4,
            acceptance_snapshot: None,
            retry_not_before: None,
            retry_hold_reason: None,
            retry_hold_provider: None,
            last_heartbeat: None,
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
            wait: None,
            needs_human_kind: None,
            steering: None,
        };
        let older_disk = IssueMonitorPrefs {
            legacy_git_launch_failure_migration_version: 0,
            failed_issues: vec![
                IssueMonitorFailedIssue {
                    issue_number: 43,
                    message: migrated_message,
                    window_id: None,
                },
                unrelated_failure.clone(),
            ],
            autonomous_records: vec![needs_human.clone()],
            ..IssueMonitorPrefs::default()
        };

        let mut gui = migrated.clone();
        gui.rebase_gui_observer_prefs(&older_disk);
        let mut daemon = migrated;
        daemon.rebase_daemon_driver_prefs(&older_disk);

        for prefs in [gui.prefs(), daemon.prefs()] {
            assert_eq!(
                prefs.legacy_git_launch_failure_migration_version,
                LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
            );
            assert_eq!(prefs.failed_issues, vec![unrelated_failure.clone()]);
            assert_eq!(prefs.autonomous_records, vec![needs_human.clone()]);
            assert!(
                prefs
                    .failed_issues
                    .iter()
                    .all(|failure| failure.issue_number != 43),
                "the exact failure removed by the local migration stays removed"
            );
        }
    }

    #[test]
    fn disk_merged_rebase_silently_clears_same_issue_nonterminal_state() {
        let autonomous_record = |issue_number| AutonomousIssueRecord {
            issue_number,
            phase: AutonomousPhase::Implementing,
            active_launch_id: Some(format!("launch-{issue_number}")),
            attempts: 1,
            acceptance_snapshot: None,
            retry_not_before: None,
            retry_hold_reason: None,
            retry_hold_provider: None,
            last_heartbeat: None,
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
            wait: None,
            needs_human_kind: None,
            steering: None,
        };
        let mut stale = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 5,
                launched_issues: vec![IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-42".to_string(),
                }],
                launching_issues: vec![IssueMonitorLaunchingIssue {
                    issue_number: 43,
                    claimed_at: Some("2026-07-21T00:00:00Z".to_string()),
                }],
                failed_issues: vec![IssueMonitorFailedIssue {
                    issue_number: 44,
                    message: "stale failure".to_string(),
                    window_id: Some("tab-1::agent-44".to_string()),
                }],
                merged_issues: vec![77],
                autonomous_mode: true,
                autonomous_records: vec![
                    autonomous_record(42),
                    autonomous_record(43),
                    autonomous_record(44),
                    autonomous_record(45),
                ],
                ..IssueMonitorPrefs::default()
            },
        );
        stale.set_gui_connected(true);
        scan_issue_monitor_candidates(
            &mut stale,
            &[issue(42), issue(43), issue(44), issue(45)],
            "2026-07-21T00:01:00Z",
        );
        stale
            .next_launch_request("2026-07-21T00:02:00Z")
            .expect("issue 45 pending launch");
        stale.push_review_dispatch(AutonomousReviewDispatch {
            issue_number: 42,
            pr_number: 420,
            reviewed_sha: "merged-sha".to_string(),
            required_criteria: vec!["AC-1".to_string()],
            diff: "stale review diff".to_string(),
            linked_issue_kind: LinkedIssueKind::Issue,
        });
        let disk = IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 5,
            merged_issues: vec![42, 43, 44, 45, 88],
            autonomous_mode: true,
            ..IssueMonitorPrefs::default()
        };

        let mut gui = stale.clone();
        gui.rebase_gui_observer_prefs(&disk);
        let mut daemon = stale;
        daemon.rebase_daemon_driver_prefs(&disk);

        for monitor in [&mut gui, &mut daemon] {
            let prefs = monitor.prefs();
            assert_eq!(prefs.merged_issues, vec![42, 43, 44, 45, 77, 88]);
            assert!(prefs.launched_issues.is_empty());
            assert!(prefs.launching_issues.is_empty());
            assert!(prefs.failed_issues.is_empty());
            assert!(prefs.autonomous_records.is_empty());
            assert_eq!(monitor.active_count(), 0);
            assert!(monitor.take_pending_launch_requests().is_empty());
            assert!(
                monitor.take_pending_review_dispatches().is_empty(),
                "a merged Issue cannot spawn a queued stale review agent"
            );
            assert_eq!(monitor.status_view().last_error, None);
            assert!(monitor.take_autonomous_notices().is_empty());
            for issue_number in [42, 43, 44, 45] {
                assert_eq!(
                    monitor.inbox_item(issue_number).map(|item| item.state),
                    Some(MonitorInboxState::Merged)
                );
            }
        }
    }

    #[test]
    fn terminal_delivery_deletion_survives_stale_disk_rebase() {
        for terminal_state in [
            MonitorInboxState::Merged,
            MonitorInboxState::Released,
            MonitorInboxState::LaunchFailed,
            MonitorInboxState::AgentFailed,
            MonitorInboxState::NeedsHuman,
        ] {
            let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            });
            monitor.record_candidate(issue(42));
            assert!(monitor.apply_confirmed_claim(
                42,
                "claim-42",
                "host/session",
                "effect-42",
                "2026-07-28T00:00:00Z",
            ));
            let stale_disk = monitor.prefs();

            monitor.clear_active_tracking(42);
            monitor.set_inbox_state(42, terminal_state);
            assert!(monitor.prefs().pending_launch_deliveries.is_empty());

            monitor.rebase_daemon_driver_prefs(&stale_disk);

            assert!(
                monitor.prefs().pending_launch_deliveries.is_empty(),
                "a stale disk outbox cannot revive a delivery deleted by local {terminal_state:?}"
            );
            assert!(monitor.take_pending_launch_requests().is_empty());
            assert_eq!(
                monitor.inbox_item(42).map(|item| item.state),
                Some(terminal_state)
            );
        }
    }

    #[test]
    fn daemon_rebase_updates_profile_and_tuning_from_disk() {
        // Issue #3222: the daemon loads prefs once at startup, so a launch
        // profile the GUI saves later stays invisible (has_launch_profile=false
        // ⇒ cap 0 ⇒ the daemon never refills slots and the GUI's re-entrant
        // scan became the de-facto driver). The scan must refresh GUI-owned
        // fields from disk.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert!(!monitor.has_launch_profile());
        let disk = IssueMonitorPrefs {
            launch_profile: Some(IssueMonitorLaunchProfile {
                agent_id: "claude".to_string(),
                model: None,
                reasoning: None,
                version: None,
                session_mode: Default::default(),
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: Default::default(),
                docker_service: None,
                docker_lifecycle_intent: Default::default(),
                windows_shell: None,
            }),
            autonomous_tuning: AutonomousTuning {
                max_attempts: 9,
                ..AutonomousTuning::default()
            },
            ..IssueMonitorPrefs::default()
        };
        monitor.rebase_daemon_driver_prefs(&disk);
        assert!(monitor.has_launch_profile(), "profile refreshed from disk");
        assert_eq!(monitor.autonomous_tuning.max_attempts, 9);
    }

    #[test]
    fn complete_spec_merge_frees_slot_marks_done_and_is_not_requeued() {
        let complete_spec = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithCompletedTasks,
            "2026-06-26T00:00:00Z",
        );
        let mut monitor = launched_monitor_for_issue(complete_spec.clone(), "tab-1::agent-1");
        monitor.record_merged(42);
        assert_eq!(monitor.active_count(), 0, "Merged frees the active slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
        // A later scan must keep a completed SPEC Merged (not re-queued).
        scan_issue_monitor_candidates(&mut monitor, &[complete_spec], "2026-06-26T01:00:00Z");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
        assert_eq!(monitor.queue_len(), 0);
        assert_eq!(monitor.active_count(), 0);
    }

    #[test]
    fn requeue_window_returns_unmerged_issue_to_pending() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let requeued = monitor.requeue_window("tab-1::agent-1");
        assert_eq!(requeued, Some(42));
        assert_eq!(monitor.active_count(), 0, "closing frees the slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "closing an unmerged window returns to pending, never a fake done state"
        );
        assert_eq!(monitor.queue_len(), 1);
    }

    #[test]
    fn delayed_exact_window_close_cannot_requeue_a_same_id_successor() {
        let mut predecessor = launched_monitor(42, "tab-1::agent-1");
        predecessor
            .launched_claims
            .insert(42, "claim-predecessor".to_string());
        let stale_close = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: Some("claim-predecessor".to_string()),
            delivery_id: None,
            window_id: Some("tab-1::agent-1".to_string()),
        };

        let durable = predecessor.prefs();
        assert_eq!(
            durable.launched_claims.get(&42).map(String::as_str),
            Some("claim-predecessor"),
            "the close generation must survive the prefs roundtrip"
        );
        let mut successor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), durable);
        successor
            .launched_claims
            .insert(42, "claim-successor".to_string());

        assert_eq!(successor.requeue_exact_window(&stale_close), None);
        assert_eq!(successor.active_count(), 1, "the successor keeps its slot");
        assert_eq!(
            successor.launched_window_issue("tab-1::agent-1"),
            Some(42),
            "the successor binding remains live"
        );

        let current_close = IssueMonitorStopTarget {
            claim_id: Some("claim-successor".to_string()),
            ..stale_close
        };
        assert_eq!(successor.requeue_exact_window(&current_close), Some(42));
    }

    /// SPEC-3431 FR-069: the reset instant to hold a launch until, when the
    /// agent's own provider is out of quota.
    #[test]
    fn rate_limit_reset_is_resolved_from_the_agents_own_provider() {
        use gwt_core::usage::{ProviderUsage, UsageProvider, UsageState, UsageWindow, WindowKind};

        let exhausted = ProviderUsage {
            provider: UsageProvider::Codex,
            account_label: None,
            plan: None,
            windows: vec![UsageWindow::new(
                WindowKind::Weekly,
                100.0,
                Some("2026-08-10T14:26:00Z".parse().expect("reset")),
            )],
            limit_reached: true,
            state: UsageState::Ok,
            fetched_at: None,
        };
        let healthy = ProviderUsage {
            provider: UsageProvider::ClaudeCode,
            limit_reached: false,
            ..exhausted.clone()
        };
        let accounts = vec![exhausted, healthy];

        assert_eq!(
            rate_limit_reset_for_agent("codex", &accounts).as_deref(),
            Some("2026-08-10T14:26:00Z"),
            "the exhausted provider's reset must be used as the floor"
        );
        // An agent on a different provider is unaffected — one account running
        // out must not stall the whole fleet.
        assert_eq!(rate_limit_reset_for_agent("claude", &accounts), None);
        assert_eq!(rate_limit_reset_for_agent("gemini", &accounts), None);
    }

    /// SPEC-3431 FR-069: a rate-limited launch releases its slot.
    ///
    /// This is the one stall whose cause is known for certain — the provider
    /// itself reports `limit_reached` with a reset instant — so it is the one
    /// stall the mechanism resolves instead of merely reporting. Waiting is
    /// pointless when the reset is days away (observed live: "try again at Aug
    /// 10th", four days out) and the held slot stops the whole queue at the
    /// default `max_active = 1`.
    ///
    /// The issue returns to the queue rather than failing: nothing is wrong
    /// with the work. The reset instant becomes the backoff floor so the next
    /// scan cannot immediately relaunch into the same wall.
    #[test]
    fn rate_limited_launch_releases_its_slot_until_the_reset() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        assert_eq!(monitor.active_count(), 1, "precondition: holding a slot");

        let released = monitor.release_rate_limited_launch(
            "tab-1::agent-1",
            "2026-08-10T14:26:00Z",
            "2026-08-07T00:00:00Z",
        );

        assert_eq!(released, Some(42));
        assert_eq!(monitor.active_count(), 0, "the slot must be freed");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "the work is fine; only the provider was unavailable"
        );
        assert!(
            !monitor.retry_ready(42, "2026-08-09T00:00:00Z"),
            "must not relaunch into the same wall before the reset"
        );
        assert!(
            monitor.retry_ready(42, "2026-08-10T15:00:00Z"),
            "must be relaunchable once the provider recovers"
        );
    }

    /// SPEC-3431 FR-069: a stall whose cause is *not* known keeps its slot.
    ///
    /// Approval prompts, rate limits, and genuine hangs are indistinguishable
    /// from the snapshot, so reclaiming a slot on elapsed time alone would kill
    /// agents that are simply thinking for a long time. The mechanism reports;
    /// the PM decides.
    #[test]
    fn an_unexplained_stall_keeps_its_slot() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_autonomous_heartbeat(42, "2026-08-01T00:00:00Z");

        let status = monitor.agent_status();

        assert_eq!(
            status.active_launches,
            vec![42],
            "an unexplained stall must not be auto-reclaimed"
        );
        assert_eq!(
            status
                .inbox
                .iter()
                .find(|row| row.issue_number == 42)
                .and_then(|row| row.last_activity_at.clone()),
            Some("2026-08-01T00:00:00Z".to_string()),
            "but it must be visible so the PM can act on it"
        );
    }

    /// Issue #3616 AC-2/AC-6: a provider quota block is held, never failed.
    ///
    /// `record_agent_window_failed` is the path this used to take, and it is
    /// terminal (`AgentFailed`) plus attempt-consuming. Nothing about the work
    /// failed, so the hold keeps the issue queueable and leaves the retry
    /// budget alone.
    #[test]
    fn a_provider_quota_block_holds_the_issue_without_spending_an_attempt() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let attempts_before = monitor
            .autonomous_record(42)
            .map(|record| record.attempts)
            .unwrap_or_default();

        let outcome = monitor.try_hold_provider_usage_limit(
            42,
            "tab-1::agent-1",
            "Codex",
            "Codex usage limit reached — resumes after 2026-08-22T03:46:00Z",
            Some("2026-08-22T03:46:00Z"),
            None,
            "2026-08-16T02:26:00Z",
        );

        assert_eq!(outcome, IssueMonitorProviderUsageLimitOutcome::Held);
        assert_eq!(monitor.active_count(), 0, "the slot must be freed");
        assert!(
            !monitor
                .inbox_item(42)
                .map(|item| item.state)
                .is_some_and(MonitorInboxState::is_terminal),
            "a quota block must not be terminal"
        );
        assert_eq!(
            monitor
                .autonomous_record(42)
                .map(|record| record.attempts)
                .unwrap_or_default(),
            attempts_before,
            "the account ran out, not the retry budget"
        );
        assert!(!monitor.retry_ready(42, "2026-08-20T00:00:00Z"));
        assert!(monitor.retry_ready(42, "2026-08-22T04:00:00Z"));
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_hold_provider.as_deref()),
            Some("codex"),
            "the retry floor must retain the normalized provider identity"
        );
    }

    /// Issue #3785 / SPEC #3165 Scenario 114: an unparseable provider notice
    /// still needs a finite admission floor. Re-probing immediately recreates
    /// launch churn; blocking forever makes recovery manual.
    #[test]
    fn a_provider_quota_block_without_reset_uses_the_default_backoff() {
        for (case, resets_at) in [
            ("missing", None),
            ("invalid", Some("not-a-reset")),
            ("past", Some("2026-08-16T02:25:59Z")),
        ] {
            let mut monitor = launched_monitor(42, "tab-1::agent-1");

            assert_eq!(
                monitor.try_hold_provider_usage_limit(
                    42,
                    "tab-1::agent-1",
                    "codex",
                    "Codex usage limit reached",
                    resets_at,
                    None,
                    "2026-08-16T02:26:00Z",
                ),
                IssueMonitorProviderUsageLimitOutcome::Held,
                "{case} reset"
            );

            assert_eq!(
                monitor
                    .autonomous_record(42)
                    .and_then(|record| record.retry_not_before.as_deref()),
                Some("2026-08-16T02:27:00Z"),
                "{case} reset must use the exact now+60 fallback"
            );
            assert_eq!(
                monitor
                    .autonomous_record(42)
                    .and_then(|record| record.retry_hold_provider.as_deref()),
                Some("codex"),
                "{case} reset must retain its typed provider"
            );
            assert_eq!(
                monitor
                    .prefs()
                    .provider_quota_holds
                    .get("codex")
                    .map(String::as_str),
                Some("2026-08-16T02:27:00Z"),
                "{case} reset must persist the same concrete provider deadline"
            );
            assert!(!monitor.retry_ready(42, "2026-08-16T02:26:59Z"));
            assert!(monitor.retry_ready(42, "2026-08-16T02:27:00Z"));
            assert_eq!(monitor.attempt_count(42), 0);
        }
    }

    #[test]
    fn legacy_rate_limit_release_records_the_saved_profiles_provider_hold() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                launch_profile: Some(test_launch_profile("Codex")),
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(issue(42));
        monitor.complete_active_launch(42, "tab-1::agent-42");

        assert_eq!(
            monitor.release_rate_limited_launch(
                "tab-1::agent-42",
                "2026-08-22T04:00:00Z",
                "2026-08-22T03:00:00Z",
            ),
            Some(42)
        );
        assert_eq!(
            monitor.prefs().provider_quota_holds,
            BTreeMap::from([("codex".to_string(), "2026-08-22T04:00:00Z".to_string(),)])
        );
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_hold_provider.as_deref()),
            Some("codex"),
            "the legacy release path uses the saved profile as its typed provider"
        );
    }

    /// Issue #3616 AC-3/AC-4: the hold and its reset are readable from
    /// `issue.monitor.status`, so the PM never has to read a pane to tell a
    /// quota block from a hang.
    #[test]
    fn a_provider_quota_block_is_visible_in_the_agent_status_projection() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.try_hold_provider_usage_limit(
            42,
            "tab-1::agent-1",
            "codex",
            "Codex usage limit reached — resumes after 2026-08-22T03:46:00Z",
            Some("2026-08-22T03:46:00Z"),
            None,
            "2026-08-16T02:26:00Z",
        );

        let status = monitor.agent_status();
        let row = status
            .inbox
            .iter()
            .find(|row| row.issue_number == 42)
            .expect("the held issue stays in the inbox");

        assert!(
            !status.active_launches.contains(&42),
            "a held launch must not still read as active"
        );
        assert_ne!(
            row.state,
            MonitorInboxState::Launched,
            "reporting `launched` is what hid this for 23 minutes"
        );
        assert_eq!(
            row.retry_not_before.as_deref(),
            Some("2026-08-22T03:46:00Z")
        );
        assert_eq!(
            row.retry_hold_reason.as_deref(),
            Some("Codex usage limit reached — resumes after 2026-08-22T03:46:00Z")
        );
    }

    /// Issue #3785 / SPEC #3165 Scenario 116: the provider-wide admission
    /// decision is one structured top-level projection shared by CLI/offline
    /// readers and the GUI, rather than something callers infer from one row's
    /// display reason.
    #[test]
    fn a_provider_quota_hold_is_structured_in_agent_and_gui_status() {
        let profile = test_launch_profile("codex");
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(profile),
                ..IssueMonitorPrefs::default()
            },
        );
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-08-16T02:26:00Z");
        monitor.complete_active_launch(42, "tab-1::agent-1");
        assert_eq!(
            monitor.try_hold_provider_usage_limit(
                42,
                "tab-1::agent-1",
                "codex",
                "Codex usage limit reached — resumes after 2026-08-22T03:46:00Z",
                Some("2026-08-22T03:46:00Z"),
                None,
                "2026-08-16T02:26:00Z",
            ),
            IssueMonitorProviderUsageLimitOutcome::Held
        );

        let expected_hold = serde_json::json!({
            "provider": "codex",
            "reset_at": "2026-08-22T03:46:00Z",
            // Issue #3923 AC-2: the hold names what it was formed from.
            "evidence": {
                "recorded_at": "2026-08-16T02:26:00Z",
                "source": "unrecorded",
                "issue_number": 42,
            },
        });
        let agent_status = serde_json::to_value(monitor.agent_status_at("2026-08-16T02:26:30Z"))
            .expect("serialize agent status");
        let gui_status = serde_json::to_value(monitor.status_view_at("2026-08-16T02:26:30Z"))
            .expect("serialize GUI status");

        assert_eq!(agent_status.get("quota_hold"), Some(&expected_hold));
        assert_eq!(gui_status.get("quota_hold"), Some(&expected_hold));
        assert_eq!(
            gui_status.get("state").and_then(serde_json::Value::as_str),
            Some("quota_hold")
        );

        monitor.restore_persisted_last_scan_at(Some("2026-08-22T03:46:01Z".to_string()));
        let released_agent = serde_json::to_value(monitor.agent_status_at("2026-08-22T03:46:01Z"))
            .expect("serialize released agent status");
        let released_gui = serde_json::to_value(monitor.status_view_at("2026-08-22T03:46:01Z"))
            .expect("serialize released GUI status");
        assert!(released_agent.get("quota_hold").is_none());
        assert!(released_gui.get("quota_hold").is_none());
        assert_eq!(
            released_gui
                .get("state")
                .and_then(serde_json::Value::as_str),
            Some("idle")
        );
    }

    #[test]
    fn legacy_prefs_default_provider_quota_holds_to_empty() {
        let prefs: IssueMonitorPrefs =
            serde_json::from_str(r#"{"enabled":false,"max_active_agents":1,"priority_order":[]}"#)
                .expect("legacy prefs without provider holds");

        assert!(prefs.provider_quota_holds.is_empty());
        assert!(
            serde_json::to_value(prefs)
                .expect("serialize legacy-compatible prefs")
                .get("provider_quota_holds")
                .is_none(),
            "the additive empty map stays absent from compact prefs"
        );
    }

    #[test]
    fn retry_hold_provider_is_additive_and_roundtrips_when_present() {
        let legacy: AutonomousIssueRecord = serde_json::from_value(serde_json::json!({
            "issue_number": 42,
            "retry_not_before": "2026-08-22T04:00:00Z",
            "retry_hold_reason": "Codex quota exhausted"
        }))
        .expect("legacy autonomous record without a typed provider");

        assert_eq!(legacy.retry_hold_provider, None);
        assert!(
            serde_json::to_value(&legacy)
                .expect("serialize legacy-compatible autonomous record")
                .get("retry_hold_provider")
                .is_none(),
            "an absent typed provider stays absent from compact legacy data"
        );

        let mut typed = legacy;
        typed.retry_hold_provider = Some("codex".to_string());
        let value = serde_json::to_value(&typed).expect("serialize typed retry hold");
        assert_eq!(
            value
                .get("retry_hold_provider")
                .and_then(serde_json::Value::as_str),
            Some("codex")
        );
        assert_eq!(
            serde_json::from_value::<AutonomousIssueRecord>(value)
                .expect("deserialize typed retry hold"),
            typed
        );
    }

    #[test]
    fn provider_quota_hold_rebase_joins_the_later_deadline_in_both_orders() {
        let earlier = "2026-08-22T12:00:00+09:00";
        let later = "2026-08-22T04:00:00Z";

        for (local_deadline, disk_deadline) in [(earlier, later), (later, earlier)] {
            let mut monitor = IssueMonitorState::with_prefs(
                IssueMonitorConfig::default(),
                IssueMonitorPrefs {
                    provider_quota_holds: BTreeMap::from([(
                        "Codex".to_string(),
                        local_deadline.to_string(),
                    )]),
                    ..IssueMonitorPrefs::default()
                },
            );
            let disk = IssueMonitorPrefs {
                provider_quota_holds: BTreeMap::from([(
                    "CODEX".to_string(),
                    disk_deadline.to_string(),
                )]),
                ..IssueMonitorPrefs::default()
            };

            monitor.rebase_daemon_driver_prefs(&disk);

            assert_eq!(
                monitor.prefs().provider_quota_holds,
                BTreeMap::from([("codex".to_string(), later.to_string())])
            );
        }

        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                provider_quota_holds: BTreeMap::from([("codex".to_string(), later.to_string())]),
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.rebase_gui_observer_prefs(&IssueMonitorPrefs::default());
        assert_eq!(
            monitor.prefs().provider_quota_holds.get("codex"),
            Some(&later.to_string()),
            "an empty older disk snapshot cannot erase the local hold"
        );
    }

    #[test]
    fn a_healthy_saved_profile_bypasses_another_providers_hold() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                launch_profile: Some(test_launch_profile("ClaudeCode")),
                provider_quota_holds: BTreeMap::from([(
                    "codex".to_string(),
                    "2026-08-22T04:00:00Z".to_string(),
                )]),
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);
        let candidate = auto_issue(42, "## Acceptance Criteria\n- [ ] AC-1: x\n");
        monitor.record_candidate(candidate.clone());
        let record = monitor.autonomous_record_mut(42);
        record.retry_not_before = Some("2026-08-22T04:00:00Z".to_string());
        record.retry_hold_reason = None;
        record.retry_hold_provider = Some("codex".to_string());
        let protection = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };

        assert_eq!(
            monitor.prepare_autonomous_candidate(&candidate, &protection, "2026-08-22T03:00:00Z",),
            EligibilityDecision::Eligible,
            "the scan eligibility gate must honor the explicit healthy-provider switch"
        );
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_hold_provider.as_deref()),
            None,
            "launch admission consumes the typed retry hold with its deadline"
        );

        assert_eq!(
            monitor
                .try_prepare_claim_effects_with_probe(
                    "host/session",
                    "2026-08-22T03:00:00Z",
                    1,
                    |_| Ok::<bool, std::convert::Infallible>(false),
                )
                .expect("infallible probe"),
            1
        );
        assert_eq!(
            monitor.status_view_at("2026-08-22T03:00:00Z").quota_hold,
            None
        );
        assert_eq!(
            monitor.agent_status_at("2026-08-22T03:00:00Z").quota_hold,
            None
        );
    }

    #[test]
    fn legacy_quota_retry_without_a_durable_provider_hold_stays_gated() {
        let mut record = AutonomousIssueRecord::new(42);
        record.retry_not_before = Some("2026-08-22T04:00:00Z".to_string());
        record.retry_hold_reason = Some("Codex quota exhausted".to_string());
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                launch_profile: Some(test_launch_profile("codex")),
                autonomous_records: vec![record],
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);
        let candidate = auto_issue(42, "## Acceptance Criteria\n- [ ] AC-1: x\n");
        monitor.record_candidate(candidate.clone());
        let protection = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };

        assert_eq!(
            monitor.prepare_autonomous_candidate(&candidate, &protection, "2026-08-22T03:00:00Z"),
            EligibilityDecision::HumanGate("retry backoff window not elapsed".to_string()),
            "legacy quota metadata without a durable provider identity cannot authorize a bypass"
        );
        assert_eq!(
            monitor
                .try_prepare_claim_effects_with_probe(
                    "host/session",
                    "2026-08-22T03:00:00Z",
                    1,
                    |_| Ok::<bool, std::convert::Infallible>(false),
                )
                .expect("infallible probe"),
            0
        );
        assert!(monitor.pending_effects().is_empty());
    }

    #[test]
    fn ordinary_retry_does_not_bypass_a_coincident_other_provider_hold() {
        let deadline = "2026-08-22T04:00:00Z";
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                launch_profile: Some(test_launch_profile("claude")),
                provider_quota_holds: BTreeMap::from([("codex".to_string(), deadline.to_string())]),
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);
        let candidate = auto_issue(42, "## Acceptance Criteria\n- [ ] AC-1: x\n");
        monitor.record_candidate(candidate.clone());
        let record = monitor.autonomous_record_mut(42);
        record.retry_not_before = Some(deadline.to_string());
        record.retry_hold_reason = None;
        record.retry_hold_provider = None;
        let protection = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };

        assert_eq!(
            monitor.prepare_autonomous_candidate(&candidate, &protection, "2026-08-22T03:00:00Z"),
            EligibilityDecision::HumanGate("retry backoff window not elapsed".to_string()),
            "an ordinary transient retry must not inherit provider identity from a matching deadline"
        );
        assert_eq!(
            monitor
                .try_prepare_claim_effects_with_probe(
                    "host/session",
                    "2026-08-22T03:00:00Z",
                    1,
                    |_| Ok::<bool, std::convert::Infallible>(false),
                )
                .expect("infallible probe"),
            0
        );
        assert!(monitor.pending_effects().is_empty());
    }

    #[test]
    fn a_saved_profile_provider_hold_gates_every_preclaim_entrypoint() {
        use gwt_github::FakeIssueClient;

        let mut base = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(test_launch_profile("codex")),
                provider_quota_holds: BTreeMap::from([(
                    "codex".to_string(),
                    "2026-08-22T04:00:00Z".to_string(),
                )]),
                ..IssueMonitorPrefs::default()
            },
        );
        base.set_gui_connected(true);
        base.record_candidate(issue(42));
        let now = "2026-08-22T03:00:00Z";

        let mut legacy = base.clone();
        assert_eq!(legacy.next_launch_request(now), None);
        assert_eq!(legacy.queued_issue_numbers(), vec![42]);

        let mut durable = base.clone();
        assert_eq!(
            durable
                .try_prepare_claim_effects_with_probe("host/session", now, 1, |_| Ok::<
                    bool,
                    std::convert::Infallible,
                >(
                    false
                ),)
                .expect("infallible probe"),
            0
        );
        assert!(durable.pending_effects().is_empty());
        assert_eq!(durable.queued_issue_numbers(), vec![42]);

        let mut synchronous = base;
        assert!(synchronous
            .claim_next_launch_requests_with_probe(
                &FakeIssueClient::new(),
                "host/session",
                now,
                1,
                |_| false,
            )
            .is_empty());
        assert_eq!(synchronous.active_count(), 0);
        assert_eq!(synchronous.queued_issue_numbers(), vec![42]);

        let mut no_profile = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                provider_quota_holds: BTreeMap::from([(
                    "codex".to_string(),
                    "2026-08-22T04:00:00Z".to_string(),
                )]),
                ..IssueMonitorPrefs::default()
            },
        );
        no_profile.set_gui_connected(true);
        no_profile.record_candidate(issue(43));
        assert_eq!(
            no_profile
                .try_prepare_claim_effects_with_probe("host/session", now, 1, |_| Ok::<
                    bool,
                    std::convert::Infallible,
                >(
                    false
                ),)
                .expect("infallible probe"),
            1,
            "without a saved profile no provider can be selected for a global gate"
        );
    }

    #[test]
    fn provider_hold_compensates_prepared_attempting_and_unclaimed_work_exactly() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 8,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 8,
                launch_profile: Some(test_launch_profile("codex")),
                ..IssueMonitorPrefs::default()
            },
        );
        for number in [42, 43, 44, 45] {
            monitor.record_candidate(issue(number));
        }
        monitor.complete_active_launch(42, "tab-1::agent-42");
        monitor
            .prepare_pending_effect(
                "claim-prepared",
                IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 43,
                    claim_id: "claim-43".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-08-22T02:00:00Z".to_string(),
                    expires_at: "2026-08-22T02:30:00Z".to_string(),
                    launched_work_id: None,
                },
            )
            .expect("prepared claim");
        let attempting = monitor
            .prepare_pending_effect(
                "claim-attempting",
                IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 44,
                    claim_id: "claim-44".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-08-22T02:00:00Z".to_string(),
                    expires_at: "2026-08-22T02:30:00Z".to_string(),
                    launched_work_id: None,
                },
            )
            .expect("attempting claim");
        assert!(monitor.mark_pending_effect_attempting(&attempting));
        monitor
            .prepare_pending_effect(
                "arm-pr-45",
                IssueMonitorEffectPayload::ArmAutoMerge {
                    issue_number: 45,
                    pr_number: 145,
                    reviewed_sha: "sha-45".to_string(),
                },
            )
            .expect("arm proposal");
        assert!(monitor.apply_confirmed_claim(
            45,
            "claim-45",
            "host/session",
            "claim-confirmed-45",
            "2026-08-22T02:00:00Z",
        ));
        let epoch = monitor.effect_authority_epoch();

        assert_eq!(
            monitor.try_hold_provider_usage_limit(
                42,
                "tab-1::agent-42",
                "Codex",
                "Codex quota exhausted",
                Some("2026-08-22T04:00:00Z"),
                None,
                "2026-08-22T02:01:00Z",
            ),
            IssueMonitorProviderUsageLimitOutcome::Held
        );

        assert_eq!(monitor.effect_authority_epoch(), epoch);
        assert!(!monitor.pending_effects().iter().any(|effect| {
            effect.state == IssueMonitorEffectState::Prepared
                && matches!(
                    effect.payload,
                    IssueMonitorEffectPayload::AcquireClaim { .. }
                )
        }));
        assert!(monitor.pending_effects().iter().any(|effect| {
            effect.effect_id == "claim-attempting"
                && effect.state == IssueMonitorEffectState::Attempting
        }));
        for (issue_number, claim_id) in [(44, "claim-44"), (45, "claim-45")] {
            assert!(monitor.pending_effects().iter().any(|effect| matches!(
                &effect.payload,
                IssueMonitorEffectPayload::ReleaseClaim {
                    issue_number: pending_issue,
                    claim_id: pending_claim,
                    owner,
                } if *pending_issue == issue_number
                    && pending_claim == claim_id
                    && owner == "host/session"
            )));
        }
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            effect.payload,
            IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 45,
                ..
            }
        )));
        assert_eq!(monitor.pending_launch_delivery_id(45), None);
        assert_eq!(
            monitor
                .inbox_item(45)
                .map(|item| (item.state, item.claim_id.clone())),
            Some((MonitorInboxState::Queued, None))
        );
        assert!(monitor.queued_issue_numbers().contains(&45));
        assert!(!monitor.active_issue_numbers().contains(&45));
        assert_eq!(monitor.attempt_count(42), 0);
    }

    #[test]
    fn provider_hold_preserves_materializer_owned_and_confirmed_launches() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 8,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 8,
                launch_profile: Some(test_launch_profile("codex")),
                ..IssueMonitorPrefs::default()
            },
        );
        for number in [42, 43, 44] {
            monitor.record_candidate(issue(number));
        }
        monitor.complete_active_launch(42, "tab-1::agent-42");
        assert!(monitor.apply_confirmed_claim(
            43,
            "claim-43",
            "host/session",
            "claim-effect-43",
            "2026-08-22T02:00:00Z",
        ));
        let delivery_43 = monitor.pending_launch_delivery_id(43).expect("delivery 43");
        assert!(monitor.claim_launch_delivery(
            43,
            &delivery_43,
            "materializer-43",
            43,
            "tab-1::materializing-43",
            |_| false,
        ));
        assert!(monitor.apply_confirmed_claim(
            44,
            "claim-44",
            "host/session",
            "claim-effect-44",
            "2026-08-22T02:00:00Z",
        ));
        let delivery_44 = monitor.pending_launch_delivery_id(44).expect("delivery 44");
        assert!(monitor.claim_launch_delivery(
            44,
            &delivery_44,
            "materializer-44",
            44,
            "tab-1::agent-44",
            |_| false,
        ));
        assert!(monitor.mark_launch_delivery_materialized(
            44,
            &delivery_44,
            "materializer-44",
            "tab-1::agent-44",
        ));
        assert!(monitor.mark_launch_delivery_workspace_durable(
            44,
            &delivery_44,
            "materializer-44",
            "tab-1::agent-44",
        ));
        assert!(monitor.complete_active_launch_delivery(44, "tab-1::agent-44", Some(&delivery_44),));

        monitor.try_hold_provider_usage_limit(
            42,
            "tab-1::agent-42",
            "codex",
            "Codex quota exhausted",
            Some("2026-08-22T04:00:00Z"),
            None,
            "2026-08-22T02:01:00Z",
        );

        assert_eq!(monitor.pending_launch_delivery_id(43), Some(delivery_43));
        assert!(monitor.active_issue_numbers().contains(&43));
        assert_eq!(
            monitor.launched_window_id(44).as_deref(),
            Some("tab-1::agent-44")
        );
        assert_eq!(monitor.live_claim_id(44).as_deref(), Some("claim-44"));
        assert!(monitor.active_issue_numbers().contains(&44));
        assert_eq!(
            monitor.status_view_at("2026-08-22T02:02:00Z").state,
            "quota_hold",
            "an active matching-provider hold outranks preserved in-flight launch projections"
        );
    }

    #[test]
    fn provider_hold_restores_fresh_required_from_an_unclaimed_delivery() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 4,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 4,
                launch_profile: Some(test_launch_profile("codex")),
                ..IssueMonitorPrefs::default()
            },
        );
        for number in [42, 43] {
            monitor.record_candidate(issue(number));
        }
        monitor.complete_active_launch(42, "tab-1::agent-42");
        monitor.require_fresh_launch_session(43);
        assert!(monitor.apply_confirmed_claim(
            43,
            "claim-43",
            "host/session",
            "claim-effect-43",
            "2026-08-22T02:00:00Z",
        ));
        assert_eq!(
            monitor.prefs().pending_launch_deliveries[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired
        );

        monitor.try_hold_provider_usage_limit(
            42,
            "tab-1::agent-42",
            "codex",
            "Codex quota exhausted",
            Some("2026-08-22T04:00:00Z"),
            None,
            "2026-08-22T02:01:00Z",
        );

        assert_eq!(
            monitor.prefs().queued_launch_session_strategies.get(&43),
            Some(&IssueMonitorLaunchSessionStrategy::FreshRequired)
        );
    }

    #[test]
    fn prepare_effect_rejects_a_stale_claim_proposal_for_the_held_provider() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                launch_profile: Some(test_launch_profile("codex")),
                provider_quota_holds: BTreeMap::from([(
                    "codex".to_string(),
                    "2026-08-22T04:00:00Z".to_string(),
                )]),
                ..IssueMonitorPrefs::default()
            },
        );
        let payload = IssueMonitorEffectPayload::AcquireClaim {
            issue_number: 42,
            claim_id: "claim-42".to_string(),
            owner: "host/session".to_string(),
            heartbeat_at: "2026-08-22T03:00:00Z".to_string(),
            expires_at: "2026-08-22T03:30:00Z".to_string(),
            launched_work_id: None,
        };

        assert!(!monitor.prepare_effect(PendingIssueMonitorEffect::prepared(
            "stale-scan-proposal",
            monitor.effect_authority_epoch(),
            payload.clone(),
        )));
        assert_eq!(
            monitor.prepare_pending_effect("stale-direct-proposal", payload),
            None
        );
        assert!(monitor.pending_effects().is_empty());

        assert!(monitor
            .prepare_pending_effect(
                "post-reset-proposal",
                IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 42,
                    claim_id: "claim-42-after-reset".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-08-22T04:00:01Z".to_string(),
                    expires_at: "2026-08-22T04:30:01Z".to_string(),
                    launched_work_id: None,
                },
            )
            .is_some());
    }

    /// Issue #3616: the same source-identity rule the writer-conflict recovery
    /// enforces. An issue number alone must not let a stale window revoke a
    /// newer launch.
    #[test]
    fn a_provider_quota_block_from_a_stale_window_is_rejected() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");

        let outcome = monitor.try_hold_provider_usage_limit(
            42,
            "tab-1::agent-superseded",
            "codex",
            "Codex usage limit reached",
            None,
            None,
            "2026-08-16T02:26:00Z",
        );

        assert_eq!(outcome, IssueMonitorProviderUsageLimitOutcome::Rejected);
        assert_eq!(
            monitor.active_count(),
            1,
            "the live launch keeps its slot when a stale window reports"
        );
    }

    /// Issue #3616 AC-5: a PM that switched launch profile must be able to run
    /// the work now on a healthy provider instead of waiting out someone
    /// else's billing cycle.
    #[test]
    fn clearing_the_hold_lets_the_pm_relaunch_before_the_reset() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.try_hold_provider_usage_limit(
            42,
            "tab-1::agent-1",
            "codex",
            "Codex usage limit reached",
            Some("2026-08-22T03:46:00Z"),
            None,
            "2026-08-16T02:26:00Z",
        );
        assert!(!monitor.retry_ready(42, "2026-08-16T02:30:00Z"));

        assert!(monitor.clear_retry_hold(42));

        assert!(monitor.retry_ready(42, "2026-08-16T02:30:00Z"));
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_hold_provider.as_deref()),
            None,
            "an explicit clear must remove the typed provider with the retry floor"
        );
        assert_eq!(
            monitor
                .agent_status()
                .inbox
                .iter()
                .find(|row| row.issue_number == 42)
                .and_then(|row| row.retry_hold_reason.clone()),
            None,
            "an overridden hold must not keep advertising a reset that no longer gates anything"
        );
    }

    /// Issue #3616: an ordinary transient retry must not inherit a quota
    /// reason. The reason describes why *this* backoff exists.
    #[test]
    fn a_later_transient_backoff_replaces_the_quota_hold_reason() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.try_hold_provider_usage_limit(
            42,
            "tab-1::agent-1",
            "codex",
            "Codex usage limit reached",
            Some("2026-08-22T03:46:00Z"),
            None,
            "2026-08-16T02:26:00Z",
        );

        monitor.complete_active_launch(42, "tab-1::agent-2");
        monitor.record_autonomous_failure(42, "boom", "2026-08-16T03:00:00Z");

        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_hold_reason.clone()),
            None,
            "a transient failure backoff is not a provider quota hold"
        );
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_hold_provider.as_deref()),
            None,
            "an ordinary transient retry must clear the previous quota provider"
        );
    }

    /// Issue #3627: a launch whose agent window is gone from the owning tab's
    /// canvas no longer holds a slot.
    ///
    /// Release used to be driven only by PTY exit events, so a window that
    /// disappeared without one held its slot forever. Window existence is the
    /// judgement source precisely because it cannot be confused by a reused
    /// pid: no pid is read at all.
    #[test]
    fn a_launch_whose_agent_window_is_gone_releases_its_slot() {
        let monitor = launched_monitor(42, "project-a::agent-24");

        let live = ["agent-51".to_string(), "agent-52".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>();

        assert_eq!(
            monitor.vanished_launched_windows("project-a", &live),
            vec!["project-a::agent-24".to_string()],
            "a window the tab no longer has cannot be holding an agent"
        );
    }

    /// Issue #3627 (companion to `an_unexplained_stall_keeps_its_slot`): the
    /// reclaim must separate "the host is provably gone" from "we cannot
    /// explain the silence". A window that still exists keeps its slot however
    /// long it has been quiet.
    #[test]
    fn a_silent_agent_whose_window_still_exists_keeps_its_slot() {
        let mut monitor = launched_monitor(42, "project-a::agent-24");
        monitor.record_autonomous_heartbeat(42, "2026-08-01T00:00:00Z");

        let live = ["agent-24".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>();

        assert!(
            monitor
                .vanished_launched_windows("project-a", &live)
                .is_empty(),
            "an unexplained stall is reported, never auto-reclaimed"
        );
        assert_eq!(monitor.active_count(), 1);
    }

    /// Issue #3627: a tab can only judge the windows it issued. Another tab's
    /// qualified window (including another gwt instance's) and a legacy bare id
    /// have no trustworthy provenance here, so both are retained.
    #[test]
    fn only_the_owning_tabs_windows_are_judged() {
        let foreign = launched_monitor(42, "project-b::agent-24");
        assert!(
            foreign
                .vanished_launched_windows("project-a", &BTreeSet::new())
                .is_empty(),
            "another tab's window is invisible from here, not dead"
        );

        let legacy = launched_monitor(42, "agent-24");
        assert!(
            legacy
                .vanished_launched_windows("project-a", &BTreeSet::new())
                .is_empty(),
            "a bare legacy id proves no ownership"
        );
    }

    /// Issue #3627 AC-1: the reclaim is slot accounting, not an autonomous
    /// pipeline concern. `stuck_autonomous_issues` is gated on
    /// `autonomous_mode`, which is why a human-gated deployment could sit
    /// wedged for 30 hours with nobody reclaiming anything.
    #[test]
    fn window_liveness_reclaim_ignores_autonomous_mode() {
        for autonomous_mode in [false, true] {
            let mut monitor = launched_monitor(42, "project-a::agent-24");
            monitor.set_autonomous_mode(autonomous_mode);

            assert_eq!(
                monitor.vanished_launched_windows("project-a", &BTreeSet::new()),
                vec!["project-a::agent-24".to_string()],
                "autonomous_mode={autonomous_mode} must not gate slot accounting"
            );
        }
    }

    /// Issue #3627 AC-2/AC-6: the reported wedge — 11 launched entries against
    /// `max_active` 5, none of whose windows still exist — recovers completely,
    /// and the restore that used to resurrect it no longer over-fills the cap.
    #[test]
    fn a_restart_that_lost_every_agent_window_frees_every_slot() {
        let prefs = IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 5,
            launched_issues: (1..=11)
                .map(|issue_number| IssueMonitorLaunchedIssue {
                    issue_number,
                    window_id: format!("project-a::agent-{issue_number}"),
                })
                .collect(),
            ..IssueMonitorPrefs::default()
        };

        let mut restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);

        assert_eq!(
            restored.active_count(),
            5,
            "restore must not re-inject 11 launches into a 5-slot cap"
        );
        assert_eq!(
            restored.prefs().launched_issues.len(),
            5,
            "the dropped excess must not stay bound to a slot it does not hold"
        );

        let vanished = restored.vanished_launched_windows("project-a", &BTreeSet::new());
        assert_eq!(
            vanished.len(),
            5,
            "every surviving window is gone after the restart"
        );
        for window_id in vanished {
            restored.requeue_window_at(&window_id, "2026-08-17T09:00:00Z");
        }

        assert_eq!(restored.active_count(), 0, "every slot is free again");
        assert!(
            restored.prefs().launched_issues.is_empty(),
            "and the wedge cannot be restored from disk a second time"
        );
    }

    /// Issue #3627 AC-3: a bound launch never takes a slot away from a
    /// claimed-but-unbound one. Dropping an in-flight claim would erase it from
    /// disk and let the next rescan launch a duplicate (Issue #3222).
    #[test]
    fn restore_caps_bound_launches_without_dropping_in_flight_claims() {
        let prefs = IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 2,
            launched_issues: (1..=4)
                .map(|issue_number| IssueMonitorLaunchedIssue {
                    issue_number,
                    window_id: format!("project-a::agent-{issue_number}"),
                })
                .collect(),
            launching_issues: vec![IssueMonitorLaunchingIssue {
                issue_number: 99,
                claimed_at: Some("2026-08-17T09:00:00Z".to_string()),
            }],
            ..IssueMonitorPrefs::default()
        };

        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);

        assert!(
            restored.active_count() <= 2,
            "the restored slot accounting must respect max_active, got {}",
            restored.active_count()
        );
        assert!(
            restored.active_issue_numbers().contains(&99),
            "the unbound claim keeps its slot so it cannot be re-claimed"
        );
        assert_eq!(
            restored.prefs().launching_issues.len(),
            1,
            "and it survives the prefs roundtrip"
        );
    }

    /// Issue #3627 AC-5: OFF/ON is the operator's manual escape hatch from a
    /// wedged queue. It used to recover nothing, because `prefs()` rebuilds
    /// `launched_issues` from `launched_windows`, which the disable left alone.
    #[test]
    fn disabling_the_monitor_leaves_no_launched_slot_behind() {
        let mut monitor = launched_monitor(42, "project-a::agent-24");

        monitor.set_enabled(false);

        assert_eq!(monitor.active_count(), 0);
        assert!(
            monitor.prefs().launched_issues.is_empty(),
            "a disabled monitor must not persist slots it no longer manages"
        );

        let restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        assert_eq!(
            restored.active_count(),
            0,
            "and the next reload must not resurrect them"
        );
    }

    /// SPEC-3431 FR-068: a launched issue reports when its agent last showed
    /// signs of life, so "running but not progressing" is observable at all.
    ///
    /// Everything a live agent can get stuck on — waiting for approval, a
    /// provider rate limit, a genuine hang — leaves `WindowProcessStatus`
    /// untouched, because gwt receives no event at all. The composed state
    /// collapses to `window_hook_states`' two values (`Idle` / `Running`),
    /// which only say whether the last hook was tool-ish or a Stop. Without a
    /// timestamp those four situations are indistinguishable from healthy
    /// work, which is why a rate-limited agent showed `● Running` for days.
    ///
    /// Hook arrivals are the right clock: `PreToolUse` / `PostToolUse` /
    /// `UserPromptSubmit` each mean one unit of work actually happened. PTY
    /// status cannot serve — its watcher thread `continue`s while the process
    /// runs and only speaks when it exits, so `last_heartbeat` never advanced
    /// past the value seeded at launch.
    #[test]
    fn launched_issue_reports_its_last_activity() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let launched_at = monitor
            .agent_status()
            .inbox
            .iter()
            .find(|row| row.issue_number == 42)
            .and_then(|row| row.last_activity_at.clone())
            .expect("a launched issue starts its activity clock");

        monitor.record_autonomous_heartbeat(42, "2026-08-07T09:00:00Z");

        let after = monitor
            .agent_status()
            .inbox
            .iter()
            .find(|row| row.issue_number == 42)
            .and_then(|row| row.last_activity_at.clone())
            .expect("still reported");
        assert_ne!(
            after, launched_at,
            "a hook arrival must advance the activity clock"
        );
        assert_eq!(after, "2026-08-07T09:00:00Z");
    }

    /// SPEC-3431 FR-066: closing a launched window is a bounded retry.
    ///
    /// Requeueing consumed no attempt and set no backoff, so with autonomous
    /// mode on, closing a pane put the issue straight back at the head of the
    /// queue and the next scan relaunched it — an unbounded loop. That is why
    /// the PM was told not to close panes, but the constraint protected
    /// nobody: the only callers are the `WindowClosed` control, so **a person
    /// closing a window by hand hits exactly the same loop**.
    ///
    /// Bound it where the loop is instead. The attempt ladder, backoff, and
    /// `NeedsHuman` escalation already exist for autonomous failures; a close
    /// is a failed attempt by the same definition ("closed without the work
    /// completing"), so it uses the same budget.
    #[test]
    fn requeue_window_consumes_an_attempt_and_stops_at_the_budget() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let max = monitor.autonomous_tuning.max_attempts;
        assert!(max >= 2, "precondition: a real budget");

        // The first close still returns the issue to the queue, but now with a
        // backoff floor so the next scan cannot relaunch it instantly.
        assert_eq!(monitor.requeue_window("tab-1::agent-1"), Some(42));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert!(
            !monitor.retry_ready(42, "2026-08-07T00:00:00Z"),
            "an immediate relaunch is what made this a loop"
        );

        // Exhausting the budget hands the issue to a human instead of looping.
        for _ in 1..max {
            monitor.complete_active_launch(42, "tab-1::agent-1");
            monitor.requeue_window("tab-1::agent-1");
        }
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman),
            "after {max} closes the issue must stop being relaunched"
        );
    }

    /// Issue #3503: cleaning up a pane whose launch failed before its PTY
    /// started is not a retry. The failure already went through the failure
    /// path, which detached the window, so the later close has no launch to
    /// requeue and charges nothing. Without this, the diagnostic cleanup the
    /// PM performs would spend a second attempt on one failure and walk the
    /// issue to `NeedsHuman` early.
    #[test]
    fn closing_a_launch_failed_pane_consumes_no_retry_attempt() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_launch_failed(42, "Launch failed before PTY started.");
        let attempts_before = monitor
            .autonomous_record(42)
            .map(|record| record.attempts)
            .unwrap_or_default();

        assert_eq!(
            monitor.requeue_window("tab-1::agent-1"),
            None,
            "a launch-failed pane has no live launch to requeue"
        );
        assert_eq!(
            monitor
                .autonomous_record(42)
                .map(|record| record.attempts)
                .unwrap_or_default(),
            attempts_before,
            "closing the leftover error pane must not consume a retry attempt"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::LaunchFailed),
            "the close must not overwrite the recorded failure with a fresh queue entry"
        );
    }

    /// SPEC-3431 FR-033 / T-086: the exact identity a stop_only request must
    /// carry for a launched issue in [`launched_monitor`].
    fn stop_target(monitor: &IssueMonitorState, issue_number: u64) -> IssueMonitorStopTarget {
        IssueMonitorStopTarget {
            issue_number,
            claim_id: monitor
                .inbox_item(issue_number)
                .and_then(|item| item.claim_id.clone()),
            delivery_id: monitor.pending_launch_delivery_id(issue_number),
            window_id: monitor.launched_window_id(issue_number),
        }
    }

    /// SPEC-3431 FR-033: a stop_only request revokes exactly one launch.
    ///
    /// Unlike [`IssueMonitorState::requeue_window_at`] (FR-066), this is not a
    /// retry: it consumes no attempt, does not requeue, and does not relaunch.
    /// It is the operation the PM uses when it means "stop", as opposed to
    /// closing a pane, which means "this attempt failed, try again".
    #[test]
    fn stop_only_revokes_the_slot_and_holds_the_issue() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = stop_target(&monitor, 42);
        let epoch_before = monitor.effect_authority_epoch();
        let attempts_before = monitor.attempt_count(42);

        assert_eq!(
            monitor.stop_only(&target, "rate limit", "2026-08-07T00:00:00Z"),
            IssueMonitorStopOutcome::Stopped {
                window_id: "tab-1::agent-1".to_string()
            }
        );

        assert_eq!(monitor.active_count(), 0, "the slot must be released");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman),
            "a stop is terminal/held, not a queued retry"
        );
        assert!(
            !monitor.queued_issue_numbers().contains(&42),
            "stop_only must not requeue"
        );
        assert!(
            monitor.effect_authority_epoch() > epoch_before,
            "effect authority must be revoked so in-flight effects cannot land"
        );
        assert_eq!(
            monitor.attempt_count(42),
            attempts_before,
            "a deliberate stop is not a failed attempt and must not spend the retry budget"
        );
        assert!(
            monitor
                .inbox_item(42)
                .and_then(|item| item.error_message.clone())
                .is_some_and(|message| message.contains("rate limit")),
            "FR-031: the reason must be diagnosable from the snapshot"
        );
    }

    /// SPEC-3431 FR-033: every identity component must match exactly, and a
    /// mismatch closes nothing. A stale notification or a concurrent scan is
    /// exactly how the wrong agent gets killed.
    #[test]
    fn stop_only_fails_closed_on_any_identity_mismatch() {
        let base = launched_monitor(42, "tab-1::agent-1");
        let good = stop_target(&base, 42);

        let mismatches: Vec<(IssueMonitorStopTarget, IssueMonitorStopMismatch)> = vec![
            (
                IssueMonitorStopTarget {
                    issue_number: 99,
                    ..good.clone()
                },
                IssueMonitorStopMismatch::UnknownIssue,
            ),
            (
                IssueMonitorStopTarget {
                    window_id: Some("tab-1::agent-2".to_string()),
                    ..good.clone()
                },
                IssueMonitorStopMismatch::WindowMismatch,
            ),
            (
                IssueMonitorStopTarget {
                    window_id: None,
                    ..good.clone()
                },
                IssueMonitorStopMismatch::WindowMismatch,
            ),
            (
                IssueMonitorStopTarget {
                    claim_id: Some("foreign-claim".to_string()),
                    ..good.clone()
                },
                IssueMonitorStopMismatch::ClaimMismatch,
            ),
            (
                IssueMonitorStopTarget {
                    delivery_id: Some("stale-delivery".to_string()),
                    ..good.clone()
                },
                IssueMonitorStopMismatch::DeliveryMismatch,
            ),
        ];

        for (target, expected) in mismatches {
            let mut monitor = base.clone();
            assert_eq!(
                monitor.stop_only(&target, "switch provider", "2026-08-07T00:00:00Z"),
                IssueMonitorStopOutcome::Mismatch(expected),
                "target {target:?} must fail closed"
            );
            assert_eq!(
                monitor.active_count(),
                1,
                "a rejected stop must not release the slot"
            );
            assert_eq!(
                monitor.inbox_item(42).map(|item| item.state),
                Some(MonitorInboxState::Launched),
                "a rejected stop must leave the launch running"
            );
            assert_eq!(
                monitor.launched_window_id(42).as_deref(),
                Some("tab-1::agent-1"),
                "a rejected stop must not detach the window"
            );
        }
    }

    /// SPEC-3431 FR-033: only a running or ready-to-run launch can be stopped.
    #[test]
    fn stop_only_rejects_issues_that_are_not_running() {
        let mut queued = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut queued, &[issue(42)], "2026-06-26T00:00:00Z");
        assert_eq!(
            queued.stop_only(
                &IssueMonitorStopTarget {
                    issue_number: 42,
                    claim_id: None,
                    delivery_id: None,
                    window_id: None,
                },
                "stop",
                "2026-08-07T00:00:00Z"
            ),
            IssueMonitorStopOutcome::Mismatch(IssueMonitorStopMismatch::NotRunning)
        );

        let mut merged = launched_monitor_for_issue(
            spec_issue(
                42,
                IssueMonitorReadiness::ReadyWithCompletedTasks,
                "2026-08-07T00:00:00Z",
            ),
            "tab-1::agent-1",
        );
        let target = stop_target(&merged, 42);
        merged.record_merged(42);
        assert_eq!(
            merged.stop_only(&target, "stop", "2026-08-07T00:00:00Z"),
            IssueMonitorStopOutcome::Mismatch(IssueMonitorStopMismatch::NotRunning),
            "merged work is already terminal and must not be reopened as stopped"
        );
        assert_eq!(
            merged.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
    }

    /// SPEC-3431 FR-033: repeating the request is idempotent — it neither
    /// errors nor revokes authority a second time.
    #[test]
    fn stop_only_is_idempotent_when_repeated() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = stop_target(&monitor, 42);
        assert!(matches!(
            monitor.stop_only(&target, "stop", "2026-08-07T00:00:00Z"),
            IssueMonitorStopOutcome::Stopped { .. }
        ));
        let epoch_after_first = monitor.effect_authority_epoch();

        assert_eq!(
            monitor.stop_only(&target, "stop", "2026-08-07T00:01:00Z"),
            IssueMonitorStopOutcome::AlreadyStopped
        );
        assert_eq!(
            monitor.effect_authority_epoch(),
            epoch_after_first,
            "a repeat must not burn another authority epoch"
        );
        assert_eq!(monitor.active_count(), 0);
    }

    /// SPEC-3431 FR-033: the `window_closed` that arrives after the stop must
    /// not requeue or relaunch the issue. This is the race the PM hits every
    /// time it stops an agent: the GUI reaps the pane a moment later.
    #[test]
    fn stop_only_makes_a_delayed_window_closed_inert() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = stop_target(&monitor, 42);
        monitor.stop_only(&target, "stop", "2026-08-07T00:00:00Z");

        assert_eq!(
            monitor.requeue_window_at("tab-1::agent-1", "2026-08-07T00:00:05Z"),
            None,
            "the delayed close must not resurrect a stopped issue"
        );
        assert!(!monitor.queued_issue_numbers().contains(&42));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
    }

    /// SPEC-3431 FR-033 / T-087b: stop_only must work in the process that
    /// actually runs it.
    ///
    /// `gwtd` is short-lived: it loads prefs and has no inbox, because
    /// [`IssueMonitorState::with_prefs`] restores the durable launch
    /// accounting but not the scanned candidate rows. An implementation that
    /// reads liveness off the inbox therefore answers `UnknownIssue` for every
    /// running agent when the PM calls it — the unit matrix passes and the
    /// operation is dead on arrival.
    ///
    /// The identity that survives the process boundary is the durable one:
    /// active launches, launched windows, pending deliveries, failed issues.
    #[test]
    fn stop_only_works_from_prefs_without_a_scanned_inbox() {
        let launched = launched_monitor(42, "tab-1::agent-1");
        // Exactly what gwtd sees: prefs on disk, no candidate scan.
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), launched.prefs());
        assert!(
            monitor.inbox_item(42).is_none(),
            "precondition: with_prefs restores no inbox"
        );
        assert_eq!(monitor.active_count(), 1, "precondition: the slot is held");

        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        assert_eq!(
            monitor.stop_only(&target, "rate limit", "2026-08-07T00:00:00Z"),
            IssueMonitorStopOutcome::Stopped {
                window_id: "tab-1::agent-1".to_string()
            }
        );
        assert_eq!(monitor.active_count(), 0, "the slot must be released");

        // The hold has to survive the next prefs roundtrip, or the following
        // scan relaunches the issue the PM just stopped.
        let reloaded =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        assert_eq!(reloaded.active_count(), 0);
        assert_eq!(
            reloaded.stop_only_reason(42).as_deref(),
            Some("rate limit"),
            "the stop must be durable and diagnosable after a reload"
        );

        // And a mismatch must still fail closed with no inbox to lean on.
        let mut fresh =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), launched.prefs());
        assert_eq!(
            fresh.stop_only(
                &IssueMonitorStopTarget {
                    window_id: Some("tab-1::agent-2".to_string()),
                    ..target.clone()
                },
                "rate limit",
                "2026-08-07T00:00:00Z"
            ),
            IssueMonitorStopOutcome::Mismatch(IssueMonitorStopMismatch::WindowMismatch)
        );
        assert_eq!(fresh.active_count(), 1);
    }

    /// SPEC-3431 FR-033 / T-087c: the running daemon must converge on a stop
    /// that another process committed.
    ///
    /// `gwtd` writes the stop to prefs, but the daemon is already holding the
    /// same launch in memory and rebases its state onto disk before each scan.
    /// If that rebase keeps the local record, the daemon still believes the
    /// agent is running, keeps the slot, and writes its own view back over the
    /// stop on the next commit — the PM's stop silently un-does itself.
    #[test]
    fn a_stop_committed_by_another_process_survives_the_daemon_rebase() {
        // The daemon: an autonomous launch in flight.
        let mut daemon = launched_monitor(42, "tab-1::agent-1");
        daemon.set_autonomous_phase(42, AutonomousPhase::Implementing);
        daemon.record_autonomous_heartbeat(42, "2026-08-07T00:00:00Z");
        assert_eq!(daemon.active_count(), 1);

        // The PM's gwtd process: same prefs, no inbox, commits the stop.
        let mut cli = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), daemon.prefs());
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: cli.live_claim_id(42),
            delivery_id: cli.pending_launch_delivery_id(42),
            window_id: cli.launched_window_id(42),
        };
        assert!(matches!(
            cli.stop_only(&target, "provider rate limit", "2026-08-07T00:00:10Z"),
            IssueMonitorStopOutcome::Stopped { .. }
        ));
        let disk = cli.prefs();

        // The daemon's next scan rebases onto that disk state.
        daemon.rebase_daemon_driver_prefs(&disk);

        assert_eq!(
            daemon.active_count(),
            0,
            "the daemon must release the slot the stop freed"
        );
        assert_eq!(
            daemon.stop_only_reason(42).as_deref(),
            Some("provider rate limit"),
            "the daemon must adopt the stop, not just the slot release"
        );
        assert_eq!(
            daemon.autonomous_record(42).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman),
            "a daemon still in Implementing will relaunch or wait for a verdict \
             that is never coming"
        );
        assert!(
            !daemon.queued_issue_numbers().contains(&42),
            "a stopped issue must not be requeued by the rebase"
        );
        assert_eq!(
            daemon.launched_window_id(42),
            None,
            "the window binding must be gone, or a delayed close requeues it"
        );

        // And the daemon's own commit must not write the stop back out.
        let committed = daemon.prefs();
        assert!(
            committed.launched_issues.is_empty(),
            "the daemon must not restore the launch it just adopted as stopped"
        );
        assert_eq!(
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), committed)
                .stop_only_reason(42)
                .as_deref(),
            Some("provider rate limit"),
            "the stop must survive the daemon's next prefs commit"
        );
    }

    /// SPEC-3431 FR-033 / T-087c: a launch ACK that lands after the stop must
    /// not resurrect the launch.
    ///
    /// The GUI ACKs a materialization it started before the PM stopped the
    /// issue. Accepting it would re-bind a window and re-take the slot for
    /// work that is already held, which is the same "stop becomes a restart"
    /// failure the delayed close causes, arriving from the other direction.
    #[test]
    fn a_launch_ack_after_a_stop_does_not_resurrect_the_issue() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-06-26T00:00:00Z");
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-1",
            "owner-1",
            "effect-1",
            "2026-08-07T00:00:00Z",
        ));

        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        assert!(
            target.delivery_id.is_some() && target.window_id.is_none(),
            "precondition: a materializing launch is identified by its delivery"
        );
        assert!(matches!(
            monitor.stop_only(&target, "switch provider", "2026-08-07T00:00:10Z"),
            IssueMonitorStopOutcome::Stopped { .. }
        ));

        assert!(
            !monitor.complete_active_launch_delivery(42, "tab-1::agent-1", Some("launch:effect-1")),
            "the in-flight delivery was revoked; its ACK must be refused"
        );
        assert_eq!(monitor.active_count(), 0, "the slot must stay released");
        assert_eq!(
            monitor.launched_window_id(42),
            None,
            "no window may be bound to a stopped issue"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
    }

    /// SPEC-3431 FR-033 / T-087c: a scan running against the stopped issue
    /// must not re-queue or re-launch it.
    ///
    /// This is the race that matters most in practice: `launch_now` and the
    /// interval tick both scan, and a stop that a scan can undo is not a stop.
    #[test]
    fn a_scan_after_a_stop_does_not_relaunch_the_issue() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        assert!(matches!(
            monitor.stop_only(&target, "switch provider", "2026-08-07T00:00:10Z"),
            IssueMonitorStopOutcome::Stopped { .. }
        ));

        // The issue is still open on GitHub, so every later scan sees it as a
        // candidate. It must stay held rather than returning to the queue.
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-08-07T00:01:00Z");

        assert!(
            !monitor.queued_issue_numbers().contains(&42),
            "a scan must not re-queue a stopped issue"
        );
        assert_eq!(monitor.active_count(), 0, "and must not re-take its slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman),
            "the hold must survive the rescan"
        );
        assert_eq!(
            monitor.stop_only_reason(42).as_deref(),
            Some("switch provider")
        );
    }

    /// SPEC-3431 FR-029/FR-030 / T-080: a failover puts the issue back in line
    /// for the profile that is saved now.
    ///
    /// This is the operation the PM needs when a provider runs out of quota:
    /// the work is fine, the account is not. So unlike a close it spends no
    /// retry attempt, and unlike a stop it does not hold the issue.
    #[test]
    fn failover_restart_requeues_at_the_head_without_spending_an_attempt() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-08-07T00:00:00Z",
        );
        let attempts_before = monitor.attempt_count(42);
        let epoch_before = monitor.effect_authority_epoch();
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };

        assert_eq!(
            monitor.failover_restart(&target, "codex rate limit", "2026-08-07T00:00:10Z"),
            IssueMonitorFailoverOutcome::Restarting {
                stopped_window_id: Some("tab-1::agent-1".to_string())
            }
        );

        assert_eq!(monitor.active_count(), 0, "the old launch must be revoked");
        assert!(
            monitor.effect_authority_epoch() > epoch_before,
            "an effect still in flight for the old provider must lose authority"
        );
        assert_eq!(
            monitor.attempt_count(42),
            attempts_before,
            "the account ran out, not the work — the retry budget is not the \
             place to charge someone else's billing cycle"
        );
        assert_eq!(
            monitor.queued_issue_numbers().first().copied(),
            Some(42),
            "the operator asked for this issue to run next, not eventually"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert_eq!(
            monitor.launched_window_id(42),
            None,
            "the old provider's session must not be reused"
        );
        assert!(
            monitor.stop_only_reason(42).is_none(),
            "a failover is not a hold; nothing may keep it out of the queue"
        );
        assert!(
            monitor.retry_ready(42, "2026-08-07T00:00:11Z"),
            "no backoff floor: the whole point is to run it now on the new profile"
        );
    }

    /// SPEC #3165 FR-100/FR-101 / T-224: an explicit failover must carry its
    /// fresh-session origin through the queued state, a prefs reload, the
    /// durable delivery, and every replay of the launch request.
    #[test]
    fn failover_launch_strategy_survives_reload_and_delivery_replay() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let attempts_before = monitor.attempt_count(42);
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };

        assert!(matches!(
            monitor.failover_restart(&target, "switch provider", "2026-08-13T00:00:00Z"),
            IssueMonitorFailoverOutcome::Restarting { .. }
        ));
        assert_eq!(monitor.attempt_count(42), attempts_before);
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.as_deref()),
            None,
            "provider failover must not schedule autonomous retry backoff"
        );
        assert_ne!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman),
            "provider failover must not spend the NeedsHuman budget"
        );

        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        scan_issue_monitor_candidates(&mut restored, &[issue(42)], "2026-08-13T00:00:01Z");
        assert!(restored.apply_confirmed_claim(
            42,
            "claim-42-retry",
            "host/session",
            "effect-42-retry",
            "2026-08-13T00:00:02Z",
        ));

        let durable = restored.prefs();
        assert_eq!(
            durable.pending_launch_deliveries[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired,
            "the durable delivery must retain the failover origin"
        );
        let mut replayed = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), durable);
        let first = replayed.take_pending_launch_requests();
        let second = replayed.take_pending_launch_requests();
        assert_eq!(first, second, "an unacked delivery remains replayable");
        assert_eq!(
            first[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired,
            "delivery replay must never downgrade failover to ResumeIfSafe"
        );
    }

    #[test]
    fn agent_writer_conflict_accepts_legacy_bare_and_qualified_window_ids() {
        let mut legacy_stored = launched_monitor(42, "agent-old");
        assert_eq!(
            legacy_stored.try_requeue_agent_resume_writer_conflict(
                42,
                "tab-1::agent-old",
                "resume writer conflict",
                None,
            ),
            IssueMonitorResumeWriterConflictOutcome::Requeued,
            "a qualified runtime event must match a legacy bare persisted id"
        );

        let mut qualified_stored = launched_monitor(42, "tab-1::agent-old");
        assert_eq!(
            qualified_stored.try_requeue_agent_resume_writer_conflict(
                42,
                "agent-old",
                "resume writer conflict",
                None,
            ),
            IssueMonitorResumeWriterConflictOutcome::Requeued,
            "a legacy bare runtime event must match a qualified persisted id"
        );

        let mut foreign_tab = launched_monitor(42, "tab-1::agent-old");
        let original = foreign_tab.prefs();
        assert_eq!(
            foreign_tab.try_requeue_agent_resume_writer_conflict(
                42,
                "tab-2::agent-old",
                "resume writer conflict",
                None,
            ),
            IssueMonitorResumeWriterConflictOutcome::Rejected,
            "two qualified ids from different project tabs must remain distinct"
        );
        assert_eq!(foreign_tab.prefs(), original, "foreign source is inert");
    }

    #[test]
    fn agent_writer_conflict_requires_exact_launched_window_and_cannot_revoke_successor() {
        let mut monitor = launched_monitor(42, "tab-1::agent-old");
        let original = monitor.prefs();

        assert!(!monitor.requeue_agent_resume_writer_conflict(
            43,
            "tab-1::agent-old",
            "resume writer conflict",
            None,
        ));
        assert!(!monitor.requeue_agent_resume_writer_conflict(
            42,
            "tab-1::agent-foreign",
            "resume writer conflict",
            None,
        ));
        assert_eq!(monitor.prefs(), original, "identity mismatch is inert");

        assert!(monitor.requeue_agent_resume_writer_conflict(
            42,
            "tab-1::agent-old",
            "resume writer conflict",
            Some("tab-1::agent-holder"),
        ));
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
        assert!(monitor
            .status_view()
            .last_error
            .as_deref()
            .is_some_and(|diagnostic| diagnostic.contains("tab-1::agent-holder")));

        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-successor",
            "host/session",
            "effect-successor",
            "2026-08-13T00:00:01Z",
        ));
        let successor = monitor.prefs();
        assert_eq!(
            successor.pending_launch_deliveries[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired
        );

        assert!(!monitor.requeue_agent_resume_writer_conflict(
            42,
            "tab-1::agent-old",
            "late duplicate",
            Some("tab-1::agent-holder"),
        ));
        assert_eq!(
            monitor.prefs(),
            successor,
            "a stale source window cannot revoke the fresh successor"
        );
    }

    #[test]
    fn launch_writer_conflict_requires_exact_delivery_materializer_identity() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-08-13T00:00:00Z",
        ));
        assert!(monitor.claim_launch_delivery(
            42,
            "launch:effect-42",
            "gui-old",
            101,
            "tab-1::agent-old",
            |_| false,
        ));
        let original = monitor.prefs();

        assert!(!monitor.requeue_launch_resume_writer_conflict(
            42,
            "launch:foreign",
            "gui-old",
            "resume writer conflict",
            None,
        ));
        assert!(!monitor.requeue_launch_resume_writer_conflict(
            42,
            "launch:effect-42",
            "gui-foreign",
            "resume writer conflict",
            None,
        ));
        assert_eq!(monitor.prefs(), original, "identity mismatch is inert");

        assert!(monitor.requeue_launch_resume_writer_conflict(
            42,
            "launch:effect-42",
            "gui-old",
            "resume writer conflict",
            None,
        ));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-successor",
            "host/session",
            "effect-successor",
            "2026-08-13T00:00:01Z",
        ));
        let successor = monitor.prefs();

        assert!(!monitor.requeue_launch_resume_writer_conflict(
            42,
            "launch:effect-42",
            "gui-old",
            "late duplicate",
            None,
        ));
        assert_eq!(
            monitor.prefs(),
            successor,
            "a stale delivery/materializer cannot revoke the fresh successor"
        );
    }

    #[test]
    fn terminal_failures_clear_queued_fresh_launch_strategy_locally_and_from_disk() {
        let mut monitor = launched_monitor(42, "tab-1::agent-old");
        let target = stop_target(&monitor, 42);
        assert!(matches!(
            monitor.failover_restart(&target, "switch", "2026-08-13T00:00:00Z"),
            IssueMonitorFailoverOutcome::Restarting { .. }
        ));
        assert_eq!(
            monitor.prefs().queued_launch_session_strategies.get(&42),
            Some(&IssueMonitorLaunchSessionStrategy::FreshRequired)
        );

        monitor.record_agent_issue_failed(42, "terminal agent failure");

        assert!(monitor.prefs().queued_launch_session_strategies.is_empty());

        let disk = IssueMonitorPrefs {
            queued_launch_session_strategies: BTreeMap::from([(
                42,
                IssueMonitorLaunchSessionStrategy::FreshRequired,
            )]),
            failed_issues: vec![IssueMonitorFailedIssue {
                issue_number: 42,
                message: "terminal disk failure".to_string(),
                window_id: None,
            }],
            ..IssueMonitorPrefs::default()
        };

        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), disk.clone());
        assert!(restored.prefs().queued_launch_session_strategies.is_empty());

        let mut rebased = IssueMonitorState::new(IssueMonitorConfig::default());
        rebased.record_candidate(issue(42));
        rebased.rebase_gui_observer_prefs(&disk);
        assert!(rebased.prefs().queued_launch_session_strategies.is_empty());
    }

    /// SPEC-3431 FR-030 / T-080: the failover shares `stop_only`'s exact
    /// identity gate, so a stale request restarts nothing.
    #[test]
    fn failover_restart_fails_closed_on_a_stale_identity() {
        let base = launched_monitor(42, "tab-1::agent-1");
        let good = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: base.live_claim_id(42),
            delivery_id: base.pending_launch_delivery_id(42),
            window_id: base.launched_window_id(42),
        };

        for (target, expected) in [
            (
                IssueMonitorStopTarget {
                    window_id: Some("tab-1::agent-2".to_string()),
                    ..good.clone()
                },
                IssueMonitorStopMismatch::WindowMismatch,
            ),
            (
                IssueMonitorStopTarget {
                    claim_id: Some("foreign".to_string()),
                    ..good.clone()
                },
                IssueMonitorStopMismatch::ClaimMismatch,
            ),
            (
                IssueMonitorStopTarget {
                    issue_number: 99,
                    ..good.clone()
                },
                IssueMonitorStopMismatch::UnknownIssue,
            ),
        ] {
            let mut monitor = base.clone();
            assert_eq!(
                monitor.failover_restart(&target, "switch", "2026-08-07T00:00:10Z"),
                IssueMonitorFailoverOutcome::Mismatch(expected)
            );
            assert_eq!(
                monitor.active_count(),
                1,
                "a refused failover must leave the launch running"
            );
            assert!(
                !monitor.queued_issue_numbers().contains(&42),
                "a refused failover must not queue anything"
            );
        }
    }

    /// An `agent_failed` row exactly as the 2026-08-17 outage left it: the
    /// launch is gone, the hold is persisted, and the issue is out of the queue.
    fn agent_failed_monitor(number: u64, message: &str) -> IssueMonitorState {
        let mut monitor = launched_monitor(number, "tab-1::agent-1");
        monitor.record_agent_issue_failed(number, message);
        assert_eq!(monitor.active_count(), 0);
        assert!(!monitor.queued_issue_numbers().contains(&number));
        assert_eq!(
            monitor.inbox_item(number).map(|item| item.state),
            Some(MonitorInboxState::AgentFailed)
        );
        monitor
    }

    /// Issue #3645 AC-1 / #3628 AC-1: an `agent_failed` row holds no live
    /// launch, so every exact-identity operation (`stop_only`,
    /// `failover_restart`) refuses it and the operator has nothing left but a
    /// hand edit of the state file. This is the operation that ends that.
    #[test]
    fn requeue_failed_issue_returns_an_agent_failed_row_to_the_queue() {
        let mut monitor = agent_failed_monitor(42, "an execution generation already exists");
        for _ in 0..3 {
            monitor.record_attempt(42);
        }
        {
            let record = monitor.autonomous_record_mut(42);
            record.phase = AutonomousPhase::Reviewing;
            record.pr_number = Some(3734);
            record.reviewed_sha = Some("reviewed-sha".to_string());
            record.review_passed = Some(true);
            record.retry_not_before = Some("2026-08-18T00:00:00Z".to_string());
            record.retry_hold_reason = Some("provider_usage_limit:codex".to_string());
            record.retry_hold_provider = Some("codex".to_string());
        }
        let attempts_before = monitor.attempt_count(42);

        assert_eq!(
            monitor.requeue_failed_issue(42, "operator recovery", "2026-08-17T16:00:00Z"),
            IssueMonitorRequeueOutcome::Requeued {
                stale_window_id: Some("tab-1::agent-1".to_string()),
                attempts_before: 3,
                attempts_after: 0,
            }
        );

        assert!(
            monitor.queued_issue_numbers().contains(&42),
            "the recovered issue must be launchable again"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert!(monitor.inbox_item(42).unwrap().error_message.is_none());
        assert!(
            !monitor
                .prefs()
                .failed_issues
                .iter()
                .any(|failed| failed.issue_number == 42),
            "the persisted hold must be gone, not just the in-memory row"
        );
        assert_eq!(
            monitor.attempt_count(42),
            0,
            "an explicit operator recovery starts a fresh bounded attempt cycle"
        );
        assert_eq!(attempts_before, 3);
        let autonomous = monitor.autonomous_record(42).expect("record retained");
        assert_eq!(autonomous.phase, AutonomousPhase::Idle);
        assert_eq!(autonomous.pr_number, Some(3734));
        assert_eq!(autonomous.reviewed_sha.as_deref(), Some("reviewed-sha"));
        assert_eq!(autonomous.review_passed, Some(true));
        assert_eq!(autonomous.retry_not_before, None);
        assert_eq!(autonomous.retry_hold_reason, None);
        assert_eq!(autonomous.retry_hold_provider, None);
        assert_eq!(
            monitor.prefs().requeue_audit,
            vec![IssueMonitorReleasedFailure {
                issue_number: 42,
                release_version: 1,
                released_at: "2026-08-17T16:00:00Z".to_string(),
                reason: "operator recovery".to_string(),
                attempts_before: 3,
                attempts_after: 0,
            }],
            "the reset delta and operator reason must survive in durable audit history"
        );
        assert_eq!(
            monitor.prefs().queued_launch_session_strategies.get(&42),
            Some(&IssueMonitorLaunchSessionStrategy::FreshRequired),
            "the abandoned session must not be resumed by the recovered launch"
        );
    }

    #[test]
    fn requeue_failed_issue_resets_launch_failed_attempts() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_attempt(42);
        monitor.record_attempt(42);
        monitor.record_launch_failed(42, "saved profile binary missing");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::LaunchFailed)
        );

        assert_eq!(
            monitor.requeue_failed_issue(42, "profile repaired", "2026-08-17T16:01:00Z"),
            IssueMonitorRequeueOutcome::Requeued {
                stale_window_id: Some("tab-1::agent-1".to_string()),
                attempts_before: 2,
                attempts_after: 0,
            }
        );
        assert_eq!(monitor.attempt_count(42), 0);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
    }

    #[test]
    fn attempt_cap_never_derives_needs_human_on_the_next_scan() {
        // Issue #3944 AC-1/AC-2: an exhausted counter is a throttled retry with a
        // steering request, not a park. The next scan launches it again once the
        // backoff elapses, and the launch clears the steering request.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_mode(true);
        monitor.autonomous_tuning.max_attempts = 2;
        assert_eq!(
            monitor.record_autonomous_failure(42, "first failure", "2026-08-17T15:00:00Z"),
            AutonomousFailureOutcome::Retry { attempt: 1 }
        );
        monitor.complete_active_launch(42, "tab-1::agent-2");
        assert_eq!(
            monitor.record_autonomous_failure(42, "second failure", "2026-08-17T15:30:00Z"),
            AutonomousFailureOutcome::Retry { attempt: 2 }
        );
        assert_eq!(monitor.attempt_count(42), 2);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert!(monitor
            .autonomous_record(42)
            .is_some_and(|record| record.steering.is_some()));
        assert_eq!(
            monitor.requeue_failed_issue(42, "operator recovery", "2026-08-17T16:00:00Z"),
            IssueMonitorRequeueOutcome::NotHeld,
            "a throttled retry is not a held failure"
        );

        let protection = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        assert_eq!(
            monitor.prepare_autonomous_candidate(
                &auto_issue(42, "## Acceptance Criteria\n- [ ] AC-1: retry\n"),
                &protection,
                "2026-08-17T17:00:01Z",
            ),
            EligibilityDecision::Eligible,
            "the next scan must not derive NeedsHuman from the exhausted count"
        );
        assert!(
            monitor
                .autonomous_record(42)
                .is_some_and(|record| record.steering.is_none()),
            "a fresh launch supersedes the steering request"
        );
        monitor.set_gui_connected(true);
        let launch = monitor
            .next_launch_request("2026-08-17T17:00:02Z")
            .expect("the throttled issue is launchable on the next request pass");
        assert_eq!(launch.issue_number, 42);
        assert_eq!(
            launch.launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired
        );
    }

    /// Issue #3628 AC-6: the recovery exists for rows no launch can reach.
    /// Applying it to a running agent would fabricate a second claim for the
    /// same issue, which no later compensation can undo.
    #[test]
    fn requeue_failed_issue_refuses_a_live_launch() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let before = monitor.prefs();

        assert_eq!(
            monitor.requeue_failed_issue(42, "operator recovery", "2026-08-17T16:00:00Z"),
            IssueMonitorRequeueOutcome::LaunchLive
        );
        assert_eq!(
            monitor.prefs(),
            before,
            "a refused recovery must leave the live launch untouched"
        );
        assert_eq!(monitor.active_count(), 1);
    }

    /// Saying "recovered" about a row nobody was holding would tell the operator
    /// the state changed when it did not.
    #[test]
    fn requeue_failed_issue_reports_when_nothing_is_held() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-06-26T00:00:00Z");
        let before = monitor.prefs();

        assert_eq!(
            monitor.requeue_failed_issue(42, "operator recovery", "2026-08-17T16:00:00Z"),
            IssueMonitorRequeueOutcome::NotHeld
        );
        assert_eq!(monitor.prefs(), before);
    }

    /// Issue #3645: the manual recovery that produced this Issue was undone
    /// because removing a failure does not propagate. Every other process keeps
    /// the failure in memory and re-stamps it on its next commit, which is why
    /// the PM's hand edit "did not stick". A release therefore has to be a
    /// published fact, not the absence of one.
    #[test]
    fn requeue_failed_issue_release_survives_a_cross_process_rebase() {
        let mut recovering = agent_failed_monitor(42, "an execution generation already exists");
        recovering.record_attempt(42);
        recovering.record_attempt(42);
        assert!(matches!(
            recovering.requeue_failed_issue(42, "operator recovery", "2026-08-17T16:00:00Z"),
            IssueMonitorRequeueOutcome::Requeued { .. }
        ));
        let disk = recovering.prefs();

        for rebase in [
            IssueMonitorState::rebase_gui_observer_prefs
                as fn(&mut IssueMonitorState, &IssueMonitorPrefs),
            IssueMonitorState::rebase_daemon_driver_prefs,
        ] {
            let mut observer = agent_failed_monitor(42, "an execution generation already exists");
            observer.record_attempt(42);
            observer.record_attempt(42);
            assert_eq!(observer.attempt_count(42), 2, "observer starts stale");
            rebase(&mut observer, &disk);
            assert!(
                !observer
                    .prefs()
                    .failed_issues
                    .iter()
                    .any(|failed| failed.issue_number == 42),
                "a process that still holds the failure must converge on the release"
            );
            assert!(
                observer.queued_issue_numbers().contains(&42),
                "converging on the release must put the issue back in line"
            );
            assert_eq!(
                observer.inbox_item(42).map(|item| item.state),
                Some(MonitorInboxState::Queued)
            );
            assert_eq!(observer.attempt_count(42), 0);
            assert_eq!(
                observer.prefs().requeue_audit.len(),
                1,
                "the release and reset audit must converge together"
            );
            rebase(&mut observer, &disk);
            assert_eq!(
                observer.prefs().requeue_audit.len(),
                1,
                "replaying the same release version must not duplicate audit history"
            );
            // And the converged state must itself publish the release, or the
            // next process to rebase on it would re-stamp the failure again.
            let mut third = agent_failed_monitor(42, "an execution generation already exists");
            third.rebase_gui_observer_prefs(&observer.prefs());
            assert!(
                !third
                    .prefs()
                    .failed_issues
                    .iter()
                    .any(|failed| failed.issue_number == 42),
                "the release must remain published across a second hop"
            );
        }
    }

    /// Issue #3757 / SPEC #3165 Scenario 102: the release is an active fence,
    /// not merely a one-shot attempt reset. A daemon that still holds the
    /// abandoned launch and heartbeat must not turn the recovered row straight
    /// back into a stuck attempt on its next rebase.
    #[test]
    fn requeue_release_fences_stale_active_lifecycle_across_rebase_and_restart() {
        let mut recovering = agent_failed_monitor(42, "attempts exhausted");
        for _ in 0..3 {
            recovering.record_attempt(42);
        }
        assert!(matches!(
            recovering.requeue_failed_issue(42, "operator recovery", "2026-08-26T00:10:00Z"),
            IssueMonitorRequeueOutcome::Requeued { .. }
        ));
        let released = recovering.prefs();

        let mut stale_daemon = launched_monitor(42, "tab-1::stale-agent");
        stale_daemon.set_autonomous_mode(true);
        for _ in 0..3 {
            stale_daemon.record_attempt(42);
        }
        stale_daemon.set_autonomous_phase(42, AutonomousPhase::Implementing);
        stale_daemon.record_autonomous_heartbeat(42, "2026-08-25T20:00:00Z");

        stale_daemon.rebase_daemon_driver_prefs(&released);

        assert_eq!(stale_daemon.active_count(), 0);
        assert!(stale_daemon.prefs().launched_issues.is_empty());
        assert!(stale_daemon.prefs().launching_issues.is_empty());
        assert!(stale_daemon.prefs().pending_launch_deliveries.is_empty());
        assert_eq!(stale_daemon.attempt_count(42), 0);
        assert_eq!(
            stale_daemon
                .autonomous_record(42)
                .and_then(|record| record.last_heartbeat.as_deref()),
            None
        );
        assert!(
            stale_daemon
                .recover_stuck_autonomous("2026-08-26T00:20:00Z")
                .is_empty(),
            "the abandoned heartbeat cannot spend the fresh retry budget"
        );

        let restarted =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), stale_daemon.prefs());
        assert_eq!(restarted.active_count(), 0);
        assert_eq!(restarted.attempt_count(42), 0);
        assert!(restarted.queued_issue_numbers().contains(&42));
    }

    /// Issue #3757 / SPEC #3165 Scenario 104: only the exact durable fresh
    /// claim transition may consume the release fence. A process still holding
    /// the old release must then converge on that positive fresh-lifecycle
    /// evidence rather than clearing it.
    #[test]
    fn confirmed_fresh_claim_consumes_requeue_fence_and_wins_stale_release() {
        let mut recovered = agent_failed_monitor(42, "attempts exhausted");
        assert!(matches!(
            recovered.requeue_failed_issue(42, "operator recovery", "2026-08-26T00:10:00Z"),
            IssueMonitorRequeueOutcome::Requeued { .. }
        ));
        let stale_release = recovered.prefs();

        assert!(recovered.apply_confirmed_claim(
            42,
            "fresh-claim",
            "host/session",
            "fresh-effect",
            "2026-08-26T00:11:00Z",
        ));
        let fresh = recovered.prefs();
        assert!(
            fresh.released_failures.is_empty(),
            "the confirmed claim and durable delivery consume the active fence atomically"
        );
        assert_eq!(fresh.pending_launch_deliveries.len(), 1);

        let mut stale_observer =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), stale_release);
        stale_observer.rebase_daemon_driver_prefs(&fresh);
        assert!(stale_observer.prefs().released_failures.is_empty());
        assert_eq!(stale_observer.active_issue_numbers(), vec![42]);
        assert_eq!(stale_observer.prefs().pending_launch_deliveries.len(), 1);
    }

    /// Issue #3757 / SPEC #3165 FR-134: an AcquireClaim that crossed its
    /// execution fence before requeue is explicitly compensated. Its late
    /// success must release the remote claim without starting a local cycle.
    #[test]
    fn requeue_fence_rejects_pre_release_attempting_claim_result() {
        let mut monitor = agent_failed_monitor(42, "attempts exhausted");
        let key = monitor
            .prepare_pending_effect(
                "claim-before-release",
                IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 42,
                    claim_id: "old-claim".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-08-26T00:00:00Z".to_string(),
                    expires_at: "2026-08-26T00:30:00Z".to_string(),
                    launched_work_id: Some("work/issue-42".to_string()),
                },
            )
            .expect("prepare old claim");
        assert!(monitor.mark_pending_effect_attempting(&key));
        assert!(matches!(
            monitor.requeue_failed_issue(42, "operator recovery", "2026-08-26T00:10:00Z"),
            IssueMonitorRequeueOutcome::Requeued { .. }
        ));
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 42,
                claim_id,
                owner,
            } if claim_id == "old-claim" && owner == "host/session"
        )));

        assert!(!monitor.apply_confirmed_claim(
            42,
            "old-claim",
            "host/session",
            "claim-before-release",
            "2026-08-26T00:11:00Z",
        ));
        let prefs = monitor.prefs();
        assert_eq!(prefs.released_failures.len(), 1);
        assert!(prefs.launching_issues.is_empty());
        assert!(prefs.pending_launch_deliveries.is_empty());
    }

    /// Issue #3757: a real terminal transition is stronger than a recovery
    /// fence. Completion/close after requeue must never be reverted to Queued.
    #[test]
    fn terminal_transition_consumes_requeue_fence() {
        let complete_spec = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithCompletedTasks,
            "2026-08-26T00:00:00Z",
        );
        let mut merged = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut merged,
            std::slice::from_ref(&complete_spec),
            "2026-08-26T00:00:00Z",
        );
        merged.record_agent_issue_failed(42, "attempts exhausted");
        assert!(matches!(
            merged.requeue_failed_issue(42, "operator recovery", "2026-08-26T00:10:00Z"),
            IssueMonitorRequeueOutcome::Requeued { .. }
        ));
        merged.record_merged(42);
        assert!(merged.prefs().released_failures.is_empty());
        let restarted =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), merged.prefs());
        assert!(restarted.merged_issues.contains(&42));
        assert!(!restarted.queued_issue_numbers().contains(&42));

        let mut released = agent_failed_monitor(43, "attempts exhausted");
        assert!(matches!(
            released.requeue_failed_issue(43, "operator recovery", "2026-08-26T00:10:00Z"),
            IssueMonitorRequeueOutcome::Requeued { .. }
        ));
        released.record_released(43);
        assert!(released.prefs().released_failures.is_empty());
        assert!(!released.queued_issue_numbers().contains(&43));
        assert!(released.issue_is_closed(43));
        assert!(released.inbox_item(43).is_none());
    }

    /// Issue #3757 / SPEC #3165 Scenario 105: the durable effect planner is a
    /// claim boundary too. It must not prepare a remote claim while the retry
    /// backoff is still in force, even though the Issue remains queued.
    #[test]
    fn claim_effect_planner_honors_autonomous_retry_backoff() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.config.enabled = true;
        monitor.set_gui_connected(true);
        monitor.set_autonomous_mode(true);
        monitor.autonomous_tuning.retry_backoff_base_secs = 60;
        assert_eq!(
            monitor.record_autonomous_failure(42, "retry later", "2026-08-26T00:00:00Z",),
            AutonomousFailureOutcome::Retry { attempt: 1 }
        );

        assert_eq!(
            monitor
                .try_prepare_claim_effects_with_probe(
                    "host/session",
                    "2026-08-26T00:00:30Z",
                    1,
                    |_| Ok::<bool, std::convert::Infallible>(false),
                )
                .expect("infallible probe"),
            0
        );
        assert!(monitor.pending_effects().is_empty());
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(monitor.attempt_count(42), 1);

        assert_eq!(
            monitor
                .try_prepare_claim_effects_with_probe(
                    "host/session",
                    "2026-08-26T00:01:00Z",
                    1,
                    |_| Ok::<bool, std::convert::Infallible>(false),
                )
                .expect("infallible probe"),
            1,
            "claim planning resumes when the existing backoff expires"
        );
    }

    /// Issue #3757: skipping a backoff-held head entry means queue cleanup must
    /// remove the selected candidate, not blindly discard the queue head.
    #[test]
    fn synchronous_claim_cleanup_preserves_backoff_held_queue_head() {
        use gwt_github::{FakeIssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt};
        let github_issue = |number| IssueSnapshot {
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: String::new(),
            labels: Vec::new(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new("t1"),
            comments: Vec::new(),
        };
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        client.seed(github_issue(43));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.set_autonomous_mode(true);
        monitor.record_candidate(issue(42));
        monitor.record_candidate(issue(43));
        monitor.complete_active_launch(42, "tab-1::agent-42");
        assert_eq!(
            monitor.record_autonomous_failure(42, "retry later", "2026-08-26T00:00:00Z",),
            AutonomousFailureOutcome::Retry { attempt: 1 }
        );
        monitor.queue.retain(|issue_number| *issue_number != 42);
        monitor.queue.push_front(42);
        monitor.inbox.retain(|item| item.issue.number != 43);

        assert!(monitor
            .claim_next_launch_requests(&client, "host-a/session-a", "2026-08-26T00:00:30Z")
            .is_empty());
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
    }

    #[test]
    fn requeue_audit_is_bounded_to_the_latest_hundred_releases() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        for issue_number in 1..=101 {
            monitor.record_candidate(issue(issue_number));
            monitor.record_attempt(issue_number);
            monitor.record_launch_failed(issue_number, "recoverable launch failure");
            assert!(matches!(
                monitor.requeue_failed_issue(
                    issue_number,
                    "operator recovery",
                    "2026-08-17T16:00:00Z",
                ),
                IssueMonitorRequeueOutcome::Requeued { .. }
            ));
        }

        let audit = monitor.prefs().requeue_audit;
        assert_eq!(audit.len(), 100);
        assert_eq!(audit.first().map(|entry| entry.release_version), Some(2));
        assert_eq!(audit.last().map(|entry| entry.release_version), Some(101));
        assert!(audit.iter().all(|entry| entry.attempts_before == 1));
        assert!(audit.iter().all(|entry| entry.attempts_after == 0));
    }

    #[test]
    fn cross_process_requeue_audit_merges_dedupes_and_caps_distinct_histories() {
        let audit_entry = |release_version| IssueMonitorReleasedFailure {
            issue_number: release_version,
            release_version,
            released_at: format!("2026-08-17T16:{:02}:00Z", release_version % 60),
            reason: format!("operator recovery {release_version}"),
            attempts_before: 1,
            attempts_after: 0,
        };
        let mut observer = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                failure_release_version: 100,
                requeue_audit: (1..=100).map(&audit_entry).collect(),
                ..IssueMonitorPrefs::default()
            },
        );
        let disk = IssueMonitorPrefs {
            failure_release_version: 150,
            requeue_audit: (51..=150).map(audit_entry).collect(),
            ..IssueMonitorPrefs::default()
        };

        observer.rebase_gui_observer_prefs(&disk);

        let audit = observer.prefs().requeue_audit;
        assert_eq!(audit.len(), 100);
        assert_eq!(audit.first().map(|entry| entry.release_version), Some(51));
        assert_eq!(audit.last().map(|entry| entry.release_version), Some(150));
        assert_eq!(
            audit
                .iter()
                .map(|entry| entry.release_version)
                .collect::<BTreeSet<_>>()
                .len(),
            100,
            "overlapping disk and memory histories must be deduplicated before the cap"
        );
    }

    /// A release is not permanent immunity. Work that fails again after being
    /// recovered must hold exactly like any other failure.
    #[test]
    fn a_failure_recorded_after_a_release_is_not_erased_by_it() {
        let mut monitor = agent_failed_monitor(42, "first failure");
        assert!(matches!(
            monitor.requeue_failed_issue(42, "operator recovery", "2026-08-17T16:00:00Z"),
            IssueMonitorRequeueOutcome::Requeued { .. }
        ));
        let released = monitor.prefs();
        assert_eq!(released.requeue_audit.len(), 1);

        monitor.complete_active_launch(42, "tab-1::agent-2");
        monitor.record_agent_issue_failed(42, "second failure");
        let refailed = monitor.prefs();

        assert!(
            refailed
                .failed_issues
                .iter()
                .any(|failed| failed.issue_number == 42),
            "the newer failure is persisted"
        );
        assert!(refailed.released_failures.is_empty());
        assert_eq!(
            refailed.requeue_audit, released.requeue_audit,
            "a later failure supersedes active convergence state but not audit history"
        );

        // A process still carrying the older release snapshot must not undo it.
        let mut observer = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                failed_issues: Vec::new(),
                ..released.clone()
            },
        );
        observer.record_candidate(issue(42));
        observer.rebase_gui_observer_prefs(&refailed);
        assert!(
            observer
                .prefs()
                .failed_issues
                .iter()
                .any(|failed| failed.issue_number == 42),
            "a stale release must not erase a failure recorded after it"
        );
    }

    /// SPEC-3431 FR-030 / T-080: after a failover the issue has exactly one
    /// active pane, and the delayed close of the abandoned one does not add a
    /// second.
    #[test]
    fn failover_restart_leaves_one_active_pane_after_the_old_one_closes() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        monitor.failover_restart(&target, "codex rate limit", "2026-08-07T00:00:10Z");

        // The new provider's launch lands.
        monitor.complete_active_launch(42, "tab-1::agent-2");
        assert_eq!(monitor.active_count(), 1);

        // The abandoned pane is reaped a moment later. It is no longer bound to
        // the issue, so it must not touch the launch that replaced it.
        assert_eq!(
            monitor.requeue_window_at("tab-1::agent-1", "2026-08-07T00:00:20Z"),
            None,
            "the old window must not requeue the issue its replacement now owns"
        );
        assert_eq!(monitor.active_count(), 1, "still exactly one active pane");
        assert_eq!(
            monitor.launched_window_id(42).as_deref(),
            Some("tab-1::agent-2")
        );
    }

    /// SPEC-3431 FR-030 / T-080: the old pane's delayed close can also land
    /// *before* the replacement launch. The requeued issue must ride it out
    /// untouched — still queued exactly once, still clean of failure markers.
    #[test]
    fn failover_requeue_survives_a_late_close_before_the_new_launch() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        monitor.failover_restart(&target, "codex rate limit", "2026-08-07T00:00:10Z");

        assert_eq!(
            monitor.requeue_window_at("tab-1::agent-1", "2026-08-07T00:00:12Z"),
            None,
            "the revoked window must not double-queue the issue it lost"
        );
        assert_eq!(
            monitor
                .queued_issue_numbers()
                .iter()
                .filter(|number| **number == 42)
                .count(),
            1,
            "exactly one queue entry survives the late close"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "no failure marker may leak from the reaped pane"
        );
    }

    /// SPEC-3431 FR-030 / T-080: a scan racing the failover (the ScanNow the
    /// failover itself requests) must not double-queue the issue or launch it
    /// twice.
    #[test]
    fn failover_then_scan_keeps_a_single_queue_entry_and_a_single_launch() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        monitor.failover_restart(&target, "codex rate limit", "2026-08-07T00:00:10Z");

        // The immediate scan the failover requested re-observes the same
        // candidate list.
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-08-07T00:00:11Z");
        assert_eq!(
            monitor
                .queued_issue_numbers()
                .iter()
                .filter(|number| **number == 42)
                .count(),
            1,
            "the concurrent scan must not add a second queue entry"
        );

        // The requeued launch lands once; a second scan afterwards must not
        // spawn a sibling.
        monitor.complete_active_launch(42, "tab-1::agent-2");
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-08-07T00:00:20Z");
        assert_eq!(monitor.active_count(), 1, "exactly one launch, ever");
        assert!(
            !monitor.queued_issue_numbers().contains(&42),
            "a running launch must not be queued again by the next scan"
        );
    }

    /// SPEC-3431 FR-031 / T-081: when the restarted launch itself fails, the
    /// failure must converge to a visible inbox error with the slot released —
    /// not a silent loss, and not a phantom active slot.
    #[test]
    fn failed_restart_after_failover_converges_to_a_visible_failure() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        monitor.failover_restart(&target, "codex rate limit", "2026-08-07T00:00:10Z");

        monitor.record_launch_failed(42, "saved profile binary missing");

        assert_eq!(monitor.active_count(), 0, "no phantom active slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::LaunchFailed),
            "the restart failure must be visible, not silently swallowed"
        );
        assert_eq!(
            monitor
                .inbox_item(42)
                .and_then(|item| item.error_message.clone())
                .as_deref(),
            Some("saved profile binary missing"),
            "the diagnosis must carry the reason the PM needs"
        );
        assert!(
            !monitor.queued_issue_numbers().contains(&42),
            "a failed launch must not silently spin in the queue"
        );
    }

    /// SPEC-3431 FR-024 / FR-033: the PM has to be able to *read* the identity
    /// it is required to send. `claim_id` was missing from the snapshot, so an
    /// exact-claim requirement was unsatisfiable from the PM's side.
    #[test]
    fn agent_status_exposes_the_claim_id_stop_only_requires() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-06-26T00:00:00Z");
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-1",
            "owner-1",
            "effect-1",
            "2026-08-07T00:00:00Z",
        ));

        let status = monitor.agent_status();
        let row = status
            .inbox
            .iter()
            .find(|row| row.issue_number == 42)
            .expect("inbox row for the launching issue");
        assert_eq!(
            row.claim_id.as_deref(),
            Some("claim-1"),
            "FR-024: reconcile facts must come from the one snapshot"
        );
        assert_eq!(
            row.delivery_id.as_deref(),
            Some("launch:effect-1"),
            "the PM cannot send a delivery id it cannot read"
        );
    }

    /// SPEC-3431 FR-033: no collateral. Stopping one issue leaves every other
    /// launch, slot, and window exactly as it was.
    #[test]
    fn stop_only_touches_no_other_issue() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_max_active_agents(2);
        scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-06-26T00:00:00Z",
        );
        monitor.complete_active_launch(42, "tab-1::agent-1");
        monitor.complete_active_launch(43, "tab-1::agent-2");
        assert_eq!(monitor.active_count(), 2);

        let target = stop_target(&monitor, 42);
        assert!(matches!(
            monitor.stop_only(&target, "stop", "2026-08-07T00:00:00Z"),
            IssueMonitorStopOutcome::Stopped { .. }
        ));

        assert_eq!(monitor.active_count(), 1, "the other launch keeps its slot");
        assert_eq!(
            monitor.inbox_item(43).map(|item| item.state),
            Some(MonitorInboxState::Launched)
        );
        assert_eq!(
            monitor.launched_window_id(43).as_deref(),
            Some("tab-1::agent-2"),
            "the other window must stay bound"
        );
    }

    #[test]
    fn requeue_window_does_not_revert_merged() {
        let mut monitor = launched_monitor_for_issue(
            spec_issue(
                42,
                IssueMonitorReadiness::ReadyWithCompletedTasks,
                "2026-08-26T00:00:00Z",
            ),
            "tab-1::agent-1",
        );
        monitor.record_merged(42);
        assert_eq!(monitor.requeue_window("tab-1::agent-1"), None);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
    }

    #[test]
    fn record_released_removes_current_state_and_frees_slot() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.transition_issue_closure(
            42,
            IssueClosureState::Reopened,
            IssueClosureEvidence::ExplicitRevision,
            Some("2026-08-26T00:00:00Z".to_string()),
        );
        monitor.record_released(42);
        assert_eq!(monitor.active_count(), 0);
        assert!(monitor.inbox_item(42).is_none());
        assert!(monitor.agent_status().needs_human.is_empty());
        let restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        assert!(restored.inbox_item(42).is_none());
    }

    #[test]
    fn record_released_revokes_target_auto_merge_effects_and_compensates_attempts() {
        let prepared =
            pending_arm_effect("arm:7:99:prepared", 4, 0, IssueMonitorEffectState::Prepared);
        let attempting = pending_arm_effect(
            "arm:7:99:attempting",
            4,
            2,
            IssueMonitorEffectState::Attempting,
        );
        let attempting_claim = PendingIssueMonitorEffect {
            effect_id: "claim:7:attempting".to_string(),
            authority_epoch: 4,
            attempt: 1,
            state: IssueMonitorEffectState::Attempting,
            payload: IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 7,
                claim_id: "claim-7".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-08-26T00:00:00Z".to_string(),
                expires_at: "2026-08-26T00:10:00Z".to_string(),
                launched_work_id: None,
            },
        };
        let unrelated = PendingIssueMonitorEffect {
            effect_id: "arm:8:100:prepared".to_string(),
            authority_epoch: 4,
            attempt: 0,
            state: IssueMonitorEffectState::Prepared,
            payload: IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 8,
                pr_number: 100,
                reviewed_sha: "def456".to_string(),
            },
        };
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                effect_authority_epoch: 4,
                pending_effects: vec![
                    prepared,
                    attempting.clone(),
                    attempting_claim.clone(),
                    unrelated.clone(),
                ],
                ..IssueMonitorPrefs::default()
            },
        );

        monitor.record_released(7);

        assert!(
            monitor.pending_effects().iter().all(|effect| !matches!(
                effect.payload,
                IssueMonitorEffectPayload::ArmAutoMerge {
                    issue_number: 7,
                    ..
                }
            )),
            "a closed Issue must retain no remotely actionable arm tuple"
        );
        assert!(
            monitor.pending_effects().iter().all(|effect| !matches!(
                effect.payload,
                IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 7,
                    ..
                }
            )),
            "a closed Issue must retain no remotely actionable claim tuple"
        );
        assert!(monitor.pending_effects().contains(&unrelated));
        assert_eq!(monitor.effect_authority_epoch(), 4);
        assert_eq!(unrelated.authority_epoch, monitor.effect_authority_epoch());
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: 7,
                pr_number: 99,
                compensates_effect_id,
            } if compensates_effect_id == &attempting.effect_id
        )));
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 7,
                claim_id,
                owner,
            } if claim_id == "claim-7" && owner == "host/session"
        )));
        assert!(
            monitor
                .complete_pending_effect(&attempting.attempt_key())
                .is_none(),
            "a late arm result must not match after closure cleanup"
        );
        assert!(
            monitor
                .complete_pending_effect(&attempting_claim.attempt_key())
                .is_none(),
            "a late claim result must not match after closure cleanup"
        );
    }

    #[test]
    fn record_released_disarms_an_already_armed_delivery_without_resurrection() {
        let mut monitor = autonomous_state();
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-08-26T00:00:00Z");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123");
        monitor.begin_delivering(7);
        assert!(
            monitor.pending_effects().is_empty(),
            "the successful arm tuple has already left the journal"
        );

        monitor.record_released(7);

        assert!(monitor.autonomous_record(7).is_none());
        assert!(monitor.inbox_item(7).is_none());
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: 7,
                pr_number: 99,
                ..
            }
        )));

        monitor.record_kill_switch_disarm_result(7, 99, true);
        assert!(monitor.autonomous_record(7).is_none());
        assert!(monitor.inbox_item(7).is_none());
        assert!(monitor.agent_status().needs_human.is_empty());
        monitor.escalate_to_needs_human(
            7,
            NeedsHumanKind::UserChoiceRequired,
            "late disarm result",
        );
        assert!(monitor.autonomous_record(7).is_none());
        assert!(monitor.agent_status().needs_human.is_empty());
    }

    #[test]
    fn complete_live_absence_closes_an_orphan_auto_merge_grant() {
        let repo = tempfile::tempdir().expect("tempdir");
        let attempting = pending_arm_effect(
            "arm:7:99:attempting",
            4,
            2,
            IssueMonitorEffectState::Attempting,
        );
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                effect_authority_epoch: 4,
                pending_effects: vec![attempting.clone()],
                ..IssueMonitorPrefs::default()
            },
        );

        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[],
            IssueMonitorCandidateSource::Live,
            repo.path(),
            "2026-08-26T00:00:00Z",
        );

        assert!(monitor.pending_effects().iter().all(|effect| !matches!(
            effect.payload,
            IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 7,
                ..
            }
        )));
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: 7,
                compensates_effect_id,
                ..
            } if compensates_effect_id == &attempting.effect_id
        )));
    }

    #[test]
    fn reconcile_merged_branches_marks_merged_and_frees_slot() {
        let mut monitor = launched_monitor_for_issue(
            spec_issue(
                42,
                IssueMonitorReadiness::ReadyWithCompletedTasks,
                "2026-08-17T00:00:00Z",
            ),
            "tab-1::agent-1",
        );
        let branch = monitor
            .active_launched_branches()
            .into_iter()
            .find(|(number, _)| *number == 42)
            .map(|(_, branch)| branch)
            .expect("launched branch");
        let merged: BTreeSet<String> = [branch].into_iter().collect();
        assert_eq!(monitor.reconcile_merged_branches(&merged), vec![42]);
        assert_eq!(monitor.active_count(), 0, "merged work frees the slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
    }

    #[test]
    fn reconcile_merged_branches_ignores_unmerged_branches() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let merged: BTreeSet<String> = ["work/some-other-branch".to_string()].into_iter().collect();
        assert!(monitor.reconcile_merged_branches(&merged).is_empty());
        assert_eq!(monitor.active_count(), 1, "unmerged work stays launched");
    }

    /// Issue #3645 AC-3/AC-4: emptying `launched_issues` — the repair the
    /// 2026-08-17 outage required — makes every in-flight launch invisible to
    /// [`IssueMonitorState::reconcile_merged_branches`], which only ever looks
    /// at `active_launched_branches`. Completion detection therefore has to have
    /// a second entrance that does not depend on the tracking at all.
    #[test]
    fn untracked_merged_work_is_still_a_completion_candidate() {
        let complete_spec = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithCompletedTasks,
            "2026-08-17T00:00:00Z",
        );
        let monitor = launched_monitor_for_issue(complete_spec.clone(), "tab-1::agent-1");
        let branch = monitor
            .active_launched_branches()
            .into_iter()
            .find(|(number, _)| *number == 42)
            .map(|(_, branch)| branch)
            .expect("launched branch");
        let merged: BTreeSet<String> = [branch].into_iter().collect();

        // The launch tracking is wiped exactly as a hand repair wipes it.
        let mut untracked = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                launched_issues: Vec::new(),
                launching_issues: Vec::new(),
                ..monitor.prefs()
            },
        );
        scan_issue_monitor_candidates(&mut untracked, &[complete_spec], "2026-08-17T16:00:00Z");
        assert!(
            untracked.reconcile_merged_branches(&merged).is_empty(),
            "the tracked path cannot see a launch that is no longer tracked"
        );

        assert_eq!(
            untracked
                .untracked_merged_branch_candidates(&merged)
                .iter()
                .map(|issue| issue.number)
                .collect::<Vec<_>>(),
            vec![42],
            "the merged work branch alone must be enough to nominate the issue"
        );
        assert!(untracked.record_untracked_completion(42));
        assert_eq!(
            untracked.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
        assert!(
            !untracked.queued_issue_numbers().contains(&42),
            "delivered work must stop being a relaunch candidate"
        );
    }

    /// A work branch merged for an earlier generation must never park work that
    /// was reopened after it. Nominating is cheap and local; the caller's
    /// generation-aware probe is what decides, so a candidate is never a
    /// completion on its own.
    #[test]
    fn untracked_candidates_exclude_terminal_and_active_rows() {
        let mut monitor = launched_monitor_for_issue(
            spec_issue(
                42,
                IssueMonitorReadiness::ReadyWithCompletedTasks,
                "2026-08-17T00:00:00Z",
            ),
            "tab-1::agent-1",
        );
        let branch = monitor
            .active_launched_branches()
            .into_iter()
            .find(|(number, _)| *number == 42)
            .map(|(_, branch)| branch)
            .expect("launched branch");
        let merged: BTreeSet<String> = [branch].into_iter().collect();

        assert!(
            monitor
                .untracked_merged_branch_candidates(&merged)
                .is_empty(),
            "an active launch belongs to the tracked path, not this one"
        );

        monitor.record_agent_issue_failed(42, "boom");
        assert!(
            monitor
                .untracked_merged_branch_candidates(&merged)
                .is_empty(),
            "a terminal row is recovered by requeue, not silently completed"
        );

        monitor.record_merged(42);
        assert!(
            monitor
                .untracked_merged_branch_candidates(&merged)
                .is_empty(),
            "already-complete work is not nominated again"
        );
    }

    fn spec_issue(
        number: u64,
        readiness: IssueMonitorReadiness,
        updated_at: &str,
    ) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("SPEC {number}"),
            labels: vec!["auto-improve".to_string(), "gwt-spec".to_string()],
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness,
            updated_at: Some(updated_at.to_string()),
        }
    }

    #[test]
    fn incomplete_spec_work_merge_frees_the_slot_and_requeues_remaining_work() {
        let candidate = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithOpenTasks,
            "2026-08-15T00:00:00Z",
        );
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[candidate], "2026-08-15T00:00:01Z");
        monitor.complete_active_launch(42, "tab-1::agent-1");
        let branch = monitor.active_launched_branches()[0].1.clone();

        assert_eq!(
            monitor.reconcile_merged_branches(&BTreeSet::from([branch])),
            vec![42]
        );
        assert_eq!(monitor.active_count(), 0, "work delivery frees the slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "one merged PR is not Issue completion while SPEC tasks remain"
        );
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
        assert!(!monitor.prefs().merged_issues.contains(&42));
    }

    #[test]
    fn ordinary_open_work_merge_frees_the_slot_and_requeues_remaining_acceptance() {
        let mut candidate = issue(42);
        candidate.labels.push("auto-improve".to_string());
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&candidate),
            "2026-08-15T00:00:01Z",
        );
        monitor.complete_active_launch(42, "tab-1::agent-1");

        monitor.record_merged(42);

        assert_eq!(
            monitor.active_count(),
            0,
            "merged work frees the slot first"
        );
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        let prefs = monitor.prefs();
        assert!(!prefs.merged_issues.contains(&42));
        assert_eq!(
            prefs.queued_launch_session_strategies.get(&42),
            Some(&IssueMonitorLaunchSessionStrategy::FreshRequired)
        );
    }

    #[test]
    fn incomplete_spec_autonomous_delivery_clears_the_finished_attempt_before_requeue() {
        let candidate = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithOpenTasks,
            "2026-08-15T00:00:00Z",
        );
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[candidate], "2026-08-15T00:00:01Z");
        monitor.complete_active_launch(42, "tab-1::agent-1");
        monitor.record_attempt(42);
        monitor.set_autonomous_phase(42, AutonomousPhase::Delivering);

        monitor.record_merged(42);

        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert!(monitor.autonomous_record(42).is_none());
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
    }

    #[test]
    fn reopened_delivery_settlement_fences_stale_disk_launch_and_autonomous_companions() {
        let candidate = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithOpenTasks,
            "2026-08-15T00:00:00Z",
        );
        let mut settled = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut settled,
            std::slice::from_ref(&candidate),
            "2026-08-15T00:00:01Z",
        );
        settled.complete_active_launch(42, "tab-1::agent-1");
        settled.record_attempt(42);
        settled.set_autonomous_phase(42, AutonomousPhase::Delivering);
        let stale_disk = settled.prefs();
        settled.record_merged(42);

        let mut daemon = settled.clone();
        daemon.rebase_daemon_driver_prefs(&stale_disk);
        let mut gui = settled;
        gui.rebase_gui_observer_prefs(&stale_disk);

        for monitor in [&daemon, &gui] {
            assert_eq!(monitor.active_count(), 0);
            assert!(monitor.autonomous_record(42).is_none());
            assert_eq!(
                monitor.inbox_item(42).map(|item| item.state),
                Some(MonitorInboxState::Queued)
            );
            let prefs = monitor.prefs();
            assert!(prefs.launched_issues.is_empty());
            assert!(prefs.launching_issues.is_empty());
            assert!(prefs.pending_launch_deliveries.is_empty());
            assert!(prefs.autonomous_records.is_empty());
        }

        let durable_reopen = daemon.prefs();
        let mut successor_disk = durable_reopen.clone();
        successor_disk.launched_issues = vec![IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "tab-1::agent-2".to_string(),
        }];
        successor_disk.autonomous_records = vec![AutonomousIssueRecord {
            phase: AutonomousPhase::Implementing,
            retry_hold_provider: None,
            ..AutonomousIssueRecord::new(42)
        }];
        let mut observer =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), durable_reopen);
        observer.rebase_gui_observer_prefs(&successor_disk);
        assert_eq!(observer.active_issue_numbers(), vec![42]);
        assert_eq!(
            observer.autonomous_record(42).map(|record| record.phase),
            Some(AutonomousPhase::Implementing),
            "the same durable Reopened generation permits a fresh successor"
        );
    }

    #[test]
    fn completed_spec_work_merge_remains_terminal_for_the_same_issue_generation() {
        let candidate = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithCompletedTasks,
            "2026-08-15T00:00:00Z",
        );
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&candidate),
            "2026-08-15T00:00:01Z",
        );
        monitor.complete_active_launch(42, "tab-1::agent-1");
        let branch = monitor.active_launched_branches()[0].1.clone();
        monitor.reconcile_merged_branches(&BTreeSet::from([branch]));

        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        scan_issue_monitor_candidates_with_provenance(
            &mut restored,
            &[candidate],
            IssueMonitorCandidateSource::Live,
            Path::new("."),
            "2026-08-15T00:01:00Z",
        );
        assert_eq!(
            restored.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged),
            "a legitimate completed-open SPEC stays suppressed"
        );
        assert!(!restored.agent_status().inbox[0].recoverable_merged);
    }

    #[test]
    fn complete_live_scan_recovers_all_reported_legacy_merged_markers() {
        let numbers = [3164, 3165, 3200, 3206, 3248, 3273, 3426];
        let legacy = format!(
            r#"{{"enabled":true,"max_active_agents":1,"priority_order":[],"merged_issues":{:?}}}"#,
            numbers
        );
        let prefs: IssueMonitorPrefs = serde_json::from_str(&legacy).expect("legacy prefs");
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        let issues = numbers
            .into_iter()
            .map(|number| {
                let mut issue = issue(number);
                issue.labels.push("auto-improve".to_string());
                issue.updated_at = Some("2026-08-15T00:00:00Z".to_string());
                issue
            })
            .collect::<Vec<_>>();

        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &issues,
            IssueMonitorCandidateSource::Live,
            Path::new("."),
            "2026-08-15T00:00:01Z",
        );

        assert!(monitor.prefs().merged_issues.is_empty());
        assert_eq!(monitor.queued_issue_numbers(), numbers);
        assert!(monitor
            .agent_status()
            .inbox
            .iter()
            .all(|row| row.state == MonitorInboxState::Queued));
    }

    #[test]
    fn live_snapshot_does_not_requeue_an_absent_legacy_completion() {
        let prefs: IssueMonitorPrefs = serde_json::from_str(
            r#"{"enabled":true,"max_active_agents":1,"priority_order":[],"merged_issues":[42]}"#,
        )
        .expect("legacy prefs");
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        let mut cached = issue(42);
        cached.labels.push("auto-improve".to_string());

        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[cached],
            IssueMonitorCandidateSource::Cache,
            Path::new("."),
            "2026-08-15T00:00:01Z",
        );
        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[],
            IssueMonitorCandidateSource::Live,
            Path::new("."),
            "2026-08-15T00:00:02Z",
        );

        assert!(monitor.prefs().merged_issues.is_empty());
        assert!(monitor.queued_issue_numbers().is_empty());
        assert!(monitor.inbox_item(42).is_none());
    }

    #[test]
    fn incomplete_snapshots_diagnose_legacy_completion_without_deleting_it() {
        for source in [
            IssueMonitorCandidateSource::Cache,
            IssueMonitorCandidateSource::LiveIncomplete,
        ] {
            let prefs: IssueMonitorPrefs = serde_json::from_str(
                r#"{"enabled":true,"max_active_agents":1,"priority_order":[],"merged_issues":[42]}"#,
            )
            .expect("legacy prefs");
            let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
            let mut candidate = issue(42);
            candidate.labels.push("auto-improve".to_string());
            candidate.updated_at = Some("2026-08-15T00:00:00Z".to_string());

            scan_issue_monitor_candidates_with_provenance(
                &mut monitor,
                &[candidate],
                source,
                Path::new("."),
                "2026-08-15T00:00:01Z",
            );

            assert_eq!(monitor.prefs().merged_issues, vec![42]);
            let row = &monitor.agent_status().inbox[0];
            assert_eq!(row.github_state, IssueMonitorIssueState::Open);
            assert_eq!(
                row.issue_updated_at.as_deref(),
                Some("2026-08-15T00:00:00Z")
            );
            assert!(row.recoverable_merged);
            assert_eq!(row.completion_reason.as_deref(), Some("legacy_unverified"));
        }
    }

    #[test]
    fn reopened_generation_beats_a_stale_legacy_marker_during_rebase() {
        let stale: IssueMonitorPrefs = serde_json::from_str(
            r#"{"enabled":true,"max_active_agents":1,"priority_order":[],"merged_issues":[42]}"#,
        )
        .expect("legacy prefs");
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), stale.clone());
        let mut candidate = issue(42);
        candidate.labels.push("auto-improve".to_string());
        candidate.updated_at = Some("2026-08-15T00:00:00Z".to_string());
        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[candidate],
            IssueMonitorCandidateSource::Live,
            Path::new("."),
            "2026-08-15T00:00:01Z",
        );

        monitor.rebase_daemon_driver_prefs(&stale);

        assert!(!monitor.prefs().merged_issues.contains(&42));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
    }

    #[test]
    fn same_revision_ordinary_open_completion_is_diagnosed_then_reopened_by_live_scan() {
        let mut current = issue(42);
        current.labels.push("auto-improve".to_string());
        current.updated_at = Some("2026-08-10T00:00:00Z".to_string());
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&current),
            "2026-08-10T00:00:01Z",
        );
        monitor.transition_completion(
            42,
            IssueCompletionState::Completed,
            current.updated_at.clone(),
            IssueCompletionEvidence::LinkedPr,
        );
        monitor.apply_merged_terminal_state(42);

        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            std::slice::from_ref(&current),
            IssueMonitorCandidateSource::Cache,
            Path::new("."),
            "2026-08-15T00:00:01Z",
        );
        let row = &monitor.agent_status().inbox[0];
        assert!(row.recoverable_merged);
        assert_eq!(row.completion_reason.as_deref(), Some("github_issue_open"));
        assert_eq!(monitor.prefs().merged_issues, vec![42]);

        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[current],
            IssueMonitorCandidateSource::Live,
            Path::new("."),
            "2026-08-15T00:00:02Z",
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert!(monitor.prefs().merged_issues.is_empty());
    }

    #[test]
    fn unknown_live_evidence_preserves_completion_but_cannot_create_a_new_one() {
        let complete_spec = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithCompletedTasks,
            "2026-08-10T00:00:00Z",
        );
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&complete_spec),
            "2026-08-10T00:00:01Z",
        );
        monitor.record_merged(42);

        let unknown_spec = IssueMonitorIssue {
            readiness: IssueMonitorReadiness::NotReady,
            updated_at: Some("2026-08-15T00:00:00Z".to_string()),
            ..complete_spec
        };
        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[unknown_spec],
            IssueMonitorCandidateSource::Live,
            Path::new("."),
            "2026-08-15T00:00:01Z",
        );
        assert_eq!(monitor.prefs().merged_issues, vec![42]);

        let mut missing_revision = issue(43);
        missing_revision.labels.push("auto-improve".to_string());
        missing_revision.updated_at = None;
        scan_issue_monitor_candidates(&mut monitor, &[missing_revision], "2026-08-15T00:00:02Z");
        monitor.record_merged(43);
        assert!(!monitor.prefs().merged_issues.contains(&43));
        assert_eq!(
            monitor.inbox_item(43).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );

        let mut ordinary = issue(44);
        ordinary.labels.push("auto-improve".to_string());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&ordinary),
            "2026-08-15T00:00:03Z",
        );
        monitor.record_merged(44);
        ordinary.updated_at = None;
        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[ordinary],
            IssueMonitorCandidateSource::Live,
            Path::new("."),
            "2026-08-15T00:00:04Z",
        );
        assert!(!monitor.prefs().merged_issues.contains(&44));
        assert_eq!(
            monitor.inbox_item(44).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
    }

    #[test]
    fn clear_completion_hold_requeues_live_open_issue_once_and_fences_stale_completion() {
        let mut current = issue(42);
        current.labels.push("auto-improve".to_string());
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&current),
            "2026-08-15T00:00:01Z",
        );
        monitor.transition_completion(
            42,
            IssueCompletionState::Completed,
            current.updated_at.clone(),
            IssueCompletionEvidence::LinkedPr,
        );
        monitor.apply_merged_terminal_state(42);
        let stale = monitor.prefs();

        assert!(monitor.clear_completion_hold(42));
        assert!(!monitor.clear_completion_hold(42), "repeat is idempotent");
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
        assert_eq!(
            monitor.prefs().queued_launch_session_strategies.get(&42),
            Some(&IssueMonitorLaunchSessionStrategy::FreshRequired)
        );

        monitor.rebase_daemon_driver_prefs(&stale);
        assert!(!monitor.prefs().merged_issues.contains(&42));
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
    }

    #[test]
    fn clear_completion_hold_tombstones_cold_completion_but_never_closed_issue() {
        let completed = IssueMonitorPrefs {
            merged_issues: vec![42, 43],
            issue_completion_migration_version: ISSUE_COMPLETION_MIGRATION_VERSION,
            completion_records: vec![
                IssueCompletionRecord {
                    issue_number: 42,
                    generation: 1,
                    state: IssueCompletionState::Completed,
                    issue_updated_at: Some("2026-08-15T00:00:00Z".to_string()),
                    evidence: IssueCompletionEvidence::LinkedPr,
                },
                IssueCompletionRecord {
                    issue_number: 43,
                    generation: 1,
                    state: IssueCompletionState::Completed,
                    issue_updated_at: Some("2026-08-15T00:00:00Z".to_string()),
                    evidence: IssueCompletionEvidence::LinkedPr,
                },
            ],
            ..IssueMonitorPrefs::default()
        };
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), completed);
        monitor.transition_issue_closure(
            43,
            IssueClosureState::Closed,
            IssueClosureEvidence::ExplicitRevision,
            Some("2026-08-15T00:00:00Z".to_string()),
        );

        assert!(monitor.clear_completion_hold(42));
        assert!(!monitor.prefs().merged_issues.contains(&42));
        assert!(
            monitor.queued_issue_numbers().is_empty(),
            "cold rows stay cold"
        );
        assert!(!monitor.clear_completion_hold(43));
        assert!(monitor.issue_is_closed(43));
    }

    #[test]
    fn explicit_fresh_launch_persists_a_reopen_tombstone() {
        let mut monitor = launched_monitor_for_issue(
            spec_issue(
                42,
                IssueMonitorReadiness::ReadyWithCompletedTasks,
                "2026-08-15T00:00:00Z",
            ),
            "tab-1::agent-1",
        );
        monitor.record_merged(42);
        monitor.complete_active_launch(42, "tab-1::agent-2");

        let restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());

        assert!(!restored.prefs().merged_issues.contains(&42));
        assert_eq!(restored.active_issue_numbers(), vec![42]);
    }

    #[test]
    fn autonomous_mode_defaults_false_and_back_compat_deserializes() {
        // SPEC #3200 FR-001/FR-030, Sc 23: pre-autonomous prefs (no
        // autonomous_mode / tuning fields) deserialize with documented defaults
        // and existing fields are preserved.
        let legacy = r#"{"enabled":true,"max_active_agents":1,"priority_order":[101,102],"merged_issues":[42]}"#;
        let prefs: IssueMonitorPrefs =
            serde_json::from_str(legacy).expect("legacy prefs deserialize");
        assert!(!prefs.autonomous_mode, "autonomous_mode defaults to false");
        assert_eq!(prefs.autonomous_tuning, AutonomousTuning::default());
        assert_eq!(prefs.autonomous_tuning.max_attempts, 3);
        assert_eq!(prefs.merged_issues, vec![42], "existing fields preserved");
        assert!(prefs.requeue_audit.is_empty());
        assert!(!IssueMonitorPrefs::default().autonomous_mode);
    }

    #[test]
    fn pre_attempt_delta_release_defaults_to_zero_and_audit_defaults_empty() {
        let legacy = r#"{
            "enabled": true,
            "max_active_agents": 1,
            "priority_order": [],
            "failure_release_version": 1,
            "released_failures": [{
                "issue_number": 42,
                "release_version": 1,
                "released_at": "2026-08-17T16:00:00Z",
                "reason": "operator recovery"
            }]
        }"#;

        let prefs: IssueMonitorPrefs =
            serde_json::from_str(legacy).expect("pre-attempt-delta prefs deserialize");
        assert!(prefs.requeue_audit.is_empty());
        assert_eq!(prefs.released_failures[0].attempts_before, 0);
        assert_eq!(prefs.released_failures[0].attempts_after, 0);
    }

    #[test]
    fn pre_effect_journal_prefs_default_to_epoch_zero_and_no_pending_effects() {
        // SPEC #3200 Phase 7 T-137 / FR-044: prefs written before the durable
        // effect journal existed must remain readable. Missing authority and
        // journal fields mean "no prior authority" rather than a parse error or
        // a fabricated effect.
        let legacy = r#"{
            "enabled": true,
            "max_active_agents": 2,
            "priority_order": [7],
            "autonomous_mode": true
        }"#;

        let prefs: IssueMonitorPrefs =
            serde_json::from_str(legacy).expect("pre-journal prefs deserialize");

        assert_eq!(prefs.effect_authority_epoch, 0);
        assert!(prefs.pending_effects.is_empty());
        assert!(prefs.enabled);
        assert!(prefs.autonomous_mode);
        assert_eq!(prefs.priority_order, vec![7]);
    }

    #[test]
    fn control_receipt_defaults_absent_and_round_trips_through_state_and_rebase() {
        let legacy = r#"{
            "enabled": true,
            "max_active_agents": 1,
            "priority_order": []
        }"#;
        let legacy: IssueMonitorPrefs =
            serde_json::from_str(legacy).expect("pre-receipt prefs deserialize");
        assert_eq!(legacy.last_control_receipt, None);
        assert_eq!(IssueMonitorPrefs::default().last_control_receipt, None);

        let receipt = IssueMonitorControlReceipt {
            control_id: uuid::Uuid::new_v4().to_string(),
            should_scan: true,
            authority_changed: false,
        };
        let restored = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                last_control_receipt: Some(receipt.clone()),
                ..IssueMonitorPrefs::default()
            },
        );
        assert_eq!(restored.prefs().last_control_receipt, Some(receipt.clone()));

        let mut stale = IssueMonitorState::new(IssueMonitorConfig::default());
        stale.rebase_daemon_driver_prefs(&restored.prefs());
        assert_eq!(stale.prefs().last_control_receipt, Some(receipt));
    }

    #[test]
    fn prepared_and_attempting_effects_round_trip_every_authority_field() {
        // A restart must retain the complete delivery identity. Dropping any of
        // effect_id / epoch / attempt / payload would make a stale result look
        // current or make an ambiguous remote mutation impossible to reconcile.
        let prepared =
            pending_arm_effect("arm:7:99:abc123:4", 4, 0, IssueMonitorEffectState::Prepared);
        let attempting = PendingIssueMonitorEffect {
            effect_id: "disarm:arm:7:99:abc123:4".to_string(),
            authority_epoch: 5,
            attempt: 3,
            state: IssueMonitorEffectState::Attempting,
            payload: IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: 7,
                pr_number: 99,
                compensates_effect_id: "arm:7:99:abc123:4".to_string(),
            },
        };
        let prefs = IssueMonitorPrefs {
            effect_authority_epoch: 5,
            pending_effects: vec![prepared.clone(), attempting.clone()],
            ..IssueMonitorPrefs::default()
        };

        let json = serde_json::to_string(&prefs).expect("effect journal serializes");
        let restored: IssueMonitorPrefs =
            serde_json::from_str(&json).expect("effect journal deserializes");

        assert_eq!(restored.effect_authority_epoch, 5);
        assert_eq!(restored.pending_effects, vec![prepared, attempting]);
    }

    #[test]
    fn effect_attempt_and_result_transitions_require_the_exact_delivery_tuple() {
        // The executor may finish after a control advanced authority or after a
        // retry allocated a newer attempt. Both Prepared -> Attempting and the
        // confirmed-result transition must compare all three tuple components.
        let prepared = pending_arm_effect("arm-7", 12, 4, IssueMonitorEffectState::Prepared);
        let exact = effect_attempt_key("arm-7", 12, 4);
        let mismatches = [
            effect_attempt_key("other-effect", 12, 4),
            effect_attempt_key("arm-7", 13, 4),
            effect_attempt_key("arm-7", 12, 5),
        ];

        for mismatch in &mismatches {
            let mut prefs = IssueMonitorPrefs {
                effect_authority_epoch: 12,
                pending_effects: vec![prepared.clone()],
                ..IssueMonitorPrefs::default()
            };
            let before = prefs.clone();
            assert!(!prefs.mark_pending_effect_attempting(mismatch));
            assert_eq!(prefs, before, "mismatched Attempting transition is inert");
        }

        let mut attempting = IssueMonitorPrefs {
            effect_authority_epoch: 12,
            pending_effects: vec![prepared],
            ..IssueMonitorPrefs::default()
        };
        assert!(attempting.mark_pending_effect_attempting(&exact));
        assert_eq!(
            attempting.pending_effects[0].state,
            IssueMonitorEffectState::Attempting
        );

        for mismatch in &mismatches {
            let mut prefs = attempting.clone();
            let before = prefs.clone();
            assert!(prefs.complete_pending_effect(mismatch).is_none());
            assert_eq!(prefs, before, "mismatched result transition is inert");
        }

        let completed = attempting
            .complete_pending_effect(&exact)
            .expect("the exact Attempting result is accepted");
        assert_eq!(completed.effect_id, "arm-7");
        assert!(attempting.pending_effects.is_empty());
    }

    #[test]
    fn autonomous_control_rejects_authority_epoch_wrap_atomically() {
        // Authority must never wrap to zero: an ancient effect could otherwise
        // regain the same epoch. The mode change and journal rewrite are one
        // atomic transition, so overflow leaves every field untouched.
        let mut prefs = IssueMonitorPrefs {
            autonomous_mode: true,
            effect_authority_epoch: u64::MAX,
            pending_effects: vec![pending_arm_effect(
                "arm-max",
                u64::MAX,
                0,
                IssueMonitorEffectState::Prepared,
            )],
            ..IssueMonitorPrefs::default()
        };
        let before = prefs.clone();

        assert_eq!(
            prefs.set_autonomous_mode_with_effect_revocation(false),
            None,
            "checked epoch overflow rejects the control transition"
        );
        assert_eq!(prefs, before);
    }

    #[test]
    fn same_value_authority_controls_are_idempotent_at_epoch_max() {
        let arm = pending_arm_effect("arm-max", u64::MAX, 3, IssueMonitorEffectState::Attempting);
        let mut autonomous = IssueMonitorPrefs {
            autonomous_mode: true,
            effect_authority_epoch: u64::MAX,
            pending_effects: vec![arm],
            ..IssueMonitorPrefs::default()
        };
        let autonomous_before = autonomous.clone();

        assert_eq!(
            autonomous.set_autonomous_mode_with_effect_revocation(true),
            Some(u64::MAX)
        );
        assert_eq!(autonomous, autonomous_before);

        let claim = PendingIssueMonitorEffect {
            effect_id: "claim-max".to_string(),
            authority_epoch: u64::MAX,
            attempt: 2,
            state: IssueMonitorEffectState::Attempting,
            payload: IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-max".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                expires_at: "2026-07-27T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        };
        let mut enabled = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: u64::MAX,
                pending_effects: vec![claim],
                ..IssueMonitorPrefs::default()
            },
        );
        let enabled_before = enabled.prefs();

        assert_eq!(
            enabled.set_enabled_with_effect_revocation(true),
            Some(u64::MAX)
        );
        assert_eq!(enabled.prefs(), enabled_before);
    }

    #[test]
    fn autonomous_off_cancels_prepared_arm_and_compensates_attempting_arm_durably() {
        // Prepared means no remote request was submitted, so OFF can cancel it.
        // Attempting is outcome-ambiguous: retain it for result reconciliation
        // and add a new-epoch disarm. Re-enabling autonomous mode must not revoke
        // that safety compensation.
        let prepared = pending_arm_effect("arm-prepared", 7, 0, IssueMonitorEffectState::Prepared);
        let attempting =
            pending_arm_effect("arm-attempting", 7, 2, IssueMonitorEffectState::Attempting);
        let mut prefs = IssueMonitorPrefs {
            autonomous_mode: true,
            effect_authority_epoch: 7,
            pending_effects: vec![prepared, attempting.clone()],
            ..IssueMonitorPrefs::default()
        };

        assert_eq!(
            prefs.set_autonomous_mode_with_effect_revocation(false),
            Some(8)
        );
        assert!(!prefs.autonomous_mode);
        assert_eq!(prefs.effect_authority_epoch, 8);
        assert!(prefs
            .pending_effects
            .iter()
            .all(|effect| effect.effect_id != "arm-prepared"));
        assert!(prefs.pending_effects.contains(&attempting));

        let compensation = prefs
            .pending_effects
            .iter()
            .find(|effect| {
                matches!(
                    &effect.payload,
                    IssueMonitorEffectPayload::DisarmAutoMerge {
                        issue_number: 7,
                        pr_number: 99,
                        compensates_effect_id,
                    } if compensates_effect_id == "arm-attempting"
                )
            })
            .cloned()
            .expect("Attempting arm receives durable disarm compensation");
        assert_eq!(compensation.authority_epoch, 8);
        assert_eq!(compensation.attempt, 0);
        assert_eq!(compensation.state, IssueMonitorEffectState::Prepared);
        assert!(!compensation.effect_id.is_empty());
        assert_ne!(compensation.effect_id, "arm-attempting");

        assert_eq!(
            prefs.set_autonomous_mode_with_effect_revocation(true),
            Some(9)
        );
        assert!(prefs.autonomous_mode);
        assert_eq!(prefs.effect_authority_epoch, 9);
        assert!(
            prefs.pending_effects.contains(&compensation),
            "a newer ON authority cannot erase an unfinished safety disarm"
        );
    }

    #[test]
    fn scan_claim_planning_is_side_effect_free_and_deduplicated() {
        // Phase 7 T-140: the cloned scan may select candidates, but GitHub
        // mutation begins only after the proposal commits and is fenced by the
        // daemon executor. Repeated scans must retain one stable proposal.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-07-27T00:00:00Z",
        );

        assert_eq!(
            monitor.prepare_claim_effects_with_probe(
                "host/session",
                "2026-07-27T00:00:01Z",
                2,
                |_| false,
            ),
            2
        );
        assert_eq!(
            monitor.prepare_claim_effects_with_probe(
                "host/session",
                "2026-07-27T00:00:02Z",
                2,
                |_| false,
            ),
            0,
            "an uncommitted/replayed scan cannot duplicate logical claims"
        );
        assert_eq!(monitor.active_count(), 0, "planning does not claim a slot");
        assert_eq!(monitor.pending_effects().len(), 2);
        assert!(monitor.pending_effects().iter().all(|effect| matches!(
            effect.payload,
            IssueMonitorEffectPayload::AcquireClaim { .. }
        )));
    }

    #[test]
    fn claim_proposal_planning_preserves_completion_probe_failure() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-07-28T00:00:00Z",
        );

        let error = monitor
            .try_prepare_claim_effects_with_probe(
                "host/session",
                "2026-07-28T00:00:01Z",
                2,
                |issue_number| {
                    if issue_number == 43 {
                        Err("merged-pr readback failed")
                    } else {
                        Ok(false)
                    }
                },
            )
            .expect_err("completion probe failure must not become false");

        assert_eq!(error, "merged-pr readback failed");
    }

    #[test]
    fn confirmed_claim_persists_and_replays_one_stable_launch_delivery() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 1,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));

        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "claim-effect-42",
            "2026-07-28T00:00:00Z",
        ));

        let prefs = monitor.prefs();
        assert_eq!(prefs.pending_launch_deliveries.len(), 1);
        assert_eq!(
            prefs.pending_launch_deliveries[0].delivery_id,
            "launch:claim-effect-42"
        );
        assert_eq!(prefs.pending_launch_deliveries[0].claim_id, "claim-42");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Launching)
        );

        let mut restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        let first = restored.take_pending_launch_requests();
        let replay = restored.take_pending_launch_requests();
        assert_eq!(first, replay, "delivery remains replayable until ACK");
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0].delivery_id.as_deref(),
            Some("launch:claim-effect-42")
        );
        assert!(
            restored
                .expire_stale_unbound_launches("2026-07-28T01:00:00Z")
                .is_empty(),
            "a durable delivery is ACK-driven, not TTL-expired"
        );
    }

    /// SPEC #3165 FR-100 / T-224: pre-policy persisted delivery/request shapes
    /// remain compatible and preserve the ordinary re-engage behavior.
    #[test]
    fn legacy_launch_session_strategy_defaults_to_resume_if_safe() {
        let delivery: PendingIssueMonitorLaunchDelivery =
            serde_json::from_value(serde_json::json!({
                "delivery_id": "launch:legacy-effect",
                "issue_number": 42,
                "branch_name": "work/issue-42",
                "linked_issue_kind": "issue",
                "claim_id": "claim-42",
                "claim_owner": "host/session",
                "created_at": "2026-08-13T00:00:00Z"
            }))
            .expect("legacy delivery without launch_session_strategy");
        assert_eq!(
            delivery.launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe
        );

        let request: IssueMonitorLaunchRequest = serde_json::from_value(serde_json::json!({
            "issue_number": 42,
            "branch_name": "work/issue-42",
            "linked_issue_kind": "issue",
            "delivery_id": "launch:legacy-effect"
        }))
        .expect("legacy request without launch_session_strategy");
        assert_eq!(
            request.launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe
        );
    }

    /// SPEC #3165 FR-100/FR-101 / T-224: a normal first delivery has no
    /// failover/failure origin and therefore retains exact-resume continuity.
    #[test]
    fn ordinary_confirmed_claim_uses_resume_if_safe() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-08-13T00:00:00Z",
        ));

        assert_eq!(
            monitor.prefs().pending_launch_deliveries[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe
        );
        assert_eq!(
            monitor.take_pending_launch_requests()[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::ResumeIfSafe
        );
    }

    #[test]
    fn deduped_confirmed_claim_returns_its_exact_delivery_not_the_queue_tail() {
        use gwt_github::{
            issue_auto_claim::render_claim_comment, CommentId, CommentSnapshot, FakeIssueClient,
            IssueSnapshot, IssueState, UpdatedAt,
        };

        let owner = "host/session";
        let now = "2026-07-28T00:00:00Z";
        let claim_id = format!("gwt-auto-improve:{owner}:42:{now}");
        let synchronous_effect_id = format!("synchronous-claim:{claim_id}");
        let expected_delivery_id = format!("launch:{synchronous_effect_id}");
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.record_candidate(issue(43));
        assert!(monitor.apply_confirmed_claim(
            42,
            claim_id.clone(),
            owner,
            &synchronous_effect_id,
            now,
        ));
        assert!(monitor.apply_confirmed_claim(43, "claim-43", owner, "effect-43", now,));
        monitor
            .active_launches
            .retain(|issue_number| *issue_number != 42);
        monitor.launching_claimed_at.remove(&42);
        monitor.set_inbox_state(42, MonitorInboxState::Queued);
        monitor.queue.push_front(42);

        let client = FakeIssueClient::new();
        let claim = ClaimComment {
            comment_id: Some(CommentId(9)),
            claim_id,
            owner: owner.to_string(),
            issue_number: 42,
            status: ClaimStatus::Active,
            heartbeat_at: now.to_string(),
            expires_at: "2026-07-28T00:30:00Z".to_string(),
            launched_work_id: Some("work/issue-42".to_string()),
        };
        client.seed(IssueSnapshot {
            number: IssueNumber(42),
            title: "Issue 42".to_string(),
            body: String::new(),
            labels: Vec::new(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new("t1"),
            comments: vec![CommentSnapshot {
                id: CommentId(9),
                body: render_claim_comment(&claim),
                updated_at: UpdatedAt::new("t1"),
            }],
        });

        let requests = monitor.claim_next_launch_requests_with_active_cap(&client, owner, now, 2);

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].issue_number, 42);
        assert_eq!(
            requests[0].delivery_id.as_deref(),
            Some(expected_delivery_id.as_str())
        );
    }

    #[test]
    fn matching_launch_ack_consumes_only_its_delivery_and_legacy_ack_consumes_none() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        monitor.record_candidate(issue(43));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));
        assert!(monitor.apply_confirmed_claim(
            43,
            "claim-43",
            "host/session",
            "effect-43",
            "2026-07-28T00:00:00Z",
        ));

        assert!(!monitor.complete_active_launch_delivery(
            42,
            "tab-1::wrong",
            Some("launch:effect-43"),
        ));
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 2);

        assert!(!monitor.complete_active_launch_delivery(
            42,
            "tab-1::agent-42",
            Some("launch:effect-42"),
        ));
        assert!(monitor.claim_launch_delivery(
            42,
            "launch:effect-42",
            "gui-a",
            101,
            "tab-1::agent-42",
            |_| false,
        ));
        assert!(!monitor.complete_active_launch_delivery(
            42,
            "tab-1::agent-42",
            Some("launch:effect-42"),
        ));
        assert!(monitor.mark_launch_delivery_materialized(
            42,
            "launch:effect-42",
            "gui-a",
            "tab-1::agent-42",
        ));
        assert!(!monitor.complete_active_launch_delivery(
            42,
            "tab-1::agent-42",
            Some("launch:effect-42"),
        ));
        assert!(monitor.mark_launch_delivery_workspace_durable(
            42,
            "launch:effect-42",
            "gui-a",
            "tab-1::agent-42",
        ));
        assert!(!monitor.complete_active_launch_delivery(
            42,
            "tab-1::wrong-window",
            Some("launch:effect-42"),
        ));
        assert!(monitor.complete_active_launch_delivery(
            42,
            "tab-1::agent-42",
            Some("launch:effect-42"),
        ));
        let acked = monitor.prefs();
        assert_eq!(acked.pending_launch_deliveries.len(), 1);
        assert_eq!(
            acked.launched_claims.get(&42).map(String::as_str),
            Some("claim-42"),
            "the consumed delivery claim becomes the durable window generation"
        );
        assert_eq!(acked.pending_launch_deliveries[0].issue_number, 43);

        assert!(monitor.complete_active_launch_delivery(43, "tab-1::legacy-43", None));
        assert!(!monitor.prefs().launched_claims.contains_key(&43));
        assert_eq!(
            monitor.prefs().pending_launch_deliveries.len(),
            1,
            "legacy ACK must not consume a new delivery"
        );
        assert!(monitor.claim_launch_delivery(
            43,
            "launch:effect-43",
            "gui-b",
            202,
            "tab-1::agent-43",
            |_| false,
        ));
        assert!(!monitor.record_launch_failed_delivery(
            43,
            "materialization failed",
            Some("launch:effect-43"),
            Some("gui-a"),
        ));
        assert!(monitor.record_launch_failed_delivery(
            43,
            "materialization failed",
            Some("launch:effect-43"),
            Some("gui-b"),
        ));
        assert!(monitor.prefs().pending_launch_deliveries.is_empty());
    }

    #[test]
    fn launch_delivery_materializer_claim_is_single_owner_and_dead_owner_is_recoverable() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));

        assert!(monitor.claim_launch_delivery(
            42,
            "launch:effect-42",
            "gui-a",
            101,
            "tab-a::agent-1",
            |pid| pid == 101,
        ));
        assert!(!monitor.claim_launch_delivery(
            42,
            "launch:effect-42",
            "gui-b",
            202,
            "tab-b::agent-1",
            |pid| pid == 101,
        ));
        let first = monitor.prefs().pending_launch_deliveries.remove(0);
        assert_eq!(first.materializer_id.as_deref(), Some("gui-a"));
        assert_eq!(
            first.materializer_window_id.as_deref(),
            Some("tab-a::agent-1")
        );

        assert!(monitor.claim_launch_delivery(
            42,
            "launch:effect-42",
            "gui-b",
            202,
            "tab-b::agent-1",
            |_| false,
        ));
        let recovered = monitor.prefs().pending_launch_deliveries.remove(0);
        assert_eq!(recovered.materializer_id.as_deref(), Some("gui-b"));
        assert_eq!(recovered.materializer_pid, Some(202));
        assert_eq!(
            recovered.materializer_window_id.as_deref(),
            Some("tab-b::agent-1")
        );
    }

    #[test]
    fn foreign_materializer_cannot_confirm_existing_materialized_or_durable_markers() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));
        assert!(monitor.claim_launch_delivery(
            42,
            "launch:effect-42",
            "gui-a",
            101,
            "tab-a::agent-42",
            |_| false,
        ));
        assert!(monitor.mark_launch_delivery_materialized(
            42,
            "launch:effect-42",
            "gui-a",
            "tab-a::agent-42",
        ));

        assert!(
            !monitor.mark_launch_delivery_materialized(
                42,
                "launch:effect-42",
                "gui-b",
                "tab-a::agent-42",
            ),
            "an idempotent materialized marker is still bound to the exact materializer"
        );
        assert!(monitor.mark_launch_delivery_workspace_durable(
            42,
            "launch:effect-42",
            "gui-a",
            "tab-a::agent-42",
        ));
        assert!(
            !monitor.mark_launch_delivery_workspace_durable(
                42,
                "launch:effect-42",
                "gui-b",
                "tab-a::agent-42",
            ),
            "an idempotent durable marker is still bound to the exact materializer"
        );
    }

    #[test]
    fn autonomous_legacy_failure_cannot_leave_one_issue_queued_and_replayable() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 2,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));
        let before = monitor.prefs();

        assert!(
            !monitor.record_launch_failed_delivery(42, "stale legacy failure", None, None),
            "a legacy failure cannot mutate an exact durable delivery without its identity"
        );

        assert_eq!(monitor.prefs(), before);
        assert_eq!(
            monitor.prepare_claim_effects_with_probe(
                "host/session",
                "2026-07-28T01:00:00Z",
                2,
                |_| false,
            ),
            0,
            "the retained outbox remains the only launch path"
        );
    }

    #[test]
    fn autonomous_direct_launch_failure_requeues_without_replaying_old_delivery() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 2,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(issue(42));
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);

        monitor.record_launch_failed(42, "legacy autonomous launch failure");

        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "the autonomous failure is scheduled through the retry path"
        );
        assert_eq!(monitor.attempt_count(42), 1);
        let record = monitor.autonomous_record(42).expect("retry record");
        assert_eq!(record.phase, AutonomousPhase::Idle);
        assert!(record.retry_not_before.is_some());
        assert!(monitor.queue.contains(&42));
        assert!(!monitor.active_launches.contains(&42));
        assert!(
            monitor.prefs().pending_launch_deliveries.is_empty(),
            "a retryable issue cannot also replay its previous durable delivery"
        );
    }

    /// SPEC #3165 FR-100/FR-101 / T-224: a generic AgentFailed autonomous
    /// retry still uses the existing attempt/backoff ladder, but its successor
    /// delivery must be fresh and must retain that strategy across restart and
    /// ACK-driven delivery replay.
    #[test]
    fn autonomous_agent_failed_retry_keeps_fresh_strategy_across_reload() {
        let candidate = auto_issue(
            42,
            "## Acceptance Criteria\n- [ ] AC-1: replacement agent completes the work\n",
        );
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(candidate.clone());
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-08-13T00:00:00Z",
        ));

        monitor.record_agent_issue_failed(42, "agent process exited");

        assert_eq!(
            monitor.attempt_count(42),
            1,
            "generic AgentFailed keeps the existing autonomous retry budget contract"
        );
        assert!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.as_ref())
                .is_some(),
            "generic AgentFailed keeps the existing bounded backoff contract"
        );
        assert_ne!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman),
            "one retry below the cap remains retryable"
        );

        let retry_at = chrono::DateTime::parse_from_rfc3339(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.as_deref())
                .expect("AgentFailed retry has a durable backoff floor"),
        )
        .expect("retry_not_before is RFC3339")
            + chrono::Duration::seconds(1);
        let retry_at = retry_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        scan_issue_monitor_candidates(&mut restored, std::slice::from_ref(&candidate), &retry_at);
        let branch_protection = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        assert_eq!(
            restored.prepare_autonomous_candidate(&candidate, &branch_protection, &retry_at),
            EligibilityDecision::Eligible
        );
        assert!(restored.apply_confirmed_claim(
            42,
            "claim-42-retry",
            "host/session",
            "effect-42-retry",
            &retry_at,
        ));

        let durable = restored.prefs();
        assert_eq!(durable.autonomous_records[0].attempts, 1);
        assert_eq!(
            durable.pending_launch_deliveries[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired
        );
        let mut replayed = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), durable);
        let first = replayed.take_pending_launch_requests();
        let second = replayed.take_pending_launch_requests();
        assert_eq!(
            first, second,
            "an unacked AgentFailed retry remains replayable"
        );
        assert_eq!(
            first[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired
        );
    }

    #[test]
    fn autonomous_exact_delivery_failure_consumes_outbox_before_retrying() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 2,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(issue(42));
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));
        assert!(monitor.claim_launch_delivery(
            42,
            "launch:effect-42",
            "gui-a",
            101,
            "tab-a::agent-42",
            |_| false,
        ));

        assert!(monitor.record_launch_failed_delivery(
            42,
            "exact autonomous launch failure",
            Some("launch:effect-42"),
            Some("gui-a"),
        ));

        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert!(monitor.queue.contains(&42));
        assert!(monitor.prefs().pending_launch_deliveries.is_empty());
    }

    #[test]
    fn disabling_monitor_compensates_durable_launch_claim_before_clearing_outbox() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));

        assert_eq!(monitor.set_enabled_with_effect_revocation(false), Some(1));
        assert!(monitor.prefs().pending_launch_deliveries.is_empty());
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 42,
                claim_id,
                owner,
            } if claim_id == "claim-42" && owner == "host/session"
        )));
    }

    #[test]
    fn autonomous_legacy_launch_failure_does_not_consume_durable_delivery() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(issue(42));
        monitor.record_attempt(42);
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));

        assert!(!monitor.record_launch_failed_delivery(42, "legacy failure", None, None));
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);
        assert_eq!(
            monitor.prefs().pending_launch_deliveries[0].delivery_id,
            "launch:effect-42"
        );
    }

    #[test]
    fn claim_proposals_use_distinct_uuid_identity_and_preserve_it_across_retry_restart() {
        fn prepare(owner: &str) -> IssueMonitorState {
            let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
                enabled: true,
                max_active: 1,
                ..IssueMonitorConfig::default()
            });
            monitor.set_gui_connected(true);
            scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-07-27T00:00:00Z");
            assert_eq!(
                monitor
                    .prepare_claim_effects_with_probe(owner, "2026-07-27T00:00:01Z", 1, |_| false,),
                1
            );
            monitor
        }

        fn identity(effect: &PendingIssueMonitorEffect) -> (&str, &str) {
            let IssueMonitorEffectPayload::AcquireClaim { claim_id, .. } = &effect.payload else {
                panic!("expected claim proposal");
            };
            (&effect.effect_id, claim_id)
        }

        let mut first = prepare("host-a/session-a");
        let second = prepare("host-b/session-b");
        let first_effect = first.pending_effects()[0].clone();
        let second_effect = second.pending_effects()[0].clone();
        let (first_effect_id, first_claim_id) = identity(&first_effect);
        let (second_effect_id, second_claim_id) = identity(&second_effect);

        assert_ne!(first_effect_id, second_effect_id);
        assert_ne!(first_claim_id, second_claim_id);
        let effect_uuid = first_effect_id
            .rsplit(':')
            .next()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .expect("effect identity ends in a UUID");
        let claim_uuid = first_claim_id
            .strip_prefix("gwt-auto-improve:")
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .expect("claim identity carries a UUID");
        assert_eq!(effect_uuid, claim_uuid, "one UUID identifies the proposal");

        let first_key = first_effect.attempt_key();
        assert!(first.mark_pending_effect_attempting(&first_key));
        assert!(first.retry_pending_effect(&first_key));
        assert_eq!(
            identity(&first.pending_effects()[0]),
            (first_effect_id, first_claim_id)
        );

        let encoded = serde_json::to_string(&first.prefs()).expect("prefs serialize");
        let restored_prefs: IssueMonitorPrefs =
            serde_json::from_str(&encoded).expect("prefs deserialize");
        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), restored_prefs);
        assert_eq!(
            identity(&restored.pending_effects()[0]),
            (first_effect_id, first_claim_id)
        );
    }

    #[test]
    fn disabling_monitor_cancels_prepared_claim_and_compensates_attempting_claim() {
        let prepared = PendingIssueMonitorEffect::prepared(
            "claim-prepared",
            3,
            IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-prepared".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                expires_at: "2026-07-27T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        );
        let mut attempting = PendingIssueMonitorEffect::prepared(
            "claim-attempting",
            3,
            IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 43,
                claim_id: "claim-attempting".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                expires_at: "2026-07-27T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-43".to_string()),
            },
        );
        attempting.state = IssueMonitorEffectState::Attempting;
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: 3,
                pending_effects: vec![prepared, attempting.clone()],
                ..IssueMonitorPrefs::default()
            },
        );

        assert_eq!(monitor.set_enabled_with_effect_revocation(false), Some(4));
        assert!(!monitor.config.enabled);
        assert!(monitor.pending_effects().contains(&attempting));
        assert!(!monitor
            .pending_effects()
            .iter()
            .any(|effect| effect.effect_id == "claim-prepared"));
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 43,
                claim_id,
                owner,
            } if claim_id == "claim-attempting" && owner == "host/session"
        )));

        let encoded = serde_json::to_string(&monitor.prefs()).expect("prefs serialize");
        let restored: IssueMonitorPrefs =
            serde_json::from_str(&encoded).expect("prefs deserialize after restart");
        assert!(restored.pending_effects.iter().any(|effect| matches!(
            &effect.payload,
            IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 43,
                claim_id,
                owner,
            } if claim_id == "claim-attempting" && owner == "host/session"
        )));
    }

    #[test]
    fn ownerless_legacy_release_journal_deserializes_without_being_dropped() {
        let prefs: IssueMonitorPrefs = serde_json::from_str(
            r#"{
                "enabled": false,
                "max_active_agents": 1,
                "priority_order": [],
                "pending_effects": [{
                    "effect_id": "release:legacy",
                    "authority_epoch": 4,
                    "attempt": 2,
                    "state": "attempting",
                    "payload": {
                        "kind": "release_claim",
                        "issue_number": 42,
                        "claim_id": "stable-effect-claim"
                    }
                }]
            }"#,
        )
        .expect("legacy ownerless journal remains readable");

        assert!(matches!(
            prefs.pending_effects.as_slice(),
            [PendingIssueMonitorEffect {
                payload: IssueMonitorEffectPayload::ReleaseClaim {
                    issue_number: 42,
                    claim_id,
                    owner,
                },
                ..
            }] if claim_id == "stable-effect-claim" && owner.is_empty()
        ));
    }

    #[test]
    fn resolve_review_model_prefers_different_configured_model() {
        // Configured + different from implementer ⇒ use it (no self-grade).
        assert_eq!(
            resolve_review_model(Some("claude-opus"), Some("claude-sonnet")),
            Some("claude-sonnet".to_string()),
        );
        // Configured == implementer ⇒ None (would be a self-grade).
        assert_eq!(
            resolve_review_model(Some("claude-opus"), Some("claude-opus")),
            None
        );
        assert_eq!(
            resolve_review_model(Some("OPUS"), Some("opus")),
            None,
            "case-insensitive"
        );
        // Unset / empty ⇒ None (fall back to saved model, still fresh session).
        assert_eq!(resolve_review_model(Some("claude-opus"), None), None);
        assert_eq!(resolve_review_model(Some("claude-opus"), Some("  ")), None);
        // No implementer model known ⇒ use the configured one.
        assert_eq!(
            resolve_review_model(None, Some("claude-sonnet")),
            Some("claude-sonnet".to_string()),
        );
    }

    #[test]
    fn pre_autonomous_prefs_fixture_file_round_trips() {
        // SPEC #3200 FR-001/FR-023, Sc 23: the committed pre-autonomous prefs
        // fixture (no autonomous_mode / tuning / records fields) must deserialize
        // with documented defaults and preserve all existing fields.
        let fixture = include_str!("../tests/fixtures/issue_monitor_prefs_pre_autonomous.json");
        let prefs: IssueMonitorPrefs =
            serde_json::from_str(fixture).expect("pre-autonomous fixture deserializes");
        assert!(prefs.enabled);
        assert_eq!(prefs.priority_order, vec![101, 102]);
        assert_eq!(prefs.merged_issues, vec![42]);
        assert!(!prefs.autonomous_mode, "autonomous_mode defaults false");
        assert_eq!(prefs.autonomous_tuning, AutonomousTuning::default());
        assert!(
            prefs.autonomous_records.is_empty(),
            "no records in a pre-autonomous prefs file",
        );
    }

    // SPEC #3245 FR-006 / AC-3: an Issue produced by the gwt-register-issue
    // template (mandatory `- [ ] AC-N:` checkbox block + the `auto-merge`
    // label applied by default) passes the autonomous-eligibility predicate,
    // and the explicit opt-out (no auto-merge label) stays on the human gate.
    #[test]
    fn registration_template_issue_passes_autonomous_eligibility_by_default() {
        use gwt_git::branch_protection::BranchProtectionStatus;
        let template_body = "## Summary\n\nfix the thing\n\n## Background\n\ncontext\n\n\
## Spec Status\n\nALIGNED — narrow bug\n\n## Related SPECs\n\n- None\n\n\
## Acceptance Criteria\n\n- [ ] AC-1: the failing call succeeds\n- [ ] AC-2: regression test stays GREEN\n\n\
## Expected Outcome\n\ngreen\n\n## Notes\n\n(none)\n";
        let criteria = crate::issue_monitor_gate::classify_acceptance_criteria(template_body);
        assert!(
            criteria.machine_checkable,
            "the template's AC block must classify as machine-checkable"
        );
        let verified = BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        assert_eq!(
            autonomous_eligibility(true, true, &criteria, &verified, false),
            EligibilityDecision::Eligible,
            "template defaults (AC block + auto-merge label) must be eligible"
        );
        assert_eq!(
            autonomous_eligibility(true, false, &criteria, &verified, false),
            EligibilityDecision::HumanGate("issue lacks the auto-merge label".to_string()),
            "the explicit opt-out (label removed) must stay on the human gate"
        );
    }

    #[test]
    fn autonomous_eligibility_truth_table() {
        // SPEC #3200 FR-003/004/005, Sc 2/3/4: two-stage-opt-in negatives →
        // HumanGate; all → Eligible. Issue #3944 AC-1/AC-4: readiness-style
        // preconditions (criteria not machine-checkable, branch protection not
        // verified) are `NotReady` with a named reason — never NeedsHuman.
        use crate::issue_monitor_gate::AcceptanceCriteria;
        use gwt_git::branch_protection::BranchProtectionStatus;
        let ok = AcceptanceCriteria {
            ids: vec!["AC-1".to_string()],
            machine_checkable: true,
            visual_surface: false,
            defect: None,
        };
        let no_criteria = AcceptanceCriteria {
            ids: vec![],
            machine_checkable: false,
            visual_surface: false,
            defect: Some(crate::issue_monitor_gate::AcceptanceDefect::MissingHeading),
        };
        let verified = BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let absent = BranchProtectionStatus::Absent;
        let unreadable = BranchProtectionStatus::Unreadable("403".to_string());

        assert_eq!(
            autonomous_eligibility(true, true, &ok, &verified, false),
            EligibilityDecision::Eligible
        );
        // (i)/(ii) opt-in negatives → HumanGate (NOT NeedsHuman).
        assert!(matches!(
            autonomous_eligibility(false, true, &ok, &verified, false),
            EligibilityDecision::HumanGate(_)
        ));
        assert!(matches!(
            autonomous_eligibility(true, false, &ok, &verified, false),
            EligibilityDecision::HumanGate(_)
        ));
        // Issue #3944 AC-4: readiness preconditions → NotReady, naming the cause.
        match autonomous_eligibility(true, true, &no_criteria, &verified, false) {
            EligibilityDecision::NotReady(reason) => assert!(
                reason.contains("Acceptance Criteria"),
                "names the missing element: {reason}"
            ),
            other => panic!("expected NotReady, got {other:?}"),
        }
        match autonomous_eligibility(true, true, &ok, &absent, false) {
            EligibilityDecision::NotReady(reason) => assert!(
                reason.contains("branch protection"),
                "names branch protection: {reason}"
            ),
            other => panic!("expected NotReady, got {other:?}"),
        }
        match autonomous_eligibility(true, true, &ok, &unreadable, false) {
            EligibilityDecision::NotReady(reason) => {
                assert!(reason.contains("permissions"), "distinct reason: {reason}")
            }
            other => panic!("expected NotReady, got {other:?}"),
        }
        // An already-parked row stays parked; the predicate never revives it.
        assert!(matches!(
            autonomous_eligibility(true, true, &ok, &verified, true),
            EligibilityDecision::NeedsHuman(_)
        ));
    }

    /// Issue #3944 AC-7: the regression table. For every reason code that can
    /// end an autonomous attempt, the resulting row state is fixed here: only
    /// the two human-answerable kinds produce `NeedsHuman`; everything else is
    /// an automatic recovery (steering request, requeue, or `NotReady`).
    #[test]
    fn autonomous_needs_human_reason_table() {
        use crate::autonomous_handoff::{
            AutonomousExecutionContext, AutonomousHandoffOption, AutonomousQuestionHandoff,
            ExtractedQuestion,
        };
        use gwt_git::branch_protection::BranchProtectionStatus;

        const NOW: &str = "2026-09-04T00:00:00Z";
        const LATER: &str = "2026-09-04T01:00:00Z";
        let verified = BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let ac_body = "## Acceptance Criteria\n- [ ] AC-1: green\n";
        let question = |text: &str| {
            let context = AutonomousExecutionContext {
                issue_number: 42,
                session_id: "session-42".to_string(),
            };
            AutonomousQuestionHandoff::new(
                format!("handoff-{}", text.len()),
                &context,
                "claude-code",
                "AskUserQuestion",
                ExtractedQuestion {
                    question: text.to_string(),
                    options: vec![AutonomousHandoffOption {
                        label: "yes".to_string(),
                        description: String::new(),
                    }],
                },
                NOW,
            )
        };

        struct Row<'a> {
            reason_code: &'static str,
            drive: Box<dyn Fn() -> IssueMonitorState + 'a>,
            expected_state: MonitorInboxState,
            expected_kind: Option<NeedsHumanKind>,
            steering_requested: bool,
        }
        let rows = vec![
            Row {
                reason_code: "stuck/idle timeout with a live window",
                drive: Box::new(|| {
                    let mut monitor = stuck_monitor(42, NOW);
                    monitor.recover_stuck_autonomous(LATER);
                    monitor
                }),
                expected_state: MonitorInboxState::Launched,
                expected_kind: None,
                steering_requested: true,
            },
            Row {
                reason_code: "stuck/idle timeout without a window",
                drive: Box::new(|| {
                    let mut monitor = stuck_monitor(42, NOW);
                    monitor.launched_windows.remove(&42);
                    monitor.recover_stuck_autonomous(LATER);
                    monitor
                }),
                expected_state: MonitorInboxState::Queued,
                expected_kind: None,
                steering_requested: false,
            },
            Row {
                reason_code: "autonomous attempts exhausted",
                drive: Box::new(|| {
                    let mut monitor = launched_monitor(42, "tab-1::agent-1");
                    monitor.set_autonomous_mode(true);
                    monitor.autonomous_tuning.max_attempts = 1;
                    monitor.record_autonomous_failure(42, "agent exited", NOW);
                    monitor
                }),
                expected_state: MonitorInboxState::Queued,
                expected_kind: None,
                steering_requested: true,
            },
            Row {
                reason_code: "launch failed (mechanical)",
                drive: Box::new(|| {
                    let mut monitor = launched_monitor(42, "tab-1::agent-1");
                    monitor.set_autonomous_mode(true);
                    monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
                    monitor.record_launch_failed(42, "generation conflict: probe timed out");
                    monitor
                }),
                expected_state: MonitorInboxState::Queued,
                expected_kind: None,
                steering_requested: false,
            },
            Row {
                reason_code: "acceptance criteria not machine-checkable",
                drive: Box::new(|| {
                    let mut monitor = autonomous_state();
                    let issue = auto_issue(42, "## Summary\n\nno criteria block\n");
                    scan_issue_monitor_candidates(&mut monitor, std::slice::from_ref(&issue), NOW);
                    monitor.prepare_autonomous_candidate(&issue, &verified, NOW);
                    monitor
                }),
                expected_state: MonitorInboxState::NotReady,
                expected_kind: None,
                steering_requested: false,
            },
            Row {
                reason_code: "branch protection not verified",
                drive: Box::new(|| {
                    let mut monitor = autonomous_state();
                    let issue = auto_issue(42, ac_body);
                    scan_issue_monitor_candidates(&mut monitor, std::slice::from_ref(&issue), NOW);
                    monitor.prepare_autonomous_candidate(
                        &issue,
                        &BranchProtectionStatus::Absent,
                        NOW,
                    );
                    monitor
                }),
                expected_state: MonitorInboxState::NotReady,
                expected_kind: None,
                steering_requested: false,
            },
            Row {
                reason_code: "question handoff: irreversible action",
                drive: Box::new(|| {
                    let mut monitor = launched_monitor(42, "tab-1::agent-1");
                    monitor.set_autonomous_mode(true);
                    monitor.absorb_autonomous_handoffs(vec![question(
                        "Delete the stale branch work/issue-12 and its worktree?",
                    )]);
                    monitor.apply_pending_autonomous_handoffs(NOW);
                    monitor
                }),
                expected_state: MonitorInboxState::NeedsHuman,
                expected_kind: Some(NeedsHumanKind::DestructiveChangeApproval),
                steering_requested: false,
            },
            Row {
                reason_code: "question handoff: user choice",
                drive: Box::new(|| {
                    let mut monitor = launched_monitor(42, "tab-1::agent-1");
                    monitor.set_autonomous_mode(true);
                    monitor.absorb_autonomous_handoffs(vec![question(
                        "Which layout should the settings page use?",
                    )]);
                    monitor.apply_pending_autonomous_handoffs(NOW);
                    monitor
                }),
                expected_state: MonitorInboxState::NeedsHuman,
                expected_kind: Some(NeedsHumanKind::UserChoiceRequired),
                steering_requested: false,
            },
        ];

        for row in rows {
            let monitor = (row.drive)();
            assert_eq!(
                monitor.inbox_item(42).map(|item| item.state),
                Some(row.expected_state),
                "{}: inbox state",
                row.reason_code
            );
            assert_eq!(
                monitor
                    .autonomous_record(42)
                    .and_then(|record| record.needs_human_kind),
                row.expected_kind,
                "{}: needs_human kind",
                row.reason_code
            );
            assert_eq!(
                monitor
                    .autonomous_record(42)
                    .is_some_and(|record| record.steering.is_some()),
                row.steering_requested,
                "{}: steering request",
                row.reason_code
            );
            assert_eq!(
                monitor
                    .inbox_item(42)
                    .is_some_and(|item| item.state == MonitorInboxState::NeedsHuman),
                row.expected_kind.is_some(),
                "{}: NeedsHuman exactly when a human-answerable kind exists",
                row.reason_code
            );
        }
    }

    /// Issue #3944 AC-5: a destructive-change park says what is being approved
    /// on one line — the question itself, not only the generic rationale.
    #[test]
    fn destructive_handoff_park_names_the_change_on_one_line() {
        use crate::autonomous_handoff::{
            AutonomousExecutionContext, AutonomousQuestionHandoff, ExtractedQuestion,
        };
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_mode(true);
        let context = AutonomousExecutionContext {
            issue_number: 42,
            session_id: "session-42".to_string(),
        };
        monitor.absorb_autonomous_handoffs(vec![AutonomousQuestionHandoff::new(
            "handoff-1".to_string(),
            &context,
            "claude-code",
            "AskUserQuestion",
            ExtractedQuestion {
                question:
                    "Force-push work/issue-42 over the remote branch?\n\nThe remote has 3 commits."
                        .to_string(),
                options: Vec::new(),
            },
            "2026-09-04T00:00:00Z",
        )]);
        monitor.apply_pending_autonomous_handoffs("2026-09-04T00:00:01Z");
        let reason = monitor
            .status_view()
            .autonomous_issues
            .iter()
            .find(|summary| summary.issue_number == 42)
            .and_then(|summary| summary.needs_human_reason.clone())
            .expect("parked reason");
        assert!(
            reason.contains("Force-push work/issue-42 over the remote branch?"),
            "the reason carries the question: {reason}"
        );
        assert!(
            !reason.contains('\n') && !reason.contains("The remote has 3 commits"),
            "one line only: {reason}"
        );
        assert_eq!(
            monitor
                .status_view()
                .autonomous_issues
                .iter()
                .find(|summary| summary.issue_number == 42)
                .and_then(|summary| summary.needs_human_kind),
            Some(NeedsHumanKind::DestructiveChangeApproval)
        );
    }

    #[test]
    fn needs_human_is_terminal_and_not_revived_by_requeue() {
        // SPEC #3200 FR-027, Sc 12/21: NeedsHuman is terminal and a window-close
        // requeue must never revive it.
        assert!(MonitorInboxState::NeedsHuman.is_terminal());
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        if let Some(item) = monitor.inbox.iter_mut().find(|i| i.issue.number == 42) {
            item.state = MonitorInboxState::NeedsHuman;
        }
        assert_eq!(
            monitor.requeue_window("tab-1::agent-1"),
            None,
            "requeue must not revive a terminal NeedsHuman item"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
    }

    #[test]
    fn completed_spec_survives_prefs_roundtrip_and_blocks_relaunch() {
        let complete_spec = spec_issue(
            42,
            IssueMonitorReadiness::ReadyWithCompletedTasks,
            "2026-06-26T00:00:00Z",
        );
        let mut monitor = launched_monitor_for_issue(complete_spec.clone(), "tab-1::agent-1");
        monitor.record_merged(42);
        let prefs = monitor.prefs();
        assert_eq!(prefs.merged_issues, vec![42]);

        let mut restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        scan_issue_monitor_candidates(&mut restored, &[complete_spec], "2026-06-26T02:00:00Z");
        assert_eq!(
            restored.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged),
            "restored monitor must not re-launch already-merged work"
        );
        assert_eq!(restored.queue_len(), 0);
    }

    #[test]
    fn prefs_tmp_path_is_process_unique_not_a_shared_fixed_name() {
        // adversarial review (shared *.json.tmp race): the daemon and GUI both
        // write this prefs file, so the atomic-write scratch path must be unique
        // per writer — never the old fixed `*.json.tmp` that concurrent writers
        // could truncate into torn JSON.
        let path = std::path::Path::new("/x/y/issue-monitor.json");
        let a = super::unique_prefs_tmp_path(path);
        let b = super::unique_prefs_tmp_path(path);
        assert_ne!(a, b, "each write gets a distinct scratch path (uuid)");
        assert_ne!(
            a,
            path.with_extension("json.tmp"),
            "not the old shared fixed name"
        );
        assert!(
            a.to_string_lossy()
                .contains(&std::process::id().to_string()),
            "scratch path is scoped to the writing process: {}",
            a.display()
        );
        assert_eq!(
            a.parent(),
            path.parent(),
            "scratch stays in the target's dir so the rename is atomic"
        );
    }

    #[test]
    fn failed_atomic_prefs_rename_removes_unique_scratch_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("issue-monitor.json");
        fs::create_dir(&path).expect("make destination a directory");

        let error = super::save_issue_monitor_prefs(
            &path,
            &super::IssueMonitorPrefs {
                enabled: true,
                ..super::IssueMonitorPrefs::default()
            },
        )
        .expect_err("rename over a directory must fail");

        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        let scratch_prefix = ".issue-monitor.json.tmp-";
        assert!(
            fs::read_dir(temp.path())
                .expect("read tempdir")
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(scratch_prefix)),
            "failed atomic write must not leave a partial scratch file"
        );
        assert!(path.is_dir(), "failed rename leaves destination untouched");
    }

    #[test]
    fn authority_fence_clear_waits_for_the_stable_prefs_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(&prefs_path, &IssueMonitorPrefs::default()).expect("seed prefs");
        let fence = IssueMonitorAuthorityFence::current_process();
        persist_issue_monitor_authority_fence(&prefs_path, &fence).expect("seed authority fence");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let clear_path = prefs_path.clone();
        let clear_fence = fence.clone();

        let (clear_thread, completed_while_locked, fence_while_locked) =
            with_issue_monitor_prefs_lock(&prefs_path, || {
                let clear_thread = std::thread::spawn(move || {
                    started_tx.send(()).expect("signal clear start");
                    let result = clear_issue_monitor_authority_fence(&clear_path, &clear_fence);
                    done_tx.send(result).expect("signal clear completion");
                });
                started_rx
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .expect("clear thread started");
                let completed_while_locked = done_rx
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .ok();
                let fence_while_locked = load_issue_monitor_authority_fence(&prefs_path)?;
                Ok((clear_thread, completed_while_locked, fence_while_locked))
            })
            .expect("hold stable prefs lock");

        let completed_early = completed_while_locked.is_some();
        let completion = match completed_while_locked {
            Some(result) => result,
            None => done_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("clear completes after lock release"),
        };
        clear_thread.join().expect("clear thread joins");

        assert!(
            !completed_early,
            "fence clear must not compare/remove while another prefs transaction owns the lock"
        );
        assert_eq!(
            fence_while_locked,
            IssueMonitorAuthorityFenceState::Active(fence)
        );
        completion.expect("exact fence clears after lock release");
    }

    #[test]
    fn current_authority_fence_recovers_under_a_reused_live_pid_when_lifetime_lock_is_free() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(
            &prefs_path,
            &IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        persist_issue_monitor_authority_fence(
            &prefs_path,
            &IssueMonitorAuthorityFence {
                version: ISSUE_MONITOR_AUTHORITY_FENCE_VERSION,
                pid: std::process::id(),
                instance_id: "stale-owner".to_string(),
            },
        )
        .expect("seed stale current fence");
        let current = IssueMonitorAuthorityFence::current_process();

        let (prefs, lease) =
            establish_issue_monitor_authority_fence(&prefs_path, &current, |_| true)
                .expect("free lifetime lock proves that the current fence is stale");

        assert_eq!(prefs.effect_authority_epoch, 8);
        assert_eq!(
            load_issue_monitor_authority_fence(&prefs_path).expect("load replacement fence"),
            IssueMonitorAuthorityFenceState::Active(current)
        );
        drop(lease);
    }

    #[test]
    fn current_authority_fence_rejects_a_second_owner_until_the_lease_is_dropped() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(&prefs_path, &IssueMonitorPrefs::default()).expect("seed prefs");
        let first = IssueMonitorAuthorityFence::current_process();
        let (_, first_lease) =
            establish_issue_monitor_authority_fence(&prefs_path, &first, |_| false)
                .expect("first owner acquires lifetime lease");
        let second = IssueMonitorAuthorityFence::current_process();

        let overlap = establish_issue_monitor_authority_fence(&prefs_path, &second, |_| false)
            .expect_err("live lifetime lease rejects a second owner");

        assert_eq!(overlap.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            load_issue_monitor_prefs(&prefs_path)
                .expect("load prefs after rejected overlap")
                .effect_authority_epoch,
            0,
            "rejected overlap must not advance authority"
        );

        drop(first_lease);
        let (recovered, second_lease) =
            establish_issue_monitor_authority_fence(&prefs_path, &second, |_| true)
                .expect("dropping the lease makes the persisted current fence recoverable");
        assert_eq!(recovered.effect_authority_epoch, 1);
        drop(second_lease);
    }

    #[test]
    fn local_fallback_lease_rejects_live_daemon_authority() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(&prefs_path, &IssueMonitorPrefs::default()).expect("seed prefs");
        let daemon = IssueMonitorAuthorityFence::current_process();
        let (_, daemon_lease) =
            establish_issue_monitor_authority_fence(&prefs_path, &daemon, |_| false)
                .expect("daemon authority");

        let error = try_acquire_issue_monitor_local_fallback_lease(&prefs_path)
            .expect_err("live daemon authority must exclude the GUI fallback");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        drop(daemon_lease);
    }

    #[test]
    fn local_fallback_lease_recovers_an_unlocked_v2_fence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(
            &prefs_path,
            &IssueMonitorPrefs {
                effect_authority_epoch: 11,
                ..IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        persist_issue_monitor_authority_fence(
            &prefs_path,
            &IssueMonitorAuthorityFence::current_process(),
        )
        .expect("seed stale v2 fence without its lifetime lock");

        let lease = try_acquire_issue_monitor_local_fallback_lease(&prefs_path)
            .expect("a free lifetime lock makes the v2 fence recoverable");

        assert_eq!(
            load_issue_monitor_prefs(&prefs_path)
                .expect("load recovered prefs")
                .effect_authority_epoch,
            12,
            "stale daemon effects must be revoked before GUI fallback execution"
        );
        assert_eq!(
            load_issue_monitor_authority_fence(&prefs_path).expect("load recovered fence"),
            IssueMonitorAuthorityFenceState::Missing,
            "the bounded GUI lease must not leave a durable daemon fence"
        );
        drop(lease);
    }

    #[test]
    fn local_fallback_lease_blocks_daemon_start_until_drop() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(&prefs_path, &IssueMonitorPrefs::default()).expect("seed prefs");
        let local_lease = try_acquire_issue_monitor_local_fallback_lease(&prefs_path)
            .expect("GUI fallback authority");
        let daemon = IssueMonitorAuthorityFence::current_process();

        let blocked = establish_issue_monitor_authority_fence(&prefs_path, &daemon, |_| false)
            .expect_err("daemon startup must not overlap a local remote effect");
        assert_eq!(blocked.kind(), io::ErrorKind::WouldBlock);

        drop(local_lease);
        let (_, daemon_lease) =
            establish_issue_monitor_authority_fence(&prefs_path, &daemon, |_| false)
                .expect("daemon starts after the local effect boundary closes");
        drop(daemon_lease);
    }

    #[test]
    fn legacy_v1_authority_fence_fails_closed_for_a_live_pid_without_a_lifetime_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(
            &prefs_path,
            &IssueMonitorPrefs {
                effect_authority_epoch: 11,
                ..IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let legacy = IssueMonitorAuthorityFence {
            version: LEGACY_ISSUE_MONITOR_AUTHORITY_FENCE_VERSION,
            pid: 4242,
            instance_id: "legacy-live-owner".to_string(),
        };
        persist_issue_monitor_authority_fence(&prefs_path, &legacy).expect("seed v1 fence");
        let current = IssueMonitorAuthorityFence::current_process();

        let overlap =
            establish_issue_monitor_authority_fence(&prefs_path, &current, |pid| pid == legacy.pid)
                .expect_err("a live v1 owner has no lock identity, so recovery stays fail-closed");

        assert_eq!(overlap.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            load_issue_monitor_authority_fence(&prefs_path).expect("load preserved v1 fence"),
            IssueMonitorAuthorityFenceState::Active(legacy)
        );
        assert_eq!(
            load_issue_monitor_prefs(&prefs_path)
                .expect("load preserved prefs")
                .effect_authority_epoch,
            11
        );
    }

    #[test]
    fn legacy_v1_authority_fence_recovers_after_its_pid_is_dead() {
        let temp = tempfile::tempdir().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        save_issue_monitor_prefs(
            &prefs_path,
            &IssueMonitorPrefs {
                effect_authority_epoch: 19,
                ..IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let legacy = IssueMonitorAuthorityFence {
            version: LEGACY_ISSUE_MONITOR_AUTHORITY_FENCE_VERSION,
            pid: 4242,
            instance_id: "legacy-dead-owner".to_string(),
        };
        persist_issue_monitor_authority_fence(&prefs_path, &legacy).expect("seed v1 fence");
        let current = IssueMonitorAuthorityFence::current_process();

        let (prefs, lease) =
            establish_issue_monitor_authority_fence(&prefs_path, &current, |_| false)
                .expect("dead v1 owner is recoverable once the lifetime lock is free");

        assert_eq!(prefs.effect_authority_epoch, 20);
        assert_eq!(
            load_issue_monitor_authority_fence(&prefs_path).expect("load upgraded fence"),
            IssueMonitorAuthorityFenceState::Active(current)
        );
        drop(lease);
    }

    #[test]
    fn expired_deadline_rejects_durable_write_before_canonical_rename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("issue-monitor.json");
        fs::write(&path, b"original").expect("seed canonical bytes");
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() - std::time::Duration::from_millis(1),
        );

        let error = super::durable_atomic_write(&path, b"replacement")
            .expect_err("expired deadline must reject the canonical rename");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(fs::read(&path).expect("read canonical bytes"), b"original");
        let scratch_prefix = ".issue-monitor.json.tmp-";
        assert!(
            fs::read_dir(temp.path())
                .expect("read tempdir")
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(scratch_prefix)),
            "deadline rejection must clean the scratch file"
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn parent_directory_sync_is_a_non_unix_compatibility_noop() {
        let missing_parent =
            std::path::Path::new("definitely-missing-parent").join("issue-monitor.json");

        super::sync_parent_directory(&missing_parent)
            .expect("non-Unix durable writers do not open directory handles");
    }

    #[test]
    fn try_mutate_noop_preserves_noncanonical_prefs_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("issue-monitor.json");
        let original = br#"{
  "enabled": false,
  "max_active_agents": 1,
  "priority_order": [],
  "legacy_extension": { "must_survive_refusal": true }
}
"#;
        fs::write(&path, original).expect("seed noncanonical prefs");

        let (_, ()) = try_mutate_issue_monitor_prefs(&path, |_prefs| Ok(()))
            .expect("no-op transaction succeeds");

        assert_eq!(
            fs::read(&path).expect("read unchanged prefs"),
            original,
            "a refused/no-op transaction must not rewrite or drop unknown fields"
        );
    }

    #[test]
    fn save_issue_monitor_prefs_round_trips_and_leaves_no_scratch_file() {
        // The unique-scratch atomic write still round-trips and cleans up (the
        // rename consumes the temp), leaving only the target file.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("issue-monitor.json");
        let prefs = IssueMonitorPrefs {
            merged_issues: vec![7, 9],
            ..IssueMonitorPrefs::default()
        };
        save_issue_monitor_prefs(&path, &prefs).expect("save");

        let loaded = load_issue_monitor_prefs(&path).expect("load");
        assert_eq!(loaded.merged_issues, vec![7, 9]);

        let scratch_prefix = ".issue-monitor.json.tmp-";
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(scratch_prefix))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no scratch file left behind: {leftovers:?}"
        );
    }

    #[test]
    fn issue_monitor_launch_profile_round_trips_all_launch_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("issue-monitor.json");
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .model("gpt-5.5")
            .reasoning_level("high")
            .version("0.121.0")
            .session_mode(gwt_agent::SessionMode::Resume)
            .skip_permissions(true)
            .fast_mode(true)
            .runtime_target(gwt_agent::LaunchRuntimeTarget::Docker)
            .docker_service("app")
            .docker_lifecycle_intent(gwt_agent::DockerLifecycleIntent::Restart)
            .windows_shell(gwt_agent::WindowsShellKind::PowerShell7)
            .build();
        let prefs = IssueMonitorPrefs {
            launch_profile: Some(IssueMonitorLaunchProfile::from(&config)),
            ..IssueMonitorPrefs::default()
        };

        save_issue_monitor_prefs(&path, &prefs).expect("save");
        let loaded = load_issue_monitor_prefs(&path).expect("load");
        let profile = loaded.launch_profile.expect("launch profile");

        assert_eq!(profile.agent_id, "codex");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(profile.reasoning.as_deref(), Some("high"));
        assert_eq!(profile.version.as_deref(), Some("0.121.0"));
        assert_eq!(profile.session_mode, gwt_agent::SessionMode::Resume);
        assert!(profile.skip_permissions);
        assert!(profile.codex_fast_mode);
        assert_eq!(
            profile.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(profile.docker_service.as_deref(), Some("app"));
        assert_eq!(
            profile.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Restart
        );
        assert_eq!(
            profile.windows_shell,
            Some(gwt_agent::WindowsShellKind::PowerShell7)
        );

        let previous = LaunchWizardPreviousProfile::from(profile);
        assert_eq!(previous.agent_id, "codex");
        assert_eq!(previous.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(previous.reasoning.as_deref(), Some("high"));
        assert_eq!(previous.version.as_deref(), Some("0.121.0"));
        assert_eq!(previous.session_mode, gwt_agent::SessionMode::Resume);
        assert!(previous.skip_permissions);
        assert!(previous.codex_fast_mode);
        assert_eq!(
            previous.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(previous.docker_service.as_deref(), Some("app"));
        assert_eq!(
            previous.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Restart
        );
        assert_eq!(
            previous.windows_shell,
            Some(gwt_agent::WindowsShellKind::PowerShell7)
        );
    }

    #[test]
    fn autonomous_phase_defaults_idle() {
        // SPEC #3200 T-022: an issue with no autonomous record reports no record,
        // and a freshly created record starts Idle.
        assert_eq!(AutonomousPhase::default(), AutonomousPhase::Idle);
        let monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert!(monitor.autonomous_record(42).is_none());
        assert_eq!(monitor.attempt_count(42), 0);
    }

    #[test]
    fn attempt_counter_increments_and_clears() {
        // SPEC #3200 T-016 / FR-026: a per-issue attempt counter increments on
        // each attempt and resets when the record is cleared (success/merge).
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert_eq!(monitor.record_attempt(7), 1);
        assert_eq!(monitor.record_attempt(7), 2);
        assert_eq!(monitor.attempt_count(7), 2);
        assert_eq!(monitor.attempt_count(8), 0, "other issues are independent");
        monitor.clear_autonomous_record(7);
        assert_eq!(monitor.attempt_count(7), 0, "clear resets the counter");
        assert!(monitor.autonomous_record(7).is_none());
    }

    #[test]
    fn autonomous_record_tracks_phase_launch_id_and_snapshot() {
        // SPEC #3200 T-022/T-018: phase, the active launch id binding the current
        // attempt, and the acceptance snapshot are all tracked per issue.
        use crate::issue_monitor_gate::classify_acceptance_criteria;
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_phase(9, AutonomousPhase::Implementing);
        monitor.set_active_launch_id(9, Some("tab-1::agent-9".to_string()));
        let snapshot =
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n").snapshot();
        monitor.capture_acceptance_snapshot(9, snapshot.clone());

        let record = monitor.autonomous_record(9).expect("record exists");
        assert_eq!(record.phase, AutonomousPhase::Implementing);
        assert_eq!(record.active_launch_id.as_deref(), Some("tab-1::agent-9"));
        assert_eq!(record.acceptance_snapshot.as_ref(), Some(&snapshot));

        monitor.set_active_launch_id(9, None);
        assert_eq!(
            monitor
                .autonomous_record(9)
                .and_then(|r| r.active_launch_id.clone()),
            None,
            "active launch id clears when the attempt's launch ends"
        );
    }

    #[test]
    fn transient_failure_under_cap_retries_and_counts() {
        // SPEC #3200 T-042/FR-026: a transient failure below max_attempts
        // re-queues the issue for resume and increments the attempt counter.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        monitor.set_active_launch_id(42, Some("tab-1::agent-1".to_string()));

        assert_eq!(
            monitor.record_autonomous_failure(42, "network blip", "2026-06-29T00:00:00Z"),
            AutonomousFailureOutcome::Retry { attempt: 1 }
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "transient retry re-queues (never a fake done state)"
        );
        assert_eq!(monitor.attempt_count(42), 1);
        assert_eq!(monitor.active_count(), 0, "slot freed for the retry");
        assert_eq!(
            monitor
                .autonomous_record(42)
                .map(|r| r.active_launch_id.clone()),
            Some(None),
            "the in-flight launch id is cleared on retry"
        );
        // T-043: the retry is scheduled for the future (bounded backoff), so the
        // issue is not eligible to relaunch immediately, but is once time passes.
        assert!(
            !monitor.retry_ready(42, "2026-06-29T00:00:00Z"),
            "not relaunchable before the backoff elapses"
        );
        assert!(
            monitor.retry_ready(42, "2026-06-29T01:00:00Z"),
            "relaunchable once the backoff window passes"
        );
    }

    #[test]
    fn transient_failure_at_cap_requeues_and_requests_steering() {
        // Issue #3944 AC-2: reaching max_attempts is not a human decision. The
        // retry ladder continues at its capped backoff and the PM is asked to
        // steer; the row is Queued, not NeedsHuman.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_mode(true);
        monitor.autonomous_tuning.max_attempts = 2;
        assert_eq!(
            monitor.record_autonomous_failure(42, "fail 1", "2026-06-29T00:00:00Z"),
            AutonomousFailureOutcome::Retry { attempt: 1 }
        );
        assert!(
            monitor
                .autonomous_record(42)
                .is_some_and(|record| record.steering.is_none()),
            "under the cap the plain retry ladder needs no steering"
        );
        // Re-launch the retried attempt, then fail again at the cap.
        monitor.complete_active_launch(42, "tab-1::agent-1b");
        assert_eq!(
            monitor.record_autonomous_failure(42, "fail 2", "2026-06-29T00:30:00Z"),
            AutonomousFailureOutcome::Retry { attempt: 2 }
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert_eq!(
            monitor.autonomous_record(42).map(|r| r.phase),
            Some(AutonomousPhase::Idle)
        );
        assert_eq!(monitor.active_count(), 0, "slot freed for the retry");
        let steering = monitor
            .autonomous_record(42)
            .and_then(|record| record.steering.clone())
            .expect("exhaustion asks the PM to steer");
        assert!(
            steering.reason.contains("exhausted") && steering.reason.contains("fail 2"),
            "the request names exhaustion and the last failure: {steering:?}"
        );
        assert!(
            !monitor.retry_ready(42, "2026-06-29T00:30:01Z"),
            "the capped backoff still throttles the next launch"
        );
    }

    #[test]
    fn retry_backoff_is_exponential_and_capped() {
        // SPEC #3200 T-043/FR-029: the transient-retry delay grows exponentially
        // per attempt and is clamped to the configured cap.
        assert_eq!(autonomous_retry_backoff_secs(1, 60, 1800), 60);
        assert_eq!(autonomous_retry_backoff_secs(2, 60, 1800), 120);
        assert_eq!(autonomous_retry_backoff_secs(3, 60, 1800), 240);
        assert_eq!(
            autonomous_retry_backoff_secs(6, 60, 1800),
            1800,
            "clamped to cap"
        );
        assert_eq!(
            autonomous_retry_backoff_secs(100, 60, 1800),
            1800,
            "no overflow at large attempt counts"
        );
        assert_eq!(
            autonomous_retry_backoff_secs(0, 60, 1800),
            60,
            "attempt 0 floors at base"
        );
    }

    #[test]
    fn retry_ready_defaults_true_without_a_schedule() {
        // An issue with no pending retry schedule is always relaunch-ready, and an
        // unparseable clock fails open (never permanently blocks a retry).
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        assert!(monitor.retry_ready(42, "2026-06-29T00:00:00Z"));
        monitor.record_autonomous_failure(42, "blip", "2026-06-29T00:00:00Z");
        assert!(
            monitor.retry_ready(42, "not-a-timestamp"),
            "unparseable now fails open"
        );
    }

    fn stuck_monitor(number: u64, launched_at: &str) -> IssueMonitorState {
        let mut monitor = launched_monitor(number, "tab-1::agent-1");
        // Stuck recovery is an autonomous-only feature (guarded by autonomous_mode).
        monitor.set_autonomous_mode(true);
        monitor.autonomous_tuning.stuck_timeout_secs = 1800;
        monitor.set_autonomous_phase(number, AutonomousPhase::Implementing);
        monitor.set_active_launch_id(number, Some("tab-1::agent-1".to_string()));
        monitor.record_autonomous_heartbeat(number, launched_at);
        monitor
    }

    #[test]
    fn stuck_detection_flags_idle_agent_past_timeout() {
        // SPEC #3200 T-044/T-035/FR-013: a launched autonomous agent with no
        // heartbeat past stuck_timeout_secs is stuck; a fresh heartbeat is not.
        let monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        // 20 min later (< 30 min timeout) ⇒ not yet stuck.
        assert!(monitor
            .stuck_autonomous_issues("2026-06-29T00:20:00Z")
            .is_empty());
        // 31 min later (> 30 min timeout) ⇒ stuck.
        assert_eq!(
            monitor.stuck_autonomous_issues("2026-06-29T00:31:00Z"),
            vec![42]
        );
    }

    #[test]
    fn stuck_detection_ignores_pipeline_in_flight() {
        // SPEC #3200 T-044: once review / Deliver is in flight, the merge-watch
        // timeout governs — a stale agent heartbeat must NOT reclaim the slot.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.set_autonomous_phase(42, AutonomousPhase::Reviewing);
        assert!(
            monitor
                .stuck_autonomous_issues("2026-06-29T02:00:00Z")
                .is_empty(),
            "Reviewing is pipeline-in-flight, not stuck"
        );
    }

    #[test]
    fn recover_stuck_returns_to_queued_and_is_idempotent() {
        // SPEC #3200 T-044/T-045: recovery of a lost launch (no window bound)
        // reclaims the stuck slot and resumes (Queued); a second pass finds
        // nothing (idempotent). Issue #3944 AC-2: a live window is steered
        // instead — see `recover_stuck_with_a_live_window_requests_steering…`.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.launched_windows.remove(&42);
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T01:00:00Z");
        assert_eq!(recovered.len(), 1);
        assert!(matches!(
            recovered[0],
            (42, AutonomousFailureOutcome::Retry { attempt: 1 })
        ));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert_eq!(monitor.active_count(), 0, "stuck slot reclaimed");
        assert!(
            monitor
                .recover_stuck_autonomous("2026-06-29T01:05:00Z")
                .is_empty(),
            "no longer launched ⇒ idempotent"
        );
    }

    #[test]
    fn recover_stuck_with_a_live_window_requests_steering_instead_of_reclaiming() {
        // Issue #3944 AC-2: a stuck agent whose window is still alive is not
        // torn down — the PM is asked to steer it. No attempt is consumed, the
        // slot stays occupied, and NeedsHuman is never derived from a stall.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.autonomous_tuning.max_attempts = 1;
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T01:00:00Z");
        assert!(matches!(
            recovered.as_slice(),
            [(42, AutonomousFailureOutcome::SteeringRequested { count: 1 })]
        ));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Launched)
        );
        assert_eq!(monitor.active_count(), 1, "the live window keeps its slot");
        assert_eq!(
            monitor.attempt_count(42),
            0,
            "a stall is not a failed attempt"
        );
        let steering = monitor
            .autonomous_record(42)
            .and_then(|record| record.steering.clone())
            .expect("steering request recorded");
        assert!(
            steering.reason.contains("stuck/idle timeout"),
            "{steering:?}"
        );
        assert_eq!(steering.requested_at, "2026-06-29T01:00:00Z");
        let notices = monitor.take_autonomous_notices();
        assert!(
            notices
                .iter()
                .any(|notice| notice.level == "warn" && notice.message.contains("steering")),
            "the PM-facing notice is raised: {notices:?}"
        );
        // The request is re-raised only after another full stuck window.
        assert!(
            monitor
                .recover_stuck_autonomous("2026-06-29T01:10:00Z")
                .is_empty(),
            "a fresh request suspends the stuck rule for one more window"
        );
        assert!(matches!(
            monitor
                .recover_stuck_autonomous("2026-06-29T01:31:00Z")
                .as_slice(),
            [(42, AutonomousFailureOutcome::SteeringRequested { count: 2 })]
        ));
        // Progress after steering clears the request.
        monitor.record_autonomous_heartbeat(42, "2026-06-29T01:32:00Z");
        assert!(monitor
            .autonomous_record(42)
            .is_some_and(|record| record.steering.is_none()));
    }

    #[test]
    fn recover_stuck_without_a_window_requeues_even_past_the_attempt_cap() {
        // Issue #3944 AC-2: no live window ⇒ the stall is a lost launch; it is
        // requeued (fresh launch) — never NeedsHuman, even at the attempt cap.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.launched_windows.remove(&42);
        monitor.autonomous_tuning.max_attempts = 1;
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T01:00:00Z");
        assert!(matches!(
            recovered.as_slice(),
            [(42, AutonomousFailureOutcome::Retry { attempt: 1 })]
        ));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert_eq!(monitor.active_count(), 0, "the lost launch frees its slot");
    }

    #[test]
    fn wait_declaration_suspends_stuck_detection_and_preserves_attempts() {
        // Issue #3844 AC-1/AC-4: a launched agent that declares it is waiting
        // (host exclusivity, PM serialization, a long verify run) is not stuck
        // while the declaration is in force, and no attempt is consumed.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        assert!(matches!(
            monitor.declare_autonomous_wait(
                42,
                "host 排他の順番待ち",
                "#3791 の verify が完了する",
                "2026-06-29T00:10:00Z",
            ),
            AutonomousWaitOutcome::Declared { .. }
        ));
        // Two hours later the heartbeat is far past stuck_timeout_secs.
        assert!(monitor
            .stuck_autonomous_issues("2026-06-29T02:00:00Z")
            .is_empty());
        assert!(monitor
            .recover_stuck_autonomous("2026-06-29T02:00:00Z")
            .is_empty());
        assert_eq!(
            monitor.attempt_count(42),
            0,
            "waiting never costs an attempt"
        );
        assert_eq!(
            monitor.active_count(),
            1,
            "the slot stays with the waiting agent"
        );
    }

    #[test]
    fn wait_declaration_expires_at_the_cap_and_redeclaration_does_not_extend_it() {
        // Issue #3844 AC-3: a declaration cannot hold stuck detection off
        // forever. Past AUTONOMOUS_WAIT_MAX_SECS the ordinary rule applies, and
        // re-declaring keeps the original `since` so renewals cannot chain.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.declare_autonomous_wait(
            42,
            "順番待ち",
            "前の agent の完了",
            "2026-06-29T00:10:00Z",
        );
        assert!(matches!(
            monitor.declare_autonomous_wait(42, "順番待ち (更新)", "同上", "2026-06-29T02:00:00Z"),
            AutonomousWaitOutcome::Declared { ref since, .. } if since == "2026-06-29T00:10:00Z"
        ));
        assert!(
            monitor
                .stuck_autonomous_issues("2026-06-29T03:05:00Z")
                .is_empty(),
            "still inside the 3h cap"
        );
        assert_eq!(
            monitor.stuck_autonomous_issues("2026-06-29T03:11:00Z"),
            vec![42],
            "past the cap the stale heartbeat counts again"
        );
    }

    #[test]
    fn wait_declaration_is_cleared_when_the_launch_ends() {
        // Issue #3844: the declaration belongs to one launch. Once the launch is
        // torn down (here: stuck recovery of a lost launch after the cap) the
        // next launch must start without an inherited wait.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.declare_autonomous_wait(
            42,
            "順番待ち",
            "前の agent の完了",
            "2026-06-29T00:10:00Z",
        );
        monitor.launched_windows.remove(&42);
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T03:20:00Z");
        assert!(matches!(
            recovered.as_slice(),
            [(42, AutonomousFailureOutcome::Retry { attempt: 1 })]
        ));
        assert!(monitor.autonomous_wait(42).is_none());
    }

    #[test]
    fn wait_declaration_requires_a_live_launch() {
        // Issue #3844: only the agent that owns a launch can declare a wait for
        // it; a queued or parked row has nothing to suspend.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_mode(true);
        assert_eq!(
            monitor.declare_autonomous_wait(42, "順番待ち", "同上", "2026-06-29T00:10:00Z"),
            AutonomousWaitOutcome::NotLaunched
        );
        assert!(monitor.autonomous_wait(42).is_none());
        assert_eq!(
            monitor.clear_autonomous_wait(42, "2026-06-29T00:11:00Z"),
            AutonomousWaitOutcome::NotWaiting
        );
    }

    #[test]
    fn agent_status_projects_the_wait_declaration() {
        // Issue #3844 AC-2: the PM reads what the agent is waiting for, since
        // when, and when the declaration stops protecting it, from the same
        // `issue.monitor.status` row that carries last_activity_at.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.declare_autonomous_wait(
            42,
            "host 排他の順番待ち",
            "#3791 の verify が完了する",
            "2026-06-29T00:10:00Z",
        );
        let row = |monitor: &IssueMonitorState| {
            monitor
                .agent_status()
                .inbox
                .into_iter()
                .find(|row| row.issue_number == 42)
                .expect("row 42")
        };
        let waiting = row(&monitor).waiting.expect("waiting projected");
        assert_eq!(waiting.reason, "host 排他の順番待ち");
        assert_eq!(waiting.resume_condition, "#3791 の verify が完了する");
        assert_eq!(waiting.since, "2026-06-29T00:10:00Z");
        assert_eq!(waiting.expires_at, "2026-06-29T03:10:00Z");
        assert_eq!(
            row(&monitor).last_activity_at.as_deref(),
            Some("2026-06-29T00:10:00Z"),
            "declaring is itself a liveness signal"
        );

        assert_eq!(
            monitor.clear_autonomous_wait(42, "2026-06-29T00:40:00Z"),
            AutonomousWaitOutcome::Cleared
        );
        assert!(row(&monitor).waiting.is_none());
        assert_eq!(
            monitor.clear_autonomous_wait(42, "2026-06-29T00:41:00Z"),
            AutonomousWaitOutcome::NotWaiting
        );
    }

    #[test]
    fn clearing_the_wait_restores_ordinary_stuck_detection() {
        // Issue #3844 AC-5/AC-6: after the agent resumes, recent activity
        // (hook heartbeats from tool calls) keeps it alive exactly as before,
        // and genuine silence is detected again.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.declare_autonomous_wait(
            42,
            "順番待ち",
            "前の agent の完了",
            "2026-06-29T00:10:00Z",
        );
        monitor.clear_autonomous_wait(42, "2026-06-29T00:20:00Z");
        assert!(monitor
            .stuck_autonomous_issues("2026-06-29T00:45:00Z")
            .is_empty());
        monitor.record_autonomous_heartbeat(42, "2026-06-29T00:45:00Z");
        assert!(monitor
            .stuck_autonomous_issues("2026-06-29T01:10:00Z")
            .is_empty());
        assert_eq!(
            monitor.stuck_autonomous_issues("2026-06-29T01:16:00Z"),
            vec![42]
        );
    }

    #[test]
    fn status_view_surfaces_autonomous_mode_and_per_issue_summary() {
        // SPEC #3200 T-048/FR-033: autonomous_mode and per-issue phase / attempts
        // / needs_human are observable in the status view.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_attempt(42);
        monitor.set_autonomous_phase(42, AutonomousPhase::Reviewing);
        monitor.escalate_to_needs_human(43, NeedsHumanKind::UserChoiceRequired, "gate unavailable");

        let view = monitor.status_view();
        assert!(view.autonomous_mode, "autonomous_mode surfaced");
        let summary_42 = view
            .autonomous_issues
            .iter()
            .find(|s| s.issue_number == 42)
            .expect("issue 42 summarized");
        assert_eq!(summary_42.phase, AutonomousPhase::Reviewing);
        assert_eq!(summary_42.attempts, 1);
        assert!(!summary_42.needs_human);
        let summary_43 = view
            .autonomous_issues
            .iter()
            .find(|s| s.issue_number == 43)
            .expect("issue 43 summarized");
        assert!(summary_43.needs_human, "escalated issue marked needs_human");
        assert_eq!(summary_43.phase, AutonomousPhase::NeedsHuman);
    }

    #[test]
    fn two_stage_candidate_requires_mode_and_label() {
        // SPEC #3200 T-032/FR-003/004: the pure pre-gate filter requires BOTH
        // autonomous_mode ON and the auto-merge label. Either missing ⇒ not a
        // candidate (falls back to the human-gated path).
        let labelled = IssueMonitorIssue {
            labels: vec!["auto-merge".to_string()],
            ..issue(42)
        };
        let unlabelled = issue(43);

        let mut off = IssueMonitorState::new(IssueMonitorConfig::default());
        assert!(
            !off.is_autonomous_two_stage_candidate(&labelled),
            "mode off"
        );

        off.set_autonomous_mode(true);
        assert!(
            off.is_autonomous_two_stage_candidate(&labelled),
            "mode on + label ⇒ candidate"
        );
        assert!(
            !off.is_autonomous_two_stage_candidate(&unlabelled),
            "mode on but no label ⇒ not a candidate"
        );
    }

    // Issue #3927 (SPEC #3340 T-630, PM ruling 1f7fdc9e): a successful exact
    // terminal delivery releases the active slot in the same mutation, unbinds
    // the window so the later vanished-window reconciliation has nothing to
    // requeue, and spends no retry attempt. The row stays out of the queue
    // until the ordinary completion probe (merged PR / closed Issue) ends it.
    #[test]
    fn settle_exact_terminal_delivery_releases_slot_without_requeue_or_attempt() {
        let window_id = "tab-1::agent-42";
        let mut monitor = launched_monitor(42, window_id);
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: Some(window_id.to_string()),
        };
        let attempts_before = monitor.autonomous_record(42).map_or(0, |r| r.attempts);

        assert_eq!(monitor.settle_exact_terminal_delivery(&target), Ok(42));

        assert_eq!(monitor.active_count(), 0, "the slot is released");
        assert_eq!(monitor.launched_window_issue(window_id), None);
        assert!(
            monitor
                .vanished_launched_windows("tab-1", &BTreeSet::new())
                .is_empty(),
            "a settled window is not a vanished launch"
        );
        assert_eq!(
            monitor.requeue_exact_window(&target),
            None,
            "the legacy window_closed path is a no-op after settlement"
        );
        assert_eq!(
            monitor.autonomous_record(42).map_or(0, |r| r.attempts),
            attempts_before,
            "settlement consumes no retry attempt"
        );
        let item = monitor.inbox_item(42).expect("row survives settlement");
        assert_eq!(item.state, MonitorInboxState::Launched);
        assert_eq!(item.launched_window_id, None);
        assert_eq!(monitor.queue_len(), 0, "not relaunched");

        // Durable roundtrip keeps the settlement.
        let mut reloaded =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        assert_eq!(reloaded.active_count(), 0);
        assert_eq!(reloaded.launched_window_issue(window_id), None);
        assert!(
            reloaded.settle_exact_terminal_delivery(&target).is_err(),
            "a second settlement of the same launch is refused"
        );
    }

    #[test]
    fn settle_exact_terminal_delivery_refuses_stale_identity() {
        let window_id = "tab-1::agent-42";
        let mut monitor = launched_monitor(42, window_id);
        let live_claim = monitor.live_claim_id(42);
        let stale_claim = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: Some("claim-predecessor".to_string()),
            delivery_id: None,
            window_id: Some(window_id.to_string()),
        };
        assert_eq!(
            monitor.settle_exact_terminal_delivery(&stale_claim),
            Err(IssueMonitorStopMismatch::ClaimMismatch)
        );
        let stale_window = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: live_claim.clone(),
            delivery_id: None,
            window_id: Some("tab-1::agent-99".to_string()),
        };
        assert_eq!(
            monitor.settle_exact_terminal_delivery(&stale_window),
            Err(IssueMonitorStopMismatch::WindowMismatch)
        );
        let no_window = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: live_claim,
            delivery_id: None,
            window_id: None,
        };
        assert_eq!(
            monitor.settle_exact_terminal_delivery(&no_window),
            Err(IssueMonitorStopMismatch::WindowMismatch)
        );
        assert_eq!(
            monitor.active_count(),
            1,
            "a refused settlement mutates nothing"
        );
        assert_eq!(monitor.launched_window_issue(window_id), Some(42));
    }

    #[test]
    fn window_closed_after_settlement_keeps_the_legacy_unsuccessful_contract() {
        // The paired regression: an unsuccessful/manual close still requeues
        // and consumes an attempt; the new exact settlement does not change it.
        let window_id = "tab-1::agent-42";
        let mut monitor = launched_monitor(42, window_id);
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: Some(window_id.to_string()),
        };
        assert_eq!(monitor.requeue_exact_window(&target), Some(42));
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(monitor.autonomous_record(42).map_or(0, |r| r.attempts), 1);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
    }

    #[test]
    fn autoclose_failed_window_only_for_autonomous_candidates() {
        // #3165/#3200 error-window lifecycle: a failed autonomous issue
        // (autonomous_mode ON + auto-merge label) auto-closes its stale window so
        // the retry relaunches clean; default issues keep theirs for inspection.
        let now = "2026-06-30T00:00:00Z";

        let mut auto = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        scan_issue_monitor_candidates(&mut auto, &[auto_issue(42, "b")], now);
        auto.record_agent_issue_failed(42, "boom");
        assert!(
            auto.should_autoclose_failed_window(42),
            "autonomous candidate failure ⇒ auto-close the stale window"
        );
        assert!(
            !auto.should_autoclose_failed_window(999),
            "unknown issue ⇒ no close"
        );

        // autonomous_mode OFF ⇒ keep the window (default human-gated path).
        let mut def = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut def, &[auto_issue(42, "b")], now);
        def.record_agent_issue_failed(42, "boom");
        assert!(
            !def.should_autoclose_failed_window(42),
            "autonomous_mode off ⇒ keep the failed window"
        );

        // autonomous_mode ON but no auto-merge label ⇒ keep the window.
        let mut nolabel = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        scan_issue_monitor_candidates(&mut nolabel, &[issue(43)], now);
        nolabel.record_agent_issue_failed(43, "boom");
        assert!(
            !nolabel.should_autoclose_failed_window(43),
            "no auto-merge label ⇒ keep the failed window"
        );
    }

    #[test]
    fn failed_window_is_retained_persisted_and_cleared_on_relaunch() {
        // #3165 error-window lifecycle: a failed agent window id is retained per
        // issue (and persisted) so an explicit Launch Now can close the stale
        // window before relaunching; a successful relaunch clears it.
        let mut monitor = launched_monitor(42, "tab-1::agent-42");
        assert_eq!(
            monitor.record_agent_window_failed("tab-1::agent-42", "boom"),
            Some(42)
        );

        // Persisted across a prefs round-trip (daemon/GUI restart).
        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        assert_eq!(
            restored.take_failed_window(42).as_deref(),
            Some("tab-1::agent-42"),
            "stale window id retained + persisted for Launch Now"
        );
        // take is one-shot.
        assert_eq!(restored.take_failed_window(42), None);

        // A successful (re)launch clears any retained stale window.
        let mut relaunch = launched_monitor(43, "old::agent-43");
        relaunch.record_agent_window_failed("old::agent-43", "boom");
        relaunch.complete_active_launch(43, "new::agent-43");
        assert_eq!(
            relaunch.take_failed_window(43),
            None,
            "relaunch clears the stale window so it is not double-closed"
        );

        // A pre-launch failure with no window records nothing to close.
        let mut no_window = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut no_window, &[issue(44)], "2026-06-30T00:00:00Z");
        no_window.record_launch_failed(44, "could not create branch");
        assert_eq!(no_window.take_failed_window(44), None);
    }

    #[test]
    fn failed_window_does_not_orphan_on_terminal_transition() {
        // Adversarial-review fix: a failed issue that ends WITHOUT an explicit
        // Launch Now relaunch (merged / released — both funnel through
        // clear_active_tracking) must not orphan its retained failed-window id,
        // which would otherwise persist into prefs unbounded.
        for terminal in ["merged", "released"] {
            let mut monitor = launched_monitor(42, "tab-1::agent-42");
            monitor.record_agent_window_failed("tab-1::agent-42", "boom");
            assert!(
                monitor
                    .prefs()
                    .failed_issues
                    .iter()
                    .any(|f| f.window_id.as_deref() == Some("tab-1::agent-42")),
                "failed window retained before {terminal}"
            );

            match terminal {
                "merged" => monitor.record_merged(42),
                "released" => monitor.record_released(42),
                _ => unreachable!(),
            }

            assert_eq!(
                monitor.take_failed_window(42),
                None,
                "{terminal} must clear the stale failed window (no orphan)"
            );
            assert!(
                monitor
                    .prefs()
                    .failed_issues
                    .iter()
                    .all(|f| f.window_id.is_none()),
                "{terminal} must not persist an orphaned window id"
            );
        }
    }

    #[test]
    fn autonomous_records_survive_prefs_roundtrip() {
        // SPEC #3200 T-016/T-022: attempt counter + phase + launch id + snapshot
        // persist through a prefs round-trip so a daemon restart does not lose an
        // in-flight autonomous attempt (and does not reset attempts to zero).
        use crate::issue_monitor_gate::classify_acceptance_criteria;
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.record_attempt(11);
        monitor.record_attempt(11);
        monitor.set_autonomous_phase(11, AutonomousPhase::Reviewing);
        monitor.set_active_launch_id(11, Some("tab-2::agent-11".to_string()));
        monitor.capture_acceptance_snapshot(
            11,
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n").snapshot(),
        );

        let prefs = monitor.prefs();
        assert_eq!(prefs.autonomous_records.len(), 1);

        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        let record = restored.autonomous_record(11).expect("record restored");
        assert_eq!(record.attempts, 2);
        assert_eq!(record.phase, AutonomousPhase::Reviewing);
        assert_eq!(record.active_launch_id.as_deref(), Some("tab-2::agent-11"));
        assert_eq!(restored.attempt_count(11), 2);
        assert!(record.acceptance_snapshot.is_some());
    }

    fn auto_issue(number: u64, body: &str) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some(body.to_string()),
            url: None,
            readiness: IssueMonitorReadiness::NotApplicable,
            updated_at: None,
        }
    }

    fn autonomous_state() -> IssueMonitorState {
        IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        )
    }

    #[test]
    fn prepare_autonomous_candidate_non_candidate_is_human_gate_noop() {
        // SPEC #3200 FR-001/003: autonomous_mode OFF (or no label) ⇒ no autonomous
        // state created; the issue uses the existing human-gated path.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let decision = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T00:00:00Z",
        );
        assert!(matches!(decision, EligibilityDecision::HumanGate(_)));
        assert!(monitor.autonomous_record(50).is_none());
    }

    #[test]
    fn prepare_autonomous_candidate_eligible_captures_snapshot_and_phase() {
        let mut monitor = autonomous_state();
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let decision = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T00:00:00Z",
        );
        assert_eq!(decision, EligibilityDecision::Eligible);
        let record = monitor.autonomous_record(50).expect("record");
        assert_eq!(record.phase, AutonomousPhase::Implementing);
        assert!(
            record.acceptance_snapshot.is_some(),
            "snapshot captured at launch"
        );
    }

    #[test]
    fn prepare_autonomous_candidate_unverified_protection_is_not_ready() {
        // Issue #3944 AC-4: unverified branch protection is a readiness gap,
        // named on the row and re-checked next scan — not a park.
        let mut monitor = autonomous_state();
        let issue = auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n");
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&issue),
            "2026-06-29T00:00:00Z",
        );
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Absent;
        let decision = monitor.prepare_autonomous_candidate(&issue, &bp, "2026-06-29T00:00:00Z");
        assert!(matches!(decision, EligibilityDecision::NotReady(_)));
        assert_ne!(
            monitor.autonomous_record(50).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman),
            "a readiness gap never parks the candidate",
        );
        assert_eq!(
            monitor.inbox_item(50).map(|item| item.state),
            Some(MonitorInboxState::NotReady)
        );
        assert!(monitor
            .inbox_item(50)
            .and_then(|item| item.exclusion_reason.as_deref())
            .is_some_and(|reason| reason.contains("branch protection")));
        assert!(
            !monitor.queued_issue_numbers().contains(&50),
            "not launchable until the repository is ready"
        );
    }

    #[test]
    fn prepare_autonomous_candidate_without_criteria_is_not_ready() {
        // SPEC #3200 FR-014 as re-scoped by Issue #3944 AC-4: no
        // machine-checkable acceptance criteria ⇒ `not_ready`, naming the gap.
        let mut monitor = autonomous_state();
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let issue = auto_issue(50, "free text, no criteria");
        scan_issue_monitor_candidates(
            &mut monitor,
            std::slice::from_ref(&issue),
            "2026-06-29T00:00:00Z",
        );
        let decision = monitor.prepare_autonomous_candidate(&issue, &bp, "2026-06-29T00:00:00Z");
        let EligibilityDecision::NotReady(reason) = decision else {
            panic!("expected NotReady, got {decision:?}");
        };
        assert_ne!(
            monitor.autonomous_record(50).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman),
        );
        // Issue #3930 AC-2: it names the element that is actually missing
        // (here: no recognized heading at all) instead of a vague "no block".
        // Issue #3944: a `not_ready` row is re-evaluated by the next scan, so
        // fixing the body is the whole remedy — no requeue operation needed.
        assert!(
            reason.contains("no acceptance criteria heading"),
            "reason = {reason}"
        );
        assert!(
            reason.contains("受け入れ基準")
                && reason.contains("受け入れ条件")
                && reason.contains("re-evaluated on the next scan"),
            "reason must carry the fix and the re-evaluation rule, got: {reason}"
        );
        // Issue #3873 AC-7 as re-scoped by Issue #3944 AC-4: the backlog does
        // not list the row under `needs_human`; the inbox row is `not_ready`
        // and carries the same reason, so it is still enumerable in one call.
        let agent_status = monitor.agent_status_at("2026-06-29T00:01:00Z");
        assert!(agent_status.needs_human.is_empty());
        assert!(!monitor
            .status_view()
            .autonomous_issues
            .iter()
            .any(|row| row.issue_number == 50 && row.needs_human));
        let row = monitor.inbox_item(50).expect("row is projected");
        assert_eq!(row.state, MonitorInboxState::NotReady);
        assert_eq!(row.exclusion_reason.as_deref(), Some(reason.as_str()));
    }

    #[test]
    fn prepare_autonomous_candidate_respects_retry_backoff() {
        // SPEC #3200 T-043/FR-029: a candidate whose transient-retry backoff has
        // not elapsed is skipped (no capture/escalation); once it elapses it is
        // processed normally.
        let mut monitor = autonomous_state();
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        // Schedule a backoff: a transient failure sets retry_not_before.
        monitor.record_attempt(50); // ensure a record exists
        monitor.record_autonomous_failure(50, "blip", "2026-06-29T00:00:00Z");
        // Still inside the backoff window ⇒ skipped (HumanGate, not captured).
        let blocked = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T00:00:30Z",
        );
        assert!(matches!(blocked, EligibilityDecision::HumanGate(_)));
        assert_ne!(
            monitor.autonomous_record(50).map(|r| r.phase),
            Some(AutonomousPhase::Implementing),
            "not launched while backing off",
        );
        // After the backoff window ⇒ eligible and prepared.
        let ready = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T02:00:00Z",
        );
        assert_eq!(ready, EligibilityDecision::Eligible);
        assert_eq!(
            monitor
                .autonomous_record(50)
                .and_then(|r| r.retry_not_before.clone()),
            None,
            "launching clears the consumed backoff marker",
        );
    }

    #[test]
    fn recover_stuck_autonomous_is_noop_when_mode_off() {
        // Fail-closed: with autonomous_mode OFF, stuck recovery never mutates
        // state (defends the SPEC #3165 path against the runtime toggle).
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_mode(false);
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        monitor.record_autonomous_heartbeat(42, "2026-06-29T00:00:00Z");
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T05:00:00Z");
        assert!(recovered.is_empty(), "off ⇒ no recovery");
        assert_eq!(monitor.active_count(), 1, "slot untouched when mode off");
        assert_eq!(monitor.attempt_count(42), 0, "no attempt recorded when off");
    }

    #[test]
    fn resume_after_restart_reverts_reviewing_to_implementing_for_redispatch() {
        // review follow-up: a record persisted mid-review reloads in `Reviewing`,
        // but its (non-persisted) review dispatch is gone, so it would wait forever
        // for a verdict. Resetting it to `Implementing` lets the next scan re-detect
        // the PR and re-issue the review — restoring the pre-persist self-healing.
        // The record is round-tripped through prefs to model an actual restart.
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123"); // → Reviewing, verdict pending
        let restored_prefs = monitor.prefs();

        let mut restarted =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), restored_prefs);
        assert_eq!(
            restarted.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::Reviewing),
            "reloads in Reviewing (the strand)"
        );

        let resumed = restarted.resume_inflight_reviews_after_restart("2026-06-29T00:00:00Z");

        assert_eq!(resumed, vec![7], "the stranded Reviewing record is resumed");
        let record = restarted.autonomous_record(7).expect("record retained");
        assert_eq!(
            record.phase,
            AutonomousPhase::Implementing,
            "reset to Implementing so the next scan re-detects the PR + re-dispatches"
        );
        assert_eq!(
            record.review_passed, None,
            "verdict cleared for the re-review"
        );
        // Still counted in-flight (holds its slot); no attempt is spent on a restart.
        assert_eq!(restarted.autonomous_in_flight_issues(), vec![7]);
        assert_eq!(
            restarted.attempt_count(7),
            0,
            "a restart is not a failed attempt"
        );
    }

    #[test]
    fn deferred_restart_resume_only_rewinds_the_startup_review_set() {
        let mut monitor = autonomous_state();
        monitor.begin_review(7, 70, "old-sha");
        monitor.begin_review(8, 80, "stable-sha");
        let startup_reviews = vec![
            monitor.autonomous_record(7).expect("review 7").clone(),
            monitor.autonomous_record(8).expect("review 8").clone(),
        ];
        monitor.begin_review(7, 71, "new-sha");

        let resumed = monitor
            .resume_inflight_reviews_after_restart_for(&startup_reviews, "2026-06-29T00:00:00Z");

        assert_eq!(resumed, vec![8]);
        assert_eq!(
            monitor.autonomous_record(7).map(|record| record.phase),
            Some(AutonomousPhase::Reviewing),
            "a newer review for the same Issue is not the startup-stranded review",
        );
        assert_eq!(
            monitor.autonomous_record(8).map(|record| record.phase),
            Some(AutonomousPhase::Implementing),
        );
    }

    #[test]
    fn resume_after_restart_preserves_durable_review_verdicts() {
        let mut monitor = autonomous_state();
        monitor.begin_review(7, 70, "pending-sha");
        monitor.begin_review(8, 80, "passed-sha");
        monitor.record_review_verdict(8, true);
        monitor.begin_review(9, 90, "failed-sha");
        monitor.record_review_verdict(9, false);

        let resumed = monitor.resume_inflight_reviews_after_restart("2026-06-29T00:00:00Z");

        assert_eq!(resumed, vec![7], "only a verdict-pending review is lost");
        for (issue_number, expected_verdict) in [(8, true), (9, false)] {
            let record = monitor
                .autonomous_record(issue_number)
                .expect("review record retained");
            assert_eq!(record.phase, AutonomousPhase::Reviewing);
            assert_eq!(
                record.review_passed,
                Some(expected_verdict),
                "a durable verdict must survive daemon restart",
            );
        }
    }

    #[test]
    fn resume_after_restart_leaves_delivering_and_other_phases_untouched() {
        // Delivering self-heals on its own (its watch polls the persisted pr_number
        // for the merge commit) and has an armed GitHub auto-merge, so it must NOT
        // be re-driven. Idle/Implementing/terminal phases are also left alone.
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(1, AutonomousPhase::Delivering);
        monitor.set_autonomous_phase(2, AutonomousPhase::Implementing);
        monitor.set_autonomous_phase(3, AutonomousPhase::NeedsHuman);
        monitor.set_autonomous_phase(4, AutonomousPhase::Idle);

        let resumed = monitor.resume_inflight_reviews_after_restart("2026-06-29T00:00:00Z");

        assert!(resumed.is_empty(), "no Reviewing records ⇒ nothing resumed");
        assert_eq!(
            monitor.autonomous_record(1).map(|r| r.phase),
            Some(AutonomousPhase::Delivering),
            "Delivering is left to its own merge-watch self-heal"
        );
        assert_eq!(
            monitor.autonomous_record(2).map(|r| r.phase),
            Some(AutonomousPhase::Implementing)
        );
        assert_eq!(
            monitor.autonomous_record(3).map(|r| r.phase),
            Some(AutonomousPhase::NeedsHuman)
        );
    }

    #[test]
    fn autonomous_transitions_emit_notices_for_the_operator() {
        // SPEC #3200 FR-034 (T-109/T-111, Sc 24): unattended autonomous lifecycle
        // transitions must surface operator notices — merged, needs-human, and
        // transient retry — so fully-unattended operation is observable. The
        // notices queue is drained by the daemon worker into `toast` payloads.
        let mut monitor = autonomous_state();
        scan_issue_monitor_candidates(
            &mut monitor,
            &[spec_issue(
                7,
                IssueMonitorReadiness::ReadyWithCompletedTasks,
                "2026-07-02T00:00:00Z",
            )],
            "2026-07-02T00:00:00Z",
        );
        monitor.complete_active_launch(7, "tab-1::agent-7");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);

        // Transient retry → warn notice naming the attempt.
        monitor.record_autonomous_failure(7, "review spawn blip", "2026-07-02T00:10:00Z");
        // Gate pass → Delivering; the info notice fires only once the arm
        // actually SUCCEEDS (codex #3217: no success toast for a failed arm).
        monitor.set_autonomous_phase(7, AutonomousPhase::Reviewing);
        monitor.begin_delivering(7);
        monitor.record_auto_merge_armed(7);
        // Merge completion → done notice.
        monitor.record_merged(7);
        // A second issue escalates → error notice.
        scan_issue_monitor_candidates(&mut monitor, &[issue(8)], "2026-07-02T00:20:00Z");
        monitor.record_attempt(8);
        monitor.escalate_to_needs_human(8, NeedsHumanKind::UserChoiceRequired, "review rejected");

        let notices = monitor.take_autonomous_notices();
        let summary: Vec<(String, u64)> = notices
            .iter()
            .map(|notice| (notice.level.clone(), notice.issue_number))
            .collect();
        assert!(
            summary.contains(&("warn".to_string(), 7)),
            "transient retry emits a warn notice: {summary:?}"
        );
        assert!(
            summary.contains(&("info".to_string(), 7)),
            "auto-merge arming emits an info notice: {summary:?}"
        );
        assert!(
            summary.contains(&("done".to_string(), 7)),
            "merge completion emits a done notice: {summary:?}"
        );
        assert!(
            summary.contains(&("error".to_string(), 8)),
            "needs-human escalation emits an error notice: {summary:?}"
        );
        let retry = notices
            .iter()
            .find(|notice| notice.level == "warn")
            .expect("retry notice");
        assert!(
            retry.message.contains("review spawn blip"),
            "retry notice carries the failure reason: {}",
            retry.message
        );
        // Drained: a second take returns nothing.
        assert!(monitor.take_autonomous_notices().is_empty());
    }

    #[test]
    fn default_off_transitions_emit_no_autonomous_notices() {
        // FR-004 non-regression: with autonomous_mode OFF (default), the human-
        // gated #3165 flow emits no autonomous notices — merges and failures are
        // already visible via inbox state without extra toasts.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_merged(42);
        assert!(
            monitor.take_autonomous_notices().is_empty(),
            "default-OFF merge emits no autonomous notice"
        );
    }

    #[test]
    fn kill_switch_retries_failed_disarms_until_success() {
        // codex #3217/#3219 review: turning autonomous mode OFF must actively
        // disarm GitHub auto-merges armed by the monitor — and a FAILED disarm
        // must stay retryable. A record leaves Delivering only after the disarm
        // succeeds; otherwise the armed auto-merge would stay live on GitHub
        // behind a NeedsHuman screen with nothing retrying it.
        let mut monitor = autonomous_state();
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-07-02T00:00:00Z");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123");
        monitor.begin_delivering(7);
        monitor.set_enabled(true);

        // Mode ON ⇒ no targets (deliveries are still owned by the loop).
        assert!(monitor.kill_switch_disarm_targets().is_empty());

        monitor.set_enabled(false);
        assert_eq!(
            monitor.kill_switch_disarm_targets(),
            vec![(7, 99)],
            "global monitor OFF is also a kill switch for an armed delivery"
        );
        monitor.set_enabled(true);

        monitor.set_autonomous_mode(false);
        assert_eq!(
            monitor.kill_switch_disarm_targets(),
            vec![(7, 99)],
            "delivering PR targeted for disarm"
        );

        // FAILED disarm: error notice, record STAYS Delivering ⇒ re-targeted.
        monitor.record_kill_switch_disarm_result(7, 99, false);
        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::Delivering),
            "failed disarm keeps the record in Delivering for retry"
        );
        assert_eq!(
            monitor.kill_switch_disarm_targets(),
            vec![(7, 99)],
            "next scan retries the disarm"
        );

        // SUCCESSFUL disarm: escalates to NeedsHuman (visible, never silently
        // resumed) and stops being targeted.
        monitor.record_kill_switch_disarm_result(7, 99, true);
        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::NeedsHuman)
        );
        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
        assert!(monitor.kill_switch_disarm_targets().is_empty());

        // Both outcomes surfaced even though the mode is OFF (ungated notices).
        let notices = monitor.take_autonomous_notices();
        assert!(notices
            .iter()
            .any(|n| n.level == "error" && n.message.contains("retry next scan")));
        assert!(notices
            .iter()
            .any(|n| n.level == "warn" && n.message.contains("disarmed")));
    }

    #[test]
    fn autonomous_notices_queue_is_bounded() {
        // Unattended operation with a disconnected GUI must not grow the queue
        // without limit: oldest notices are dropped past the cap.
        let mut monitor = autonomous_state();
        for n in 0..200u64 {
            monitor.record_attempt(n);
            monitor.escalate_to_needs_human(n, NeedsHumanKind::UserChoiceRequired, "boom");
        }
        let notices = monitor.take_autonomous_notices();
        assert!(
            notices.len() <= 100,
            "queue bounded to 100, got {}",
            notices.len()
        );
        assert_eq!(
            notices.last().map(|notice| notice.issue_number),
            Some(199),
            "newest notice retained when the cap drops oldest"
        );
    }

    #[test]
    fn resume_after_restart_refreshes_heartbeat_so_stuck_recovery_does_not_fail_it() {
        // review follow-up (codex #3210): on restart the persisted active slot and
        // a STALE last_heartbeat are restored. Resetting Reviewing → Implementing
        // makes the record eligible for stuck/idle detection, which runs BEFORE the
        // re-dispatch on the next scan. Without refreshing the heartbeat, a review
        // that ran longer than stuck_timeout_secs before the restart would be
        // wrongly counted as a failed attempt. The refresh must prevent that.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_mode(true);
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        monitor.begin_review(42, 99, "abc123"); // → Reviewing (holds the active slot)
        monitor.record_autonomous_heartbeat(42, "2026-06-29T00:00:00Z"); // stale pre-restart

        // Resume, then run stuck recovery in the same scan order as the daemon.
        let resumed = monitor.resume_inflight_reviews_after_restart("2026-06-29T05:00:00Z");
        assert_eq!(resumed, vec![42]);
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T05:00:00Z");

        assert!(
            recovered.is_empty(),
            "the resumed record's fresh heartbeat keeps it out of stuck recovery"
        );
        assert_eq!(
            monitor.autonomous_record(42).map(|r| r.phase),
            Some(AutonomousPhase::Implementing),
            "still Implementing, ready for the next scan to re-dispatch review"
        );
        assert_eq!(
            monitor.attempt_count(42),
            0,
            "a restart is not counted as a failed attempt"
        );
    }

    #[test]
    fn autonomous_loop_transitions_track_pr_sha_and_verdict() {
        // SPEC #3200: Implementing → Reviewing (bind PR + reviewed SHA) →
        // verdict recorded → Delivering. autonomous_in_flight_issues tracks it.
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        assert_eq!(monitor.autonomous_in_flight_issues(), vec![7]);

        monitor.begin_review(7, 99, "abc123");
        let record = monitor.autonomous_record(7).expect("record");
        assert_eq!(record.phase, AutonomousPhase::Reviewing);
        assert_eq!(record.pr_number, Some(99));
        assert_eq!(record.reviewed_sha.as_deref(), Some("abc123"));
        assert_eq!(record.review_passed, None, "verdict pending");

        monitor.record_review_verdict(7, true);
        assert_eq!(
            monitor.autonomous_record(7).unwrap().review_passed,
            Some(true)
        );

        monitor.begin_delivering(7);
        assert_eq!(
            monitor.autonomous_record(7).unwrap().phase,
            AutonomousPhase::Delivering
        );
        assert_eq!(monitor.autonomous_in_flight_issues(), vec![7]);

        // Completion clears the whole record.
        monitor.record_merged(7);
        assert!(monitor.autonomous_record(7).is_none());
        assert!(monitor.autonomous_in_flight_issues().is_empty());
    }

    #[test]
    fn launch_failure_routes_inflight_autonomous_issue_through_retry() {
        // SPEC #3200 (review follow-up): a launch/agent failure for an in-flight
        // autonomous issue — e.g. the independent review agent could not spawn —
        // must funnel through the autonomous retry machinery (count an attempt,
        // schedule backoff, re-queue) instead of stranding the record in
        // `Reviewing` forever, waiting for a verdict that will never arrive.
        let mut monitor = autonomous_state();
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-06-30T00:00:00Z");
        monitor.complete_active_launch(7, "tab-1::agent-7");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123"); // Implementing → Reviewing
        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::Reviewing)
        );
        assert!(monitor.is_autonomous_in_flight(7));

        monitor.record_launch_failed(7, "Independent review could not start");

        let record = monitor.autonomous_record(7).expect("record retained");
        assert_eq!(
            record.phase,
            AutonomousPhase::Idle,
            "routed back to Idle for retry, not stranded in Reviewing"
        );
        assert_eq!(monitor.attempt_count(7), 1, "the failed attempt is counted");
        assert!(
            record.retry_not_before.is_some(),
            "a retry backoff is scheduled"
        );
        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "re-queued for automatic relaunch (not parked in LaunchFailed)"
        );

        let mut pre_materialization = monitor.clone();
        pre_materialization.record_launch_failed(7, "fresh pre-materialization failure");
        assert_eq!(
            pre_materialization.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::LaunchFailed),
            "a distinct admission before materialization cannot be mistaken for receipt replay"
        );
        assert_manual_relaunch_accepts_fresh_lifecycle_failures(&monitor);
    }

    #[test]
    fn launch_failure_at_cap_requeues_inflight_autonomous_issue_with_steering() {
        // SPEC #3200 (review follow-up) as re-scoped by Issue #3944 AC-3: a
        // launch failure for an in-flight autonomous issue still funnels through
        // the autonomous routing (never strands in `Reviewing`), and at the cap
        // it requeues with a steering request instead of parking the Issue.
        let mut monitor = autonomous_state();
        monitor.autonomous_tuning.max_attempts = 1;
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-06-30T00:00:00Z");
        monitor.complete_active_launch(7, "tab-1::agent-7");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123");

        monitor.record_launch_failed(7, "review spawn failed at cap");

        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::Idle),
            "attempts exhausted ⇒ retried through the ladder, not parked"
        );
        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert!(
            monitor
                .autonomous_record(7)
                .and_then(|r| r.steering.as_ref())
                .is_some_and(|steering| steering.reason.contains("review spawn failed at cap")),
            "the PM is asked to look at the failing launch"
        );
    }

    #[test]
    fn launch_failure_for_non_autonomous_issue_keeps_plain_failed_state() {
        // Non-regression: with no in-flight autonomous record, the launch failure
        // stays on the human-gated LaunchFailed path (SPEC #3165), untouched by
        // the autonomous routing.
        let mut monitor = autonomous_state(); // mode on, but no record for #7
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-06-30T00:00:00Z");
        assert!(!monitor.is_autonomous_in_flight(7));

        monitor.record_launch_failed(7, "binary missing");

        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::LaunchFailed),
            "plain launch-failed path is preserved when no autonomous attempt is in flight"
        );
        assert_eq!(
            monitor.attempt_count(7),
            0,
            "no autonomous attempt is counted"
        );
    }

    #[test]
    fn autonomous_gate_inputs_assemble_into_a_pass_and_detect_drift() {
        // SPEC #3200 FR-009..FR-016: assembled inputs run through the real gate.
        use crate::issue_monitor_gate::{
            classify_acceptance_criteria, evaluate_autonomous_gate, GateDecision,
        };
        use gwt_git::branch_protection::BranchProtectionStatus;
        let body = "## Acceptance Criteria\n- [ ] AC-1: x\n";
        let mut monitor = autonomous_state();
        monitor.capture_acceptance_snapshot(7, classify_acceptance_criteria(body).snapshot());
        monitor.begin_review(7, 99, "abc123");
        monitor.record_review_verdict(7, true);

        let bp = BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let rollup = r#"[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]"#;

        // All conditions hold at the reviewed SHA ⇒ gate Pass.
        let inputs = monitor
            .autonomous_gate_inputs(7, bp.clone(), rollup, "abc123", body)
            .expect("gate ready");
        assert_eq!(evaluate_autonomous_gate(&inputs), GateDecision::Pass);

        // Issue body edited after launch ⇒ acceptance drift ⇒ gate Fail.
        let drifted = monitor
            .autonomous_gate_inputs(
                7,
                bp.clone(),
                rollup,
                "abc123",
                "## Acceptance Criteria\n- [ ] AC-2: new\n",
            )
            .expect("gate ready");
        assert!(matches!(
            evaluate_autonomous_gate(&drifted),
            GateDecision::Fail(_)
        ));

        // HEAD advanced past reviewed SHA ⇒ TOCTOU ⇒ gate Fail.
        let advanced = monitor
            .autonomous_gate_inputs(7, bp, rollup, "def456", body)
            .expect("gate ready");
        assert!(matches!(
            evaluate_autonomous_gate(&advanced),
            GateDecision::Fail(_)
        ));
    }

    #[test]
    fn autonomous_gate_inputs_none_until_review_returns() {
        let mut monitor = autonomous_state();
        monitor.begin_review(7, 99, "abc123"); // verdict pending
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        assert!(
            monitor
                .autonomous_gate_inputs(7, bp, "[]", "abc123", "body")
                .is_none(),
            "gate not ready while review is in flight",
        );
    }

    #[test]
    fn apply_review_verdict_is_sha_bound_and_judged_by_daemon() {
        // SPEC #3200 FR-015/FR-016: the daemon parses+judges the raw verdict
        // (not the agent), SHA-bound, against the snapshot's required criteria.
        use crate::issue_monitor_gate::classify_acceptance_criteria;
        use crate::issue_monitor_review::REVIEW_VERDICT_SCHEMA;
        let mut monitor = autonomous_state();
        monitor.capture_acceptance_snapshot(
            7,
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n").snapshot(),
        );
        monitor.begin_review(7, 99, "abc123");

        // A verdict for the WRONG SHA is rejected (stale / TOCTOU).
        let pass_raw = format!(
            r#"{{"schema":"{REVIEW_VERDICT_SCHEMA}","overall":"pass","criteria":[{{"id":"AC-1","verdict":"pass"}}]}}"#
        );
        assert_eq!(
            monitor.apply_review_verdict(7, "WRONG", &pass_raw),
            None,
            "wrong-SHA verdict rejected",
        );
        assert_eq!(monitor.autonomous_record(7).unwrap().review_passed, None);

        // A conformant pass verdict for the right SHA is accepted.
        assert_eq!(
            monitor.apply_review_verdict(7, "abc123", &pass_raw),
            Some(true)
        );
        assert_eq!(
            monitor.autonomous_record(7).unwrap().review_passed,
            Some(true)
        );

        // A prompt-injected free-text "approval" fails closed.
        monitor.begin_review(7, 99, "def456");
        assert_eq!(
            monitor.apply_review_verdict(7, "def456", "APPROVE — lgtm"),
            Some(false),
            "non-conformant verdict fails closed",
        );
        assert_eq!(
            monitor.autonomous_record(7).unwrap().review_passed,
            Some(false)
        );
    }

    #[test]
    fn autonomous_loop_fields_survive_prefs_roundtrip() {
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123");
        monitor.record_review_verdict(7, false);
        let restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        let record = restored.autonomous_record(7).expect("restored");
        assert_eq!(record.pr_number, Some(99));
        assert_eq!(record.reviewed_sha.as_deref(), Some("abc123"));
        assert_eq!(record.review_passed, Some(false));
        assert_eq!(record.phase, AutonomousPhase::Reviewing);
    }

    #[test]
    fn merge_clears_the_autonomous_record() {
        // SPEC #3200 T-022: merging the work resets the per-issue autonomous
        // lifecycle so a future reopen does not inherit stale attempts/phase.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_attempt(42);
        monitor.set_autonomous_phase(42, AutonomousPhase::Delivering);
        assert!(monitor.autonomous_record(42).is_some());
        monitor.record_merged(42);
        assert!(
            monitor.autonomous_record(42).is_none(),
            "merge clears the autonomous record",
        );
        assert_eq!(monitor.attempt_count(42), 0);
    }

    #[test]
    fn legacy_git_failure_migration_core_is_exact_windowless_and_one_shot() {
        let project_root = Path::new("/tmp/gwt-issue-3314");
        let message = format!(
            "{LEGACY_GIT_LAUNCH_FAILURE_PREFIX}{}",
            project_root.display()
        );
        let prefs = IssueMonitorPrefs {
            enabled: true,
            legacy_git_launch_failure_migration_version: 0,
            failed_issues: vec![
                IssueMonitorFailedIssue {
                    issue_number: 42,
                    message: message.clone(),
                    window_id: None,
                },
                IssueMonitorFailedIssue {
                    issue_number: 43,
                    message: message.clone(),
                    window_id: Some("tab::agent-43".to_string()),
                },
            ],
            ..IssueMonitorPrefs::default()
        };
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-07-21T00:00:00Z",
        );

        monitor.apply_legacy_git_launch_failure_migration(project_root);

        assert_eq!(
            monitor.legacy_git_launch_failure_migration_version,
            LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        );
        assert!(!monitor.failed_issues.contains_key(&42));
        assert!(monitor.failed_issues.contains_key(&43));
        assert!(monitor.inbox_item(42).is_none());
        assert_eq!(
            monitor.inbox_item(43).map(|item| item.state),
            Some(MonitorInboxState::AgentFailed)
        );

        monitor.record_launch_failed(42, message);
        monitor.apply_legacy_git_launch_failure_migration(project_root);
        assert!(
            monitor.failed_issues.contains_key(&42),
            "an equal marker cannot erase a newly recorded same-text failure"
        );
    }

    #[test]
    fn newer_migration_marker_preserves_inbox_only_needs_human_failure() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                legacy_git_launch_failure_migration_version: 0,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(issue(42));
        monitor.record_agent_issue_failed(42, "manual intervention required");
        monitor
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == 42)
            .expect("failed inbox item")
            .state = MonitorInboxState::NeedsHuman;
        assert!(
            monitor.autonomous_record(42).is_none(),
            "the regression requires an inbox-only terminal state"
        );

        assert!(
            monitor.adopt_newer_legacy_git_launch_failure_migration_from_prefs(
                &IssueMonitorPrefs::default()
            )
        );

        assert!(monitor.failed_issues.contains_key(&42));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman),
            "adopting a newer marker must not downgrade a terminal inbox row"
        );
    }

    #[test]
    fn failover_restart_at_authority_epoch_max_is_an_atomic_noop() {
        let mut monitor = launched_monitor(42, "tab-1::agent-max");
        monitor.effect_authority_epoch = u64::MAX;
        let target = IssueMonitorStopTarget {
            issue_number: 42,
            claim_id: monitor.live_claim_id(42),
            delivery_id: monitor.pending_launch_delivery_id(42),
            window_id: monitor.launched_window_id(42),
        };
        let before = monitor.clone();

        let outcome = monitor.failover_restart(&target, "switch provider", "2026-08-13T00:00:00Z");

        assert!(
            !matches!(outcome, IssueMonitorFailoverOutcome::Restarting { .. }),
            "an exact source cannot report successful failover when its authority epoch cannot advance"
        );
        assert_eq!(
            monitor, before,
            "authority exhaustion must not clear the live source, requeue, mark FreshRequired, emit a notice, or mutate any transient/durable field"
        );
    }

    fn quota_hold_evidence_fixture() -> IssueMonitorProviderQuotaHoldEvidence {
        IssueMonitorProviderQuotaHoldEvidence {
            recorded_at: "2026-09-02T09:01:00Z".to_string(),
            source: "screen_notice".to_string(),
            issue_number: None,
            window_id: Some("tab-1::agent-42".to_string()),
            screen_text: Some(
                "You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage \
                 to purchase more credits or try again at Sep 7th, 2026 12:58 PM."
                    .to_string(),
            ),
            poller_state: Some("ok".to_string()),
            poller_limit_reached: Some(false),
            poller_windows: vec![
                IssueMonitorProviderQuotaPollerWindow {
                    kind: "five_hour".to_string(),
                    used_percent: 26,
                },
                IssueMonitorProviderQuotaPollerWindow {
                    kind: "weekly".to_string(),
                    used_percent: 26,
                },
            ],
        }
    }

    /// Issue #3923 AC-1 / AC-2 / AC-4: a hold is listed with the evidence it
    /// was formed from, and an explicit operator release removes the
    /// provider-wide hold and readmits every issue it was holding.
    #[test]
    fn clearing_a_provider_quota_hold_readmits_the_issues_it_held() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(test_launch_profile("codex")),
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.complete_active_launch(42, "tab-1::agent-42");
        let evidence = quota_hold_evidence_fixture();

        assert_eq!(
            monitor.try_hold_provider_usage_limit(
                42,
                "tab-1::agent-42",
                "codex",
                "Codex usage limit reached",
                Some("2026-09-07T03:58:00Z"),
                Some(evidence.clone()),
                "2026-09-02T09:01:00Z",
            ),
            IssueMonitorProviderUsageLimitOutcome::Held
        );

        let status = monitor.agent_status_at("2026-09-02T09:02:00Z");
        assert_eq!(
            status.provider_quota_holds.len(),
            1,
            "AC-1: every provider hold is listed"
        );
        let hold = &status.provider_quota_holds[0];
        assert_eq!(hold.provider, "codex");
        assert_eq!(hold.reset_at, "2026-09-07T03:58:00Z");
        let recorded = hold
            .evidence
            .as_ref()
            .expect("AC-2: the hold carries the evidence it was formed from");
        assert_eq!(recorded.issue_number, Some(42));
        assert_eq!(recorded.recorded_at, "2026-09-02T09:01:00Z");
        assert_eq!(recorded.screen_text, evidence.screen_text);
        assert_eq!(recorded.poller_windows, evidence.poller_windows);
        assert_eq!(
            monitor.prefs().provider_quota_hold_evidence.get("codex"),
            Some(recorded),
            "the evidence is durable alongside the hold"
        );
        assert!(!monitor.retry_ready_for_saved_profile(42, "2026-09-02T09:02:00Z"));

        assert_eq!(
            monitor.clear_provider_quota_hold(
                "codex",
                "PM: Codex is not rate limited",
                "2026-09-02T09:11:00Z",
            ),
            IssueMonitorProviderQuotaHoldClearOutcome::Cleared {
                released_reset_at: Some("2026-09-07T03:58:00Z".to_string()),
                released_issues: vec![42],
            }
        );

        let status = monitor.agent_status_at("2026-09-02T09:12:00Z");
        assert!(status.provider_quota_holds.is_empty());
        assert_eq!(status.quota_hold, None);
        assert!(
            monitor.retry_ready_for_saved_profile(42, "2026-09-02T09:12:00Z"),
            "the issues the hold was holding are admitted again"
        );
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.clone()),
            None
        );
        let prefs = monitor.prefs();
        assert!(prefs.provider_quota_holds.is_empty());
        assert!(prefs.provider_quota_hold_evidence.is_empty());
        let release = prefs
            .provider_quota_hold_releases
            .get("codex")
            .expect("the release is a durable fact, not the absence of one");
        assert_eq!(release.released_at, "2026-09-02T09:11:00Z");
        assert_eq!(release.reason, "PM: Codex is not rate limited");
        assert_eq!(
            release.released_reset_at.as_deref(),
            Some("2026-09-07T03:58:00Z")
        );

        assert_eq!(
            monitor.clear_provider_quota_hold("codex", "again", "2026-09-02T09:13:00Z"),
            IssueMonitorProviderQuotaHoldClearOutcome::Cleared {
                released_reset_at: None,
                released_issues: Vec::new(),
            },
            "clearing an unheld provider is idempotent and still fences in-memory holds elsewhere"
        );
        assert_eq!(
            monitor.clear_provider_quota_hold("   ", "x", "2026-09-02T09:13:00Z"),
            IssueMonitorProviderQuotaHoldClearOutcome::UnknownProvider,
            "a custom agent id is a valid provider, so only a blank name is unknown"
        );
    }

    /// Issue #3923 AC-1: removing the hold from disk says nothing to a process
    /// that still holds it in memory (holds join by the later reset), so the
    /// release must be a published fence — and it must not erase a hold that
    /// was formed after it.
    #[test]
    fn a_provider_quota_hold_release_fences_the_stale_hold_but_not_a_newer_one() {
        let mut daemon = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(test_launch_profile("codex")),
                ..IssueMonitorPrefs::default()
            },
        );
        daemon.set_gui_connected(true);
        daemon.record_candidate(issue(42));
        daemon.complete_active_launch(42, "tab-1::agent-42");
        assert_eq!(
            daemon.try_hold_provider_usage_limit(
                42,
                "tab-1::agent-42",
                "codex",
                "Codex usage limit reached",
                Some("2026-09-07T03:58:00Z"),
                None,
                "2026-09-02T09:01:00Z",
            ),
            IssueMonitorProviderUsageLimitOutcome::Held
        );

        let mut operator =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), daemon.prefs());
        assert!(matches!(
            operator.clear_provider_quota_hold("codex", "PM ruling", "2026-09-02T09:11:00Z"),
            IssueMonitorProviderQuotaHoldClearOutcome::Cleared { .. }
        ));
        let disk = operator.prefs();
        assert!(disk.provider_quota_holds.is_empty());

        daemon.rebase_daemon_driver_prefs(&disk);
        assert!(
            daemon.prefs().provider_quota_holds.is_empty(),
            "a hold released on disk must not be re-stamped by a process that still holds it"
        );
        assert!(daemon.retry_ready_for_saved_profile(42, "2026-09-02T09:12:00Z"));

        daemon.complete_active_launch(42, "tab-1::agent-43");
        assert_eq!(
            daemon.try_hold_provider_usage_limit(
                42,
                "tab-1::agent-43",
                "codex",
                "Codex usage limit reached",
                Some("2026-09-07T03:58:00Z"),
                None,
                "2026-09-02T09:30:00Z",
            ),
            IssueMonitorProviderUsageLimitOutcome::Held
        );
        daemon.rebase_daemon_driver_prefs(&disk);
        assert_eq!(
            daemon
                .prefs()
                .provider_quota_holds
                .get("codex")
                .map(String::as_str),
            Some("2026-09-07T03:58:00Z"),
            "a hold recorded after the release is new evidence and survives the same fence"
        );

        let restored = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                provider_quota_holds: BTreeMap::from([(
                    "codex".to_string(),
                    "2026-09-07T03:58:00Z".to_string(),
                )]),
                provider_quota_hold_evidence: BTreeMap::from([(
                    "codex".to_string(),
                    quota_hold_evidence_fixture(),
                )]),
                provider_quota_hold_releases: BTreeMap::from([(
                    "codex".to_string(),
                    IssueMonitorProviderQuotaHoldRelease {
                        released_at: "2026-09-02T09:11:00Z".to_string(),
                        reason: "PM ruling".to_string(),
                        released_reset_at: Some("2026-09-07T03:58:00Z".to_string()),
                    },
                )]),
                ..IssueMonitorPrefs::default()
            },
        );
        assert!(
            restored.prefs().provider_quota_holds.is_empty(),
            "restore applies the same fence as a rebase"
        );
    }

    /// Issue #3923 AC-5: the PM can move the fleet off a held provider from
    /// the CLI. Provider-specific model choices reset; the wizard's runtime
    /// choices survive; a profile that was never saved cannot be invented.
    #[test]
    fn switching_the_launch_profile_agent_keeps_runtime_choices_and_lifts_the_hold() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(IssueMonitorLaunchProfile {
                    model: Some("gpt-5.5".to_string()),
                    reasoning: Some("high".to_string()),
                    version: Some("0.153.2".to_string()),
                    skip_permissions: true,
                    runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                    docker_service: Some("dev".to_string()),
                    ..test_launch_profile("codex")
                }),
                provider_quota_holds: BTreeMap::from([(
                    "codex".to_string(),
                    "2026-09-07T03:58:00Z".to_string(),
                )]),
                ..IssueMonitorPrefs::default()
            },
        );
        assert!(monitor
            .provider_quota_hold_at("2026-09-02T09:00:00Z")
            .is_some());

        let switched = monitor
            .switch_launch_profile_agent("Claude")
            .expect("switch to a healthy provider");
        assert_eq!(switched.agent_id, "claude");
        assert_eq!(
            switched.model, None,
            "a Codex model does not apply to Claude"
        );
        assert_eq!(switched.reasoning, None);
        assert_eq!(switched.version, None);
        assert!(switched.skip_permissions);
        assert_eq!(
            switched.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(switched.docker_service.as_deref(), Some("dev"));
        assert_eq!(monitor.prefs().launch_profile, Some(switched.clone()));
        assert_eq!(
            monitor.provider_quota_hold_at("2026-09-02T09:00:00Z"),
            None,
            "the queue is gated by the saved profile's provider, which is no longer held"
        );

        let again = monitor
            .switch_launch_profile_agent("claude")
            .expect("idempotent");
        assert_eq!(again, switched);
        assert_eq!(
            monitor.switch_launch_profile_agent("  "),
            Err(IssueMonitorLaunchProfileSwitchError::InvalidAgent)
        );

        let mut unsaved = IssueMonitorState::new(IssueMonitorConfig::default());
        assert_eq!(
            unsaved.switch_launch_profile_agent("claude"),
            Err(IssueMonitorLaunchProfileSwitchError::NoSavedProfile),
            "runtime and Docker choices cannot be invented from an agent name"
        );
        assert!(!unsaved.has_launch_profile());
    }

    /// Issue #3923 AC-3: the poller reading that says "this account is fine"
    /// is scoped to the agent's own provider and requires fresh data.
    #[test]
    fn a_healthy_poller_reading_is_recognized_only_for_a_fresh_ok_account() {
        use gwt_core::usage::{ProviderUsage, UsageProvider, UsageState, UsageWindow, WindowKind};

        let healthy = ProviderUsage {
            provider: UsageProvider::Codex,
            account_label: None,
            plan: None,
            windows: vec![
                UsageWindow::new(WindowKind::FiveHour, 26.0, None),
                UsageWindow::new(WindowKind::Weekly, 26.0, None),
            ],
            limit_reached: false,
            state: UsageState::Ok,
            fetched_at: None,
        };
        assert!(provider_reports_healthy_for_agent(
            "codex",
            &[healthy.clone()]
        ));
        assert!(
            !provider_reports_healthy_for_agent("claude", &[healthy.clone()]),
            "a Codex reading says nothing about Claude"
        );
        let stale = ProviderUsage {
            state: UsageState::Stale { age_secs: 900 },
            ..healthy.clone()
        };
        assert!(!provider_reports_healthy_for_agent("codex", &[stale]));
        let full = ProviderUsage {
            windows: vec![UsageWindow::new(WindowKind::Weekly, 100.0, None)],
            ..healthy.clone()
        };
        assert!(!provider_reports_healthy_for_agent("codex", &[full]));
        let reached = ProviderUsage {
            limit_reached: true,
            ..healthy
        };
        assert!(!provider_reports_healthy_for_agent("codex", &[reached]));
    }

    #[test]
    fn transient_launch_failure_requeues_without_consuming_an_attempt() {
        // Issue #3941 AC-3: a launch aborted by transient infrastructure (probe
        // timeout with no cached version, concurrent fetch ref race) is not an
        // agent failure. It goes back to the queue behind a backoff, keeps its
        // reason visible, and spends no attempt — no PM requeue needed.
        let now = "2026-09-04T00:39:39Z";
        let message = "exact npx package probe timed out for @openai/codex@0.153.2 after 120 seconds and the requested version is not in the local package cache; launch was aborted before spawn. Next: warm the cache; gwt retries this launch automatically without spending an attempt.";
        for autonomous in [true, false] {
            let mut monitor = if autonomous {
                autonomous_state()
            } else {
                IssueMonitorState::new(IssueMonitorConfig::default())
            };
            scan_issue_monitor_candidates(
                &mut monitor,
                &[auto_issue(3928, "## Acceptance Criteria\n- [ ] AC-1: x\n")],
                now,
            );
            monitor.complete_active_launch(3928, "agent-93");
            assert_eq!(monitor.active_count(), 1, "autonomous={autonomous}");

            monitor.record_agent_issue_failed(3928, message);

            assert_eq!(monitor.attempt_count(3928), 0, "autonomous={autonomous}");
            assert_eq!(monitor.active_count(), 0, "autonomous={autonomous}");
            assert!(monitor.queue.contains(&3928), "autonomous={autonomous}");
            assert!(
                !monitor.failed_issues.contains_key(&3928),
                "autonomous={autonomous}: transient failures are not parked"
            );
            let item = monitor.inbox_item(3928).expect("inbox item");
            assert_eq!(
                item.state,
                MonitorInboxState::Queued,
                "autonomous={autonomous}"
            );
            assert_eq!(item.error_message.as_deref(), Some(message));
            assert!(item.launched_window_id.is_none());
            // The funnel stamps the backoff from the wall clock.
            let wall_now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            assert!(
                !monitor.retry_ready(3928, &wall_now),
                "autonomous={autonomous}: a backoff is scheduled"
            );
            assert!(monitor.retry_ready(3928, "2099-01-01T00:00:00Z"));
        }

        // A non-transient failure keeps the human-gated terminal path.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[auto_issue(3928, "b")], now);
        monitor.complete_active_launch(3928, "agent-93");
        monitor.record_agent_issue_failed(3928, "boom");
        assert_eq!(
            monitor.inbox_item(3928).map(|item| item.state),
            Some(MonitorInboxState::AgentFailed)
        );
    }
}
