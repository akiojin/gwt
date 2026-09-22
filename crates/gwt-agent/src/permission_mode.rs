//! Cross-provider Permission Mode Decision and the forced-skip launch
//! contract (Issue #4543).
//!
//! Before this module the answer to "does this launch run without permission
//! prompts?" was a bare `bool` that every entry point computed for itself, and
//! the per-provider flag that carries it was open-coded eight times inside
//! [`crate::launch`]. Two failure modes followed from that shape:
//!
//! * a producing-work launch inherited `skip_permissions = false` from a saved
//!   profile / last settings / a resumed session and then stalled on a prompt
//!   nobody was watching, and
//! * a provider with no skip mapping at all (OpenClaw) or a Custom Coding Agent
//!   with an empty `skip_permissions_args` accepted the request silently and
//!   dropped the flag on the floor.
//!
//! The decision model here is the single place that answers the question. It is
//! pure: it takes what the launch already knows and returns a typed outcome
//! with a reason, a recovery action, and the exact provider mapping evidence it
//! used, so the caller can record *why* a launch is permissioned the way it is.
//!
//! Enforcement is deliberately not here. Issue #4543 wires the validator into
//! every launch entry point and records its verdict; refusing an
//! `unsupported_provider` / `missing_custom_skip_mapping` launch is a dependent
//! follow-up. Until then an unsupported launch still starts — it just stops
//! being invisible.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::custom::CustomCodingAgent;
use crate::types::{AgentId, LaunchRoute};

/// Which launch surface supplied the permission preference.
///
/// Issue #4543 AC-3 names the surfaces whose stored preference used to decide
/// a producing-work launch. Tagging the source keeps the recorded decision
/// answerable ("which stored setting did the forced skip override?") without
/// the decision model needing to know anything else about the caller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLaunchSource {
    /// A human pressed Start Work in the GUI.
    StartWork,
    /// A saved Issue Monitor launch profile.
    IssueMonitorProfile,
    /// The Launch Wizard's last-used settings for this agent.
    LaunchWizardLastSettings,
    /// A Resume / Continue launch, or an `execution.adopt` takeover.
    ResumeOrAdopt,
    /// A `PhaseLaunchPacket` materialized for a phase hand-off.
    PhaseLaunchPacket,
    /// An Issue Monitor launch made with no window and nobody watching.
    SilentIssueMonitor,
    /// A `$gwt-execute` prompt launch.
    GwtExecute,
    /// Any launch surface that has not been tagged yet.
    #[default]
    Unspecified,
}

impl PermissionLaunchSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartWork => "start_work",
            Self::IssueMonitorProfile => "issue_monitor_profile",
            Self::LaunchWizardLastSettings => "launch_wizard_last_settings",
            Self::ResumeOrAdopt => "resume_or_adopt",
            Self::PhaseLaunchPacket => "phase_launch_packet",
            Self::SilentIssueMonitor => "silent_issue_monitor",
            Self::GwtExecute => "gwt_execute",
            Self::Unspecified => "unspecified",
        }
    }
}

/// The typed verdict of the decision model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionModeOutcome {
    /// The launch runs without permission prompts and the provider mapping
    /// that makes that true is known.
    SkipForcedReady,
    /// The provider has no skip-permissions mapping at all, so the request
    /// cannot be honored by any flag or overlay.
    UnsupportedProvider,
    /// A Custom Coding Agent was asked to skip permissions but defines no
    /// `skip_permissions_args`, so gwt has nothing to pass.
    MissingCustomSkipMapping,
    /// The mapping exists, the decision asked for it, and the materialized
    /// launch does not carry it. This is the regression detector.
    SkipFlagDropped,
    /// No forced skip applies and the launch keeps its interactive prompts.
    InteractiveRetained,
}

impl PermissionModeOutcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SkipForcedReady => "skip_forced_ready",
            Self::UnsupportedProvider => "unsupported_provider",
            Self::MissingCustomSkipMapping => "missing_custom_skip_mapping",
            Self::SkipFlagDropped => "skip_flag_dropped",
            Self::InteractiveRetained => "interactive_retained",
        }
    }

    /// Whether this outcome needs an operator or a follow-up to act.
    #[must_use]
    pub fn is_diagnostic(self) -> bool {
        matches!(
            self,
            Self::UnsupportedProvider | Self::MissingCustomSkipMapping | Self::SkipFlagDropped
        )
    }
}

/// One environment variable a provider uses to carry skip-permissions when it
/// has no CLI flag for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEnvOverlay {
    /// Environment variable the launch sets.
    pub key: String,
    /// Path the value points at, relative to the launch working directory.
    pub worktree_relative_path: String,
}

/// How one provider expresses "skip permissions", and whether it can at all.
///
/// This is the provider mapping evidence carried on every decision. It mirrors
/// the per-provider argument builders in [`crate::launch`]; the contract test
/// `provider_skip_mapping_matches_launch_materialization` pins the two
/// together so the table cannot drift away from what actually ships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSkipMapping {
    /// Stable provider key (`AgentId::as_str`-style, `custom:<id>` for a
    /// Custom Coding Agent).
    pub provider: String,
    /// Whether any mapping exists for this provider.
    pub supported: bool,
    /// Exact CLI arguments the launch appends for skip-permissions.
    pub args: Vec<String>,
    /// Exact environment overlay the launch sets for skip-permissions.
    pub env_overlay: Vec<ProviderEnvOverlay>,
}

impl ProviderSkipMapping {
    /// Whether this mapping carries anything the materialized launch can be
    /// checked against.
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        !self.args.is_empty() || !self.env_overlay.is_empty()
    }

    fn unsupported(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            supported: false,
            args: Vec::new(),
            env_overlay: Vec::new(),
        }
    }

    fn flags(provider: impl Into<String>, args: &[&str]) -> Self {
        Self {
            provider: provider.into(),
            supported: true,
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            env_overlay: Vec::new(),
        }
    }
}

/// Stable provider key for a launch.
#[must_use]
pub fn provider_key(agent_id: &AgentId, custom_agent: Option<&CustomCodingAgent>) -> String {
    if let Some(custom_agent) = custom_agent {
        return format!("custom:{}", custom_agent.id);
    }
    match agent_id {
        AgentId::ClaudeCode => "claude".to_string(),
        AgentId::Codex => "codex".to_string(),
        AgentId::GrokBuild => "grok".to_string(),
        AgentId::Antigravity => "antigravity".to_string(),
        AgentId::Gemini => "gemini".to_string(),
        AgentId::OpenCode => "opencode".to_string(),
        AgentId::OpenClaw => "openclaw".to_string(),
        AgentId::Hermes => "hermes".to_string(),
        AgentId::Copilot => "copilot".to_string(),
        AgentId::Custom(id) => format!("custom:{id}"),
    }
}

/// The single source of truth for how each provider expresses skip-permissions.
///
/// A Custom Coding Agent's mapping is whatever it declares; every other
/// provider's mapping mirrors its argument builder in [`crate::launch`].
#[must_use]
pub fn provider_skip_mapping(
    agent_id: &AgentId,
    custom_agent: Option<&CustomCodingAgent>,
) -> ProviderSkipMapping {
    let provider = provider_key(agent_id, custom_agent);
    if let Some(custom_agent) = custom_agent {
        return ProviderSkipMapping {
            provider,
            supported: !custom_agent.skip_permissions_args.is_empty(),
            args: custom_agent.skip_permissions_args.clone(),
            env_overlay: Vec::new(),
        };
    }
    match agent_id {
        AgentId::ClaudeCode | AgentId::Antigravity => {
            ProviderSkipMapping::flags(provider, &["--dangerously-skip-permissions"])
        }
        AgentId::Codex | AgentId::Gemini | AgentId::Hermes | AgentId::Copilot => {
            ProviderSkipMapping::flags(provider, &["--yolo"])
        }
        AgentId::GrokBuild => ProviderSkipMapping::flags(provider, &["--always-approve"]),
        // SPEC-3151 FR-005: OpenCode has no skip-permissions CLI flag; the
        // launch layers a permissive config overlay through OPENCODE_CONFIG.
        AgentId::OpenCode => ProviderSkipMapping {
            provider,
            supported: true,
            args: Vec::new(),
            env_overlay: vec![ProviderEnvOverlay {
                key: "OPENCODE_CONFIG".to_string(),
                worktree_relative_path: ".gwt/opencode/skip-permissions.json".to_string(),
            }],
        },
        // OpenClaw exposes neither a skip flag nor a permission overlay. A
        // producing-work launch on it cannot be made prompt-free today, and
        // saying so is the whole point of this outcome.
        AgentId::OpenClaw => ProviderSkipMapping::unsupported(provider),
        AgentId::Custom(_) => ProviderSkipMapping::unsupported(provider),
    }
}

/// Everything the decision model needs, and nothing it does not.
#[derive(Debug, Clone)]
pub struct PermissionModeInputs<'a> {
    pub agent_id: &'a AgentId,
    pub custom_agent: Option<&'a CustomCodingAgent>,
    pub source: PermissionLaunchSource,
    /// How the session was started (`$gwt-*` token, `resume`, `launch`).
    pub entrypoint: &'a str,
    pub launch_route: LaunchRoute,
    /// The permission preference the source carried. `None` means the source
    /// stored no preference at all — which used to be indistinguishable from
    /// an explicit "interactive, please".
    pub requested_skip_permissions: Option<bool>,
    /// Whether this launch produces work against an owner. Forced skip applies
    /// exactly here: an agent that is expected to land a change cannot stop at
    /// a prompt, whatever a stored setting says.
    pub producing_work: bool,
}

/// The recorded verdict, with the evidence it was reached from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionModeDecision {
    pub outcome: PermissionModeOutcome,
    pub provider: String,
    pub source: PermissionLaunchSource,
    pub entrypoint: String,
    pub launch_route: String,
    /// The preference the source carried, before the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_skip_permissions: Option<bool>,
    /// Whether the producing-work contract overrode that preference.
    pub skip_forced: bool,
    /// Whether the launch really runs without permission prompts.
    pub effective_skip_permissions: bool,
    /// Whether an explicit or implied interactive request was overridden.
    pub interactive_request_ignored: bool,
    pub reason: String,
    pub recovery_action: String,
    pub provider_mapping: ProviderSkipMapping,
    /// Set when the materialized launch failed the mapping check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_evidence: Option<String>,
    pub decided_at: DateTime<Utc>,
}

impl Default for PermissionModeDecision {
    /// An untagged, non-forced decision: what a launch config that was
    /// hand-assembled (rather than built) is entitled to claim.
    fn default() -> Self {
        Self {
            outcome: PermissionModeOutcome::InteractiveRetained,
            provider: String::new(),
            source: PermissionLaunchSource::Unspecified,
            entrypoint: String::new(),
            launch_route: LaunchRoute::Manual.as_str().to_string(),
            requested_skip_permissions: None,
            skip_forced: false,
            effective_skip_permissions: false,
            interactive_request_ignored: false,
            reason: "no permission mode decision was recorded for this launch".to_string(),
            recovery_action: "Build the launch through `AgentLaunchBuilder` so the permission mode decision is recorded.".to_string(),
            provider_mapping: ProviderSkipMapping::unsupported(String::new()),
            dropped_evidence: None,
            decided_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}

impl PermissionModeDecision {
    /// Whether this decision needs an operator or a follow-up to act.
    #[must_use]
    pub fn is_diagnostic(&self) -> bool {
        self.outcome.is_diagnostic()
    }

    /// Whether the launch should carry skip-permissions at all.
    ///
    /// Distinct from [`Self::effective_skip_permissions`]: a launch on a
    /// provider with no mapping still *asks* to skip. Enforcement of that gap
    /// is the dependent follow-up; today the launch proceeds and the decision
    /// records that it could not be honored.
    #[must_use]
    pub fn skip_permissions_requested(&self) -> bool {
        self.skip_forced || self.requested_skip_permissions == Some(true)
    }
}

/// Derive the execution entrypoint from a launch's arguments.
///
/// The last `$gwt-*` token in argv names the skill the session was started
/// for; otherwise the launch is identified by whether it resumes. Shared so
/// the permission decision and the Execution Control Record agree on what
/// "entrypoint" means for the same launch.
#[must_use]
pub fn entrypoint_from_launch_args(args: &[String], resume: bool) -> String {
    for arg in args.iter().rev() {
        let trimmed = arg.trim_start();
        if trimmed.starts_with("$gwt-") {
            if let Some(token) = trimmed.split_whitespace().next() {
                return token.trim_start_matches('$').to_string();
            }
        }
    }
    if resume {
        "resume".to_string()
    } else {
        "launch".to_string()
    }
}

/// Decide the permission mode for one launch.
#[must_use]
pub fn decide(inputs: &PermissionModeInputs<'_>) -> PermissionModeDecision {
    decide_with_mapping(
        provider_skip_mapping(inputs.agent_id, inputs.custom_agent),
        inputs.source,
        inputs.entrypoint,
        inputs.launch_route.as_str(),
        inputs.requested_skip_permissions,
        inputs.producing_work,
        inputs.custom_agent.is_some() || matches!(inputs.agent_id, AgentId::Custom(_)),
    )
}

/// Re-decide an already-materialized launch.
///
/// Two launch surfaces only identify themselves — or only force
/// skip-permissions — *after* the config exists: the Issue Monitor stamps its
/// own route onto a request the wizard already built. Re-deciding from the
/// mapping the first decision resolved keeps the recorded verdict describing
/// the launch that actually ships, instead of the one the builder guessed at.
#[must_use]
pub fn redecide_for_materialized_launch(
    previous: &PermissionModeDecision,
    source: PermissionLaunchSource,
    force_skip: bool,
    args: &[String],
    env_vars: &HashMap<String, String>,
) -> PermissionModeDecision {
    let decision = decide_with_mapping(
        previous.provider_mapping.clone(),
        source,
        &previous.entrypoint,
        &previous.launch_route,
        previous.requested_skip_permissions,
        previous.skip_forced || force_skip,
        previous.provider.starts_with("custom:"),
    );
    validate_materialized_launch(decision, args, env_vars)
}

fn decide_with_mapping(
    mapping: ProviderSkipMapping,
    source: PermissionLaunchSource,
    entrypoint: &str,
    launch_route: &str,
    requested_skip_permissions: Option<bool>,
    producing_work: bool,
    is_custom: bool,
) -> PermissionModeDecision {
    let requested = requested_skip_permissions.unwrap_or(false);
    let skip_forced = producing_work && !requested;
    let wants_skip = producing_work || requested;
    let interactive_request_ignored = skip_forced;

    let (outcome, reason, recovery_action) = if !wants_skip {
        (
            PermissionModeOutcome::InteractiveRetained,
            format!(
                "{} launch is not producing work and did not ask to skip permissions",
                source.as_str()
            ),
            "No action: the launch keeps its interactive permission prompts.".to_string(),
        )
    } else if is_custom && !mapping.supported {
        (
            PermissionModeOutcome::MissingCustomSkipMapping,
            format!(
                "custom agent `{}` must skip permissions but declares no skip_permissions_args",
                mapping.provider
            ),
            "Add the agent's own skip-permissions arguments to its External Agent entry (`skip_permissions_args`), then relaunch.".to_string(),
        )
    } else if !mapping.supported {
        (
            PermissionModeOutcome::UnsupportedProvider,
            format!(
                "provider `{}` exposes no skip-permissions flag or config overlay",
                mapping.provider
            ),
            "Launch this owner on a provider that supports skip-permissions, or run it as a non-producing session.".to_string(),
        )
    } else {
        (
            PermissionModeOutcome::SkipForcedReady,
            if skip_forced {
                format!(
                    "producing-work launch forces skip-permissions over the {} preference ({})",
                    source.as_str(),
                    describe_request(requested_skip_permissions)
                )
            } else {
                format!("{} launch asked to skip permissions", source.as_str())
            },
            "No action: the provider mapping below carries skip-permissions into the launch."
                .to_string(),
        )
    };

    PermissionModeDecision {
        effective_skip_permissions: outcome == PermissionModeOutcome::SkipForcedReady,
        outcome,
        provider: mapping.provider.clone(),
        source,
        entrypoint: entrypoint.to_string(),
        launch_route: launch_route.to_string(),
        requested_skip_permissions,
        skip_forced,
        interactive_request_ignored,
        reason,
        recovery_action,
        provider_mapping: mapping,
        dropped_evidence: None,
        decided_at: Utc::now(),
    }
}

fn describe_request(requested: Option<bool>) -> &'static str {
    match requested {
        Some(true) => "skip",
        Some(false) => "interactive",
        None => "unset",
    }
}

/// Check a materialized launch against the decision that authorized it.
///
/// This is the shared validator every launch entry point runs after the argv
/// and environment exist. A decision that claimed the launch would be
/// prompt-free and then finds its own mapping evidence missing is downgraded to
/// [`PermissionModeOutcome::SkipFlagDropped`] — the regression that silently
/// stalled monitor launches now names itself.
#[must_use]
pub fn validate_materialized_launch(
    decision: PermissionModeDecision,
    args: &[String],
    env_vars: &HashMap<String, String>,
) -> PermissionModeDecision {
    if decision.outcome != PermissionModeOutcome::SkipForcedReady {
        return decision;
    }
    let missing_args: Vec<&str> = decision
        .provider_mapping
        .args
        .iter()
        .filter(|expected| !args.iter().any(|arg| arg == *expected))
        .map(String::as_str)
        .collect();
    let missing_env: Vec<&str> = decision
        .provider_mapping
        .env_overlay
        .iter()
        .filter(|overlay| {
            env_vars.get(&overlay.key).is_none_or(|value| {
                !value
                    .replace('\\', "/")
                    .ends_with(&overlay.worktree_relative_path)
            })
        })
        .map(|overlay| overlay.key.as_str())
        .collect();

    if missing_args.is_empty() && missing_env.is_empty() {
        return decision;
    }

    let mut dropped = Vec::new();
    if !missing_args.is_empty() {
        dropped.push(format!("missing args: {}", missing_args.join(", ")));
    }
    if !missing_env.is_empty() {
        dropped.push(format!("missing env overlay: {}", missing_env.join(", ")));
    }
    let dropped = dropped.join("; ");

    PermissionModeDecision {
        outcome: PermissionModeOutcome::SkipFlagDropped,
        effective_skip_permissions: false,
        reason: format!(
            "provider `{}` was decided skip-forced but the materialized launch dropped it ({dropped})",
            decision.provider
        ),
        recovery_action:
            "This is a launch-materialization regression: the provider mapping and the argument builder disagree. Fix the builder, do not relaunch."
                .to_string(),
        dropped_evidence: Some(dropped),
        ..decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom::{CustomAgentType, CustomCodingAgent};
    use crate::launch::AgentLaunchBuilder;

    fn custom_agent(id: &str, skip_args: &[&str]) -> CustomCodingAgent {
        CustomCodingAgent {
            id: id.to_string(),
            display_name: id.to_string(),
            agent_type: CustomAgentType::Command,
            command: "my-agent".to_string(),
            default_args: Vec::new(),
            mode_args: None,
            skip_permissions_args: skip_args.iter().map(|arg| (*arg).to_string()).collect(),
            env: HashMap::new(),
            supports_resume_picker: false,
        }
    }

    fn producing(agent_id: &AgentId, requested: Option<bool>) -> PermissionModeInputs<'_> {
        PermissionModeInputs {
            agent_id,
            custom_agent: None,
            source: PermissionLaunchSource::IssueMonitorProfile,
            entrypoint: "$gwt-execute",
            launch_route: LaunchRoute::Autonomous,
            requested_skip_permissions: requested,
            producing_work: true,
        }
    }

    /// AC-1: every provider the Issue names resolves to a typed decision that
    /// carries its own mapping evidence.
    #[test]
    fn decide_covers_every_provider_with_reason_recovery_and_mapping_evidence() {
        let supported = [
            (AgentId::Codex, vec!["--yolo"]),
            (AgentId::ClaudeCode, vec!["--dangerously-skip-permissions"]),
            (AgentId::Gemini, vec!["--yolo"]),
            (AgentId::Copilot, vec!["--yolo"]),
            (AgentId::Antigravity, vec!["--dangerously-skip-permissions"]),
            (AgentId::GrokBuild, vec!["--always-approve"]),
            (AgentId::Hermes, vec!["--yolo"]),
        ];
        for (agent_id, expected_args) in supported {
            let decision = decide(&producing(&agent_id, None));
            assert_eq!(
                decision.outcome,
                PermissionModeOutcome::SkipForcedReady,
                "{agent_id:?} must be skip_forced_ready"
            );
            assert!(decision.effective_skip_permissions, "{agent_id:?}");
            assert_eq!(
                decision.provider_mapping.args, expected_args,
                "{agent_id:?}"
            );
            assert!(!decision.reason.is_empty(), "{agent_id:?}");
            assert!(!decision.recovery_action.is_empty(), "{agent_id:?}");
        }

        // OpenCode carries a config overlay instead of a flag.
        let opencode = decide(&producing(&AgentId::OpenCode, None));
        assert_eq!(opencode.outcome, PermissionModeOutcome::SkipForcedReady);
        assert!(opencode.provider_mapping.args.is_empty());
        assert_eq!(
            opencode.provider_mapping.env_overlay,
            vec![ProviderEnvOverlay {
                key: "OPENCODE_CONFIG".to_string(),
                worktree_relative_path: ".gwt/opencode/skip-permissions.json".to_string(),
            }]
        );

        // OpenClaw has neither.
        let openclaw = decide(&producing(&AgentId::OpenClaw, None));
        assert_eq!(openclaw.outcome, PermissionModeOutcome::UnsupportedProvider);
        assert!(!openclaw.effective_skip_permissions);
        assert!(openclaw.reason.contains("openclaw"), "{}", openclaw.reason);
        assert!(!openclaw.recovery_action.is_empty());

        // A Custom Coding Agent's mapping is whatever it declares.
        let mapped = custom_agent("acme", &["--trust-me"]);
        let decision = decide(&PermissionModeInputs {
            custom_agent: Some(&mapped),
            ..producing(&AgentId::Custom("acme".to_string()), None)
        });
        assert_eq!(decision.outcome, PermissionModeOutcome::SkipForcedReady);
        assert_eq!(decision.provider, "custom:acme");
        assert_eq!(decision.provider_mapping.args, vec!["--trust-me"]);

        let unmapped = custom_agent("bare", &[]);
        let decision = decide(&PermissionModeInputs {
            custom_agent: Some(&unmapped),
            ..producing(&AgentId::Custom("bare".to_string()), None)
        });
        assert_eq!(
            decision.outcome,
            PermissionModeOutcome::MissingCustomSkipMapping
        );
        assert!(
            decision.recovery_action.contains("skip_permissions_args"),
            "{}",
            decision.recovery_action
        );
    }

    /// AC-2: the mapping table is the same contract the launch builder ships.
    /// This pins the two together so the table cannot drift from reality.
    #[test]
    fn provider_skip_mapping_matches_launch_materialization() {
        let builtins = [
            AgentId::ClaudeCode,
            AgentId::Codex,
            AgentId::GrokBuild,
            AgentId::Antigravity,
            AgentId::Gemini,
            AgentId::OpenCode,
            AgentId::OpenClaw,
            AgentId::Hermes,
            AgentId::Copilot,
        ];
        let dir = std::env::temp_dir().join("gwt-permission-mode-contract");
        for agent_id in builtins {
            let mapping = provider_skip_mapping(&agent_id, None);
            let skipped = AgentLaunchBuilder::new(agent_id.clone())
                .working_dir(dir.clone())
                .skip_permissions(true)
                .build();
            let plain = AgentLaunchBuilder::new(agent_id.clone())
                .working_dir(dir.clone())
                .skip_permissions(false)
                .build();

            for expected in &mapping.args {
                assert!(
                    skipped.args.contains(expected),
                    "{agent_id:?} skip launch must carry {expected}: {:?}",
                    skipped.args
                );
                assert!(
                    !plain.args.contains(expected),
                    "{agent_id:?} non-skip launch must not carry {expected}: {:?}",
                    plain.args
                );
            }
            for overlay in &mapping.env_overlay {
                let value = skipped
                    .env_vars
                    .get(&overlay.key)
                    .unwrap_or_else(|| panic!("{agent_id:?} skip launch must set {}", overlay.key));
                assert!(
                    value
                        .replace('\\', "/")
                        .ends_with(&overlay.worktree_relative_path),
                    "{agent_id:?} overlay value {value} must point at {}",
                    overlay.worktree_relative_path
                );
                assert!(
                    !plain.env_vars.contains_key(&overlay.key),
                    "{agent_id:?} non-skip launch must not set {}",
                    overlay.key
                );
            }
            if !mapping.supported {
                // An unsupported provider must genuinely have nothing: if the
                // launch grew a mapping, this table is the thing that is stale.
                assert_eq!(
                    skipped.args, plain.args,
                    "{agent_id:?} is recorded unsupported but its launch args changed under skip_permissions"
                );
                assert_eq!(
                    skipped.env_vars.len(),
                    plain.env_vars.len(),
                    "{agent_id:?} is recorded unsupported but its launch env changed under skip_permissions"
                );
            }
        }
    }

    /// AC-3 / AC-4: a producing-work launch forces skip over a stored `false`
    /// or a missing preference, from every source that stores one.
    #[test]
    fn producing_work_forces_skip_over_every_stored_interactive_preference() {
        let sources = [
            PermissionLaunchSource::StartWork,
            PermissionLaunchSource::IssueMonitorProfile,
            PermissionLaunchSource::LaunchWizardLastSettings,
            PermissionLaunchSource::ResumeOrAdopt,
            PermissionLaunchSource::PhaseLaunchPacket,
            PermissionLaunchSource::SilentIssueMonitor,
            PermissionLaunchSource::GwtExecute,
        ];
        for source in sources {
            for requested in [None, Some(false)] {
                let agent_id = AgentId::Codex;
                let decision = decide(&PermissionModeInputs {
                    source,
                    ..producing(&agent_id, requested)
                });
                assert_eq!(
                    decision.outcome,
                    PermissionModeOutcome::SkipForcedReady,
                    "{source:?} / {requested:?}"
                );
                assert!(decision.skip_forced, "{source:?} / {requested:?}");
                assert!(
                    decision.interactive_request_ignored,
                    "{source:?} / {requested:?}"
                );
                assert!(
                    decision.effective_skip_permissions,
                    "{source:?} / {requested:?}"
                );
                assert_eq!(decision.source, source);
            }
        }
    }

    /// AC-4: an interactive request on an unsupported provider becomes the
    /// unsupported block, never a silent interactive launch.
    #[test]
    fn producing_work_interactive_request_on_unsupported_provider_blocks() {
        let agent_id = AgentId::OpenClaw;
        let decision = decide(&PermissionModeInputs {
            source: PermissionLaunchSource::LaunchWizardLastSettings,
            ..producing(&agent_id, Some(false))
        });
        assert_eq!(decision.outcome, PermissionModeOutcome::UnsupportedProvider);
        assert!(decision.interactive_request_ignored);
        assert!(!decision.effective_skip_permissions);
    }

    /// A non-producing launch is untouched: this contract only governs agents
    /// that are expected to land a change.
    #[test]
    fn non_producing_launch_keeps_its_interactive_prompts() {
        let agent_id = AgentId::ClaudeCode;
        let decision = decide(&PermissionModeInputs {
            source: PermissionLaunchSource::StartWork,
            launch_route: LaunchRoute::Manual,
            producing_work: false,
            ..producing(&agent_id, Some(false))
        });
        assert_eq!(decision.outcome, PermissionModeOutcome::InteractiveRetained);
        assert!(!decision.skip_forced);
        assert!(!decision.interactive_request_ignored);
        assert!(!decision.effective_skip_permissions);
    }

    /// A non-producing launch that explicitly asked to skip still gets its
    /// mapping — it is just not *forced*.
    #[test]
    fn non_producing_explicit_skip_is_honored_without_being_forced() {
        let agent_id = AgentId::Codex;
        let decision = decide(&PermissionModeInputs {
            source: PermissionLaunchSource::StartWork,
            launch_route: LaunchRoute::Manual,
            producing_work: false,
            ..producing(&agent_id, Some(true))
        });
        assert_eq!(decision.outcome, PermissionModeOutcome::SkipForcedReady);
        assert!(!decision.skip_forced);
        assert!(!decision.interactive_request_ignored);
        assert!(decision.effective_skip_permissions);
    }

    /// AC-3: a dropped flag is a failure, not a silent interactive launch.
    #[test]
    fn validator_downgrades_a_launch_that_dropped_the_mapped_flag() {
        let agent_id = AgentId::Codex;
        let decision = decide(&producing(&agent_id, None));
        let validated = validate_materialized_launch(decision.clone(), &[], &HashMap::new());
        assert_eq!(validated.outcome, PermissionModeOutcome::SkipFlagDropped);
        assert!(!validated.effective_skip_permissions);
        assert!(validated.is_diagnostic());
        assert!(
            validated
                .dropped_evidence
                .as_deref()
                .is_some_and(|evidence| evidence.contains("--yolo")),
            "{:?}",
            validated.dropped_evidence
        );

        let kept = validate_materialized_launch(decision, &["--yolo".to_string()], &HashMap::new());
        assert_eq!(kept.outcome, PermissionModeOutcome::SkipForcedReady);
        assert!(kept.dropped_evidence.is_none());
    }

    /// The overlay half of the same check.
    #[test]
    fn validator_downgrades_a_launch_that_dropped_the_config_overlay() {
        let agent_id = AgentId::OpenCode;
        let decision = decide(&producing(&agent_id, None));
        let dropped = validate_materialized_launch(decision.clone(), &[], &HashMap::new());
        assert_eq!(dropped.outcome, PermissionModeOutcome::SkipFlagDropped);
        assert!(dropped
            .dropped_evidence
            .as_deref()
            .is_some_and(|evidence| evidence.contains("OPENCODE_CONFIG")));

        let env = HashMap::from([(
            "OPENCODE_CONFIG".to_string(),
            "/tmp/wt/.gwt/opencode/skip-permissions.json".to_string(),
        )]);
        let kept = validate_materialized_launch(decision, &[], &env);
        assert_eq!(kept.outcome, PermissionModeOutcome::SkipForcedReady);
    }

    /// A diagnostic outcome is never rewritten by the validator: only a
    /// decision that claimed readiness can lose it.
    #[test]
    fn validator_leaves_a_diagnostic_decision_alone() {
        let agent_id = AgentId::OpenClaw;
        let decision = decide(&producing(&agent_id, None));
        let validated = validate_materialized_launch(decision, &[], &HashMap::new());
        assert_eq!(
            validated.outcome,
            PermissionModeOutcome::UnsupportedProvider
        );
        assert!(validated.dropped_evidence.is_none());
    }

    /// AC-3 / AC-6: the contract is wired into launch materialization itself,
    /// so a linked-owner launch from any source carries the provider's skip
    /// flag even when the source stored `false` or stored nothing.
    #[test]
    fn linked_owner_launch_forces_skip_through_launch_materialization() {
        let sources = [
            PermissionLaunchSource::StartWork,
            PermissionLaunchSource::IssueMonitorProfile,
            PermissionLaunchSource::LaunchWizardLastSettings,
            PermissionLaunchSource::ResumeOrAdopt,
            PermissionLaunchSource::PhaseLaunchPacket,
            PermissionLaunchSource::SilentIssueMonitor,
            PermissionLaunchSource::GwtExecute,
        ];
        for source in sources {
            // `stored_false` opts out explicitly; `unset` never calls the
            // setter at all. Both used to reach the agent as an interactive
            // launch that then stalled on a prompt.
            let stored_false = AgentLaunchBuilder::new(AgentId::Codex)
                .linked_issue_number(4543)
                .permission_launch_source(source)
                .skip_permissions(false)
                .build();
            let unset = AgentLaunchBuilder::new(AgentId::Codex)
                .linked_issue_number(4543)
                .permission_launch_source(source)
                .build();

            for config in [&stored_false, &unset] {
                assert!(config.skip_permissions, "{source:?}");
                assert!(
                    config.args.contains(&"--yolo".to_string()),
                    "{source:?} must carry Codex's skip flag: {:?}",
                    config.args
                );
                let decision = &config.permission_decision;
                assert_eq!(
                    decision.outcome,
                    PermissionModeOutcome::SkipForcedReady,
                    "{source:?}"
                );
                assert_eq!(decision.source, source);
                assert!(decision.skip_forced, "{source:?}");
                assert!(decision.interactive_request_ignored, "{source:?}");
                assert_eq!(decision.provider, "codex");
            }
            assert_eq!(
                stored_false.permission_decision.requested_skip_permissions,
                Some(false)
            );
            assert_eq!(unset.permission_decision.requested_skip_permissions, None);
        }
    }

    /// A launch with no linked owner produces no work, so nothing is forced.
    #[test]
    fn unlinked_launch_materialization_keeps_its_stored_preference() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .permission_launch_source(PermissionLaunchSource::StartWork)
            .skip_permissions(false)
            .build();
        assert!(!config.skip_permissions);
        assert!(!config.args.contains(&"--yolo".to_string()));
        assert_eq!(
            config.permission_decision.outcome,
            PermissionModeOutcome::InteractiveRetained
        );
    }

    /// A launch that is subordinate to another session's execution does not
    /// produce work of its own, so it keeps its own preference.
    #[test]
    fn suppressed_execution_control_launch_keeps_its_stored_preference() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .linked_issue_number(4543)
            .suppress_execution_control()
            .permission_launch_source(PermissionLaunchSource::IssueMonitorProfile)
            .skip_permissions(false)
            .build();
        assert!(!config.skip_permissions);
        assert_eq!(
            config.permission_decision.outcome,
            PermissionModeOutcome::InteractiveRetained
        );
    }

    /// AC-6: the wiring is in, blocking is not. An unsupported provider still
    /// launches — it just stops being invisible.
    #[test]
    fn unsupported_provider_launch_is_recorded_but_not_blocked() {
        let config = AgentLaunchBuilder::new(AgentId::OpenClaw)
            .linked_issue_number(4543)
            .permission_launch_source(PermissionLaunchSource::SilentIssueMonitor)
            .build();
        assert_eq!(
            config.permission_decision.outcome,
            PermissionModeOutcome::UnsupportedProvider
        );
        assert!(config.permission_decision.is_diagnostic());
        assert!(!config.permission_decision.effective_skip_permissions);
        assert!(!config.command.is_empty(), "the launch still materializes");
    }

    /// AC-1: the entrypoint the decision records is the one the launch was
    /// started for, so the record answers "which skill asked for this?".
    #[test]
    fn decision_records_the_launch_entrypoint() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .linked_issue_number(4543)
            .extra_arg("$gwt-execute #4543".to_string())
            .build();
        assert_eq!(config.permission_decision.entrypoint, "gwt-execute");

        let resumed = AgentLaunchBuilder::new(AgentId::Codex)
            .linked_issue_number(4543)
            .session_mode(crate::types::SessionMode::Resume)
            .build();
        assert_eq!(resumed.permission_decision.entrypoint, "resume");

        let plain = AgentLaunchBuilder::new(AgentId::Codex)
            .linked_issue_number(4543)
            .build();
        assert_eq!(plain.permission_decision.entrypoint, "launch");
    }

    /// AC-5 serializes into the Execution Control Record, so the decision must
    /// round-trip through serde without losing its evidence.
    #[test]
    fn decision_round_trips_through_serde() {
        let agent_id = AgentId::OpenCode;
        let decision = decide(&producing(&agent_id, Some(false)));
        let json = serde_json::to_string(&decision).expect("serialize decision");
        let restored: PermissionModeDecision =
            serde_json::from_str(&json).expect("deserialize decision");
        assert_eq!(restored, decision);
        assert!(json.contains("skip_forced_ready"), "{json}");
        assert!(json.contains("issue_monitor_profile"), "{json}");
    }
}
