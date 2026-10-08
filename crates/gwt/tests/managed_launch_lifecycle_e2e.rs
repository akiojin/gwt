//! Issue #4544 AC-6: the managed launch lifecycle, end to end.
//!
//! Six things have to hold together for a managed launch to run unattended and
//! still be deliverable, and each of them used to be provable only by watching
//! a pane:
//!
//! 1. a producing-work launch forces skip-permissions even when nothing was
//!    saved about it,
//! 2. a provider that cannot skip permissions is blocked before the launch,
//! 3. a launch that *can* skip permissions produces no gate decision and stays
//!    deliverable,
//! 4. a dropped skip-permissions flag blocks and captures a work note,
//! 5. no interactive route survives for a producing-work launch, whatever the
//!    surface asked for, and
//! 6. skip-permissions being in force does not exempt anything from the PR and
//!    verification gates.
//!
//! These run against the real decision model and the real gate records rather
//! than a fake, because the failure this Issue is about — an agent sitting at a
//! prompt nobody is watching — is precisely the case where a fake would have
//! agreed that everything was fine.

use std::path::Path;

use gwt::cli::permission_readiness::{self, PermissionReadinessKind, PermissionReadinessRecord};
use gwt_agent::{
    AgentId, CustomCodingAgent, LaunchRoute, PermissionLaunchSource, PermissionModeDecision,
    PermissionModeInputs, PermissionModeOutcome,
};

fn decide(
    agent_id: &AgentId,
    custom_agent: Option<&CustomCodingAgent>,
    source: PermissionLaunchSource,
    requested_skip_permissions: Option<bool>,
    producing_work: bool,
) -> PermissionModeDecision {
    gwt_agent::decide_permission_mode(&PermissionModeInputs {
        agent_id,
        custom_agent,
        source,
        entrypoint: "gwt-execute",
        launch_route: LaunchRoute::Autonomous,
        requested_skip_permissions,
        producing_work,
    })
}

fn block_for(decision: &PermissionModeDecision) -> Option<PermissionReadinessRecord> {
    permission_readiness::pre_launch_block("issue", 4544, "monitor-launch:4544", decision)
}

/// One test's isolated gwt home plus the worktrees that live under it.
///
/// The gwt home is scoped with [`ScopedGwtHome`], which is *thread-local*
/// rather than an environment variable. That matters: an earlier version of
/// this file set `HOME` in one test while the others read it unlocked, and the
/// tempdir behind it was removed while a sibling test was mid-write — the
/// trusted-store write then failed `NotFound` under CI's parallelism and
/// passed locally. Thread-local isolation removes the shared state instead of
/// serializing access to it.
struct Fixture {
    _home: tempfile::TempDir,
    _scope: gwt_core::test_support::ScopedGwtHome,
    worktrees: Vec<tempfile::TempDir>,
}

impl Fixture {
    /// `count` real Git worktrees with an origin, so the trusted store
    /// resolves a repository-scoped directory instead of the degenerate
    /// mirror-only mode.
    fn with_worktrees(count: usize) -> Self {
        let home = tempfile::tempdir().expect("home tempdir");
        let scope = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let worktrees = (0..count).map(|_| git_worktree()).collect();
        Self {
            _home: home,
            _scope: scope,
            worktrees,
        }
    }

    fn path(&self, index: usize) -> &Path {
        self.worktrees[index].path()
    }
}

fn git_worktree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(gwt_core::process::hidden_command("git")
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("git init")
        .success());
    assert!(gwt_core::process::hidden_command("git")
        .arg("-C")
        .arg(dir.path())
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-managed-launch.git",
        ])
        .status()
        .expect("git remote add")
        .success());
    dir
}

/// (1) The saved preference is absent — the case that used to be
/// indistinguishable from an explicit "interactive, please".
#[test]
fn a_producing_launch_with_no_saved_preference_is_forced_prompt_free() {
    for agent_id in [
        AgentId::Codex,
        AgentId::ClaudeCode,
        AgentId::Copilot,
        AgentId::Antigravity,
        AgentId::OpenCode,
    ] {
        let decision = decide(
            &agent_id,
            None,
            PermissionLaunchSource::SilentIssueMonitor,
            None,
            true,
        );

        assert_eq!(
            decision.outcome,
            PermissionModeOutcome::SkipForcedReady,
            "{agent_id:?} must launch prompt-free with no stored preference"
        );
        assert!(decision.skip_forced);
        assert!(decision.effective_skip_permissions);
        assert!(
            block_for(&decision).is_none(),
            "{agent_id:?} is supported, so nothing may block it"
        );
    }
}

/// (2) A provider with no mapping at all is blocked before the launch, and the
/// block says what to do about it.
#[test]
fn an_unsupported_provider_is_blocked_before_the_launch_with_a_recovery_action() {
    let fx = Fixture::with_worktrees(1);
    let dir = fx.path(0);
    let decision = decide(
        &AgentId::OpenClaw,
        None,
        PermissionLaunchSource::SilentIssueMonitor,
        None,
        true,
    );
    assert_eq!(decision.outcome, PermissionModeOutcome::UnsupportedProvider);

    let refusal = permission_readiness::block_launch_if_unready(
        dir,
        "issue",
        4544,
        "monitor-launch:4544",
        &decision,
    )
    .expect_err("an unsupported provider must not launch");

    assert!(refusal.starts_with("launch refused:"), "{refusal}");
    assert!(refusal.contains("Recovery:"), "{refusal}");
    assert!(refusal.contains("unsupported"), "{refusal}");

    // The block is durable, so `execution.status` can answer for it later.
    let record = permission_readiness::load(dir)
        .expect("load")
        .expect("the block is recorded");
    assert_eq!(record.kind, PermissionReadinessKind::PreLaunchBlock);
    assert!(record.integrity_ok());
    assert!(!record.recovery_action.is_empty());

    // A custom agent that declares no skip args is the same fault with a
    // different cause, and must be refused the same way.
    let custom = CustomCodingAgent {
        id: "house-agent".to_string(),
        display_name: "House Agent".to_string(),
        agent_type: gwt_agent::custom::CustomAgentType::Command,
        command: "house-agent".to_string(),
        default_args: Vec::new(),
        mode_args: None,
        skip_permissions_args: Vec::new(),
        env: Default::default(),
        supports_resume_picker: false,
    };
    let custom_decision = decide(
        &AgentId::Custom("house-agent".to_string()),
        Some(&custom),
        PermissionLaunchSource::SilentIssueMonitor,
        None,
        true,
    );
    assert_eq!(
        custom_decision.outcome,
        PermissionModeOutcome::MissingCustomSkipMapping
    );
    assert!(block_for(&custom_decision).is_some());
}

/// (3) The ordinary case: a supported provider runs, and nothing in the
/// lifecycle is refused because of permissions.
#[test]
fn a_supported_launch_leaves_no_gate_decision_and_nothing_is_refused() {
    let fx = Fixture::with_worktrees(1);
    let dir = fx.path(0);
    let decision = decide(
        &AgentId::Codex,
        None,
        PermissionLaunchSource::SilentIssueMonitor,
        None,
        true,
    );

    permission_readiness::block_launch_if_unready(
        dir,
        "issue",
        4544,
        "monitor-launch:4544",
        &decision,
    )
    .expect("a supported launch proceeds");

    assert!(permission_readiness::load(dir).expect("load").is_none());
    assert!(permission_readiness::settlement_refusal(dir).is_none());
}

/// (4) The mapping existed, the launch asked for it, and it did not reach the
/// materialized launch. That is gwt's own fault, so it blocks *and* leaves a
/// work note for the next generation.
#[test]
fn a_dropped_skip_flag_blocks_and_captures_a_work_note() {
    let fx = Fixture::with_worktrees(1);
    let dir = fx.path(0);

    // The launch was decided prompt-free, and the argv that shipped does not
    // carry the flag the mapping named.
    let decided = decide(
        &AgentId::Codex,
        None,
        PermissionLaunchSource::SilentIssueMonitor,
        None,
        true,
    );
    let dropped = gwt_agent::validate_materialized_launch(
        decided,
        &["--model".to_string(), "gpt-5.4".to_string()],
        &Default::default(),
    );
    assert_eq!(dropped.outcome, PermissionModeOutcome::SkipFlagDropped);

    let refusal = permission_readiness::block_launch_if_unready(
        dir,
        "issue",
        4544,
        "monitor-launch:4544",
        &dropped,
    )
    .expect_err("a dropped flag must not launch");
    assert!(refusal.contains("Recovery:"), "{refusal}");

    let memory = std::fs::read_to_string(gwt_core::paths::gwt_work_notes_memory_path(dir))
        .expect("the work-notes memory file exists");
    assert!(memory.contains("skip-permissions"), "{memory}");
    assert!(memory.contains("Future Action:"), "{memory}");
}

/// (5) No surface can keep a producing-work launch interactive — the whole
/// point of the contract. The same surfaces leave a non-producing launch
/// alone, so the forcing is scoped rather than global.
#[test]
fn no_surface_can_keep_a_producing_launch_interactive() {
    let surfaces = [
        PermissionLaunchSource::StartWork,
        PermissionLaunchSource::IssueMonitorProfile,
        PermissionLaunchSource::LaunchWizardLastSettings,
        PermissionLaunchSource::ResumeOrAdopt,
        PermissionLaunchSource::PhaseLaunchPacket,
        PermissionLaunchSource::SilentIssueMonitor,
        PermissionLaunchSource::GwtExecute,
    ];

    for source in surfaces {
        for requested in [None, Some(false), Some(true)] {
            let decision = decide(&AgentId::Codex, None, source, requested, true);
            assert_ne!(
                decision.outcome,
                PermissionModeOutcome::InteractiveRetained,
                "{source:?} with {requested:?} must not keep a producing launch interactive"
            );
            assert!(decision.effective_skip_permissions);
        }

        // A launch that produces nothing keeps whatever the surface stored.
        let unlinked = decide(&AgentId::Codex, None, source, Some(false), false);
        assert_eq!(unlinked.outcome, PermissionModeOutcome::InteractiveRetained);
        assert!(!unlinked.effective_skip_permissions);
    }
}

/// (6) Skip-permissions being in force is about prompts, not about proof. A
/// prompt-free launch that then prompts anyway still cannot settle anything.
#[test]
fn skip_permissions_does_not_exempt_the_pr_or_verification_gates() {
    let fx = Fixture::with_worktrees(1);
    let dir = fx.path(0);
    let decision = decide(
        &AgentId::Codex,
        None,
        PermissionLaunchSource::SilentIssueMonitor,
        None,
        true,
    );
    assert!(decision.effective_skip_permissions);
    permission_readiness::block_launch_if_unready(
        dir,
        "issue",
        4544,
        "monitor-launch:4544",
        &decision,
    )
    .expect("a supported launch proceeds");

    // Nothing refuses yet.
    assert!(permission_readiness::settlement_refusal(dir).is_none());

    // The launch prompts anyway. Every settlement now refuses, through the one
    // predicate `execution.complete`, the Ready PR gate, `verify.run`, and the
    // monitor's Deliver routing all consume.
    permission_readiness::record_prompt_regression(
        dir,
        "issue",
        4544,
        "sess-4544",
        "codex",
        0xfeed,
    )
    .expect("record the regression");

    let refusal = permission_readiness::settlement_refusal(dir).expect("settlement is refused");
    assert!(
        refusal.contains("permission_prompt_regression"),
        "{refusal}"
    );
    assert!(refusal.contains("Recovery:"), "{refusal}");

    assert_gate_text_is_not_misleading(&refusal);
}

fn assert_gate_text_is_not_misleading(text: &str) {
    let lowered = text.to_ascii_lowercase();
    for misleading in [
        "implementation started",
        "tests ran",
        "tests passed",
        "verification passed",
    ] {
        assert!(
            !lowered.contains(misleading),
            "a gate refusal must not imply {misleading}: {text}"
        );
    }
}

/// Issue #4544 AC-1 / AC-5: the gate has to sit *above* everything the AC
/// names, and ordering is the one property a behavioural test cannot show —
/// a gate that refuses correctly but one statement too late has already
/// injected the prompt, opened the window, and materialized the record the
/// pass evidence hangs off.
///
/// Both launch owners are checked, because the independent review agent
/// deliberately carries no Execution Control Record and so never reaches the
/// producing-owner gate in `launch.rs`.
#[test]
fn the_permission_gate_precedes_the_window_on_every_managed_launch_route() {
    let root = repo_root();

    let launch = read_source(&root, "crates/gwt/src/app_runtime/launch.rs");
    let gate = launch
        .find("permission_readiness::block_launch_if_unready")
        .expect("the producing-owner launch route must consult the permission gate");
    let install = launch
        .find("let capability_install = FinalizedAgentCapabilityLaunch {")
        .expect("the capability install is the point everything observable starts at");
    assert!(
        gate < install,
        "the permission gate must refuse before capability issuance, the Execution Control Record, and the PTY"
    );

    let wizard = read_source(&root, "crates/gwt/src/app_runtime/wizard.rs");
    let monitor_gate = wizard
        .find("permission_readiness::pre_launch_block")
        .expect("the Issue Monitor launch route must consult the permission gate");
    let spawn = wizard
        .find("spawn_agent_window_with_feedback_at_geometry")
        .expect("the monitor route spawns its window here");
    assert!(
        monitor_gate < spawn,
        "the monitor route must refuse before the agent window exists"
    );

    // AC-2's half of the same ordering: above the GitHub claim, not below it.
    let monitor = read_source(&root, "crates/gwt/src/issue_monitor.rs");
    let block = monitor
        .find("self.permission_readiness_block_for_launch(")
        .expect("the claim loop must consult the permission gate");
    let acquire = monitor
        .find("match acquire_claim(client,")
        .expect("the claim loop acquires the GitHub claim here");
    assert!(
        block < acquire,
        "the claim loop must refuse before a GitHub claim comment is written"
    );
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_source(root: &Path, relative: &str) -> String {
    let path = root.join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

/// The record is worktree-scoped, so one execution's block must not refuse a
/// different worktree's settlement.
#[test]
fn a_block_in_one_worktree_does_not_refuse_another() {
    let fx = Fixture::with_worktrees(2);
    let (blocked, clear) = (fx.path(0), fx.path(1));
    permission_readiness::record_prompt_regression(blocked, "issue", 4544, "sess-4544", "codex", 1)
        .unwrap();

    assert!(permission_readiness::settlement_refusal(blocked).is_some());
    assert!(permission_readiness::settlement_refusal(clear).is_none());
    assert!(permission_readiness::settlement_refusal(Path::new("/nonexistent-worktree")).is_none());
}
