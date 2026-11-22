//! Issue #7771 (SPEC #7007 mock v3): a new launch that starts work on an
//! Issue is planned in three stages — Issue & assignee, Launch plan,
//! Confirm — after the start method is chosen.
//!
//! The stages are the wizard's existing phases shown differently: the
//! assignee stage is the Settings phase, the plan stage is the Runtime phase
//! and the confirm stage is the Confirm phase. Every other launch keeps the
//! settings form.

use super::*;

const STAGE_ASSIGNEE: &str = "assignee";
const STAGE_PLAN: &str = "plan";
const STAGE_CONFIRM: &str = "confirm";

fn reasoning_label(traits: &crate::agent_names::AgentTraits) -> Option<&'static str> {
    use crate::agent_names::TraitReasoningLevel as Level;
    traits.reasoning_level.map(|level| match level {
        Level::Low => "Low",
        Level::Standard => "Standard",
        Level::High => "High",
        Level::Max => "Max",
    })
}

fn speed_label(traits: &crate::agent_names::AgentTraits) -> Option<&'static str> {
    use crate::agent_names::TraitSpeed as Speed;
    traits.speed.map(|speed| match speed {
        Speed::Normal => "Normal",
        Speed::Fast => "Fast",
        Speed::Fastest => "Fastest",
    })
}

/// The first line of the personality text, which is what a row has room for.
fn personality_excerpt(traits: &crate::agent_names::AgentTraits) -> Option<String> {
    traits
        .personality
        .as_deref()
        .and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(str::to_string)
}

impl LaunchWizardState {
    /// Offer `candidates` as the assignee of this launch, which then plans
    /// the Issue's agent in three stages. A launch with no Issue, a mode that
    /// is not a new launch on a branch, or an empty roster keeps the settings
    /// form.
    pub fn enable_issue_plan(&mut self, candidates: Vec<LaunchWizardAssigneeCandidate>) {
        let applies = self.linked_issue_number.is_some()
            && matches!(
                self.wizard_mode,
                LaunchWizardMode::Branch | LaunchWizardMode::Knowledge
            )
            && !self.preserve_profile_launch_preferences;
        self.assignee_candidates = if applies { candidates } else { Vec::new() };
        // Roster order is the default (SPEC #7007 item 190).
        self.selected_assignee_id = self
            .assignee_candidates
            .iter()
            .find(|candidate| candidate.unavailable_reason.is_none())
            .map(|candidate| candidate.agent_member_id.clone());
    }

    pub(super) fn issue_plan_enabled(&self) -> bool {
        !self.assignee_candidates.is_empty()
    }

    /// Whether the three stages are what the wizard is showing: the user
    /// chose to configure a new agent launch.
    pub(super) fn issue_plan_active(&self) -> bool {
        self.issue_plan_enabled()
            && self.launch_target_is_agent()
            && self.launch_path == LaunchWizardLaunchPath::ManualSetup
            && self.manual_setup_initialized
            && self.start_method_selected
    }

    fn selected_assignee(&self) -> Option<&LaunchWizardAssigneeCandidate> {
        let id = self.selected_assignee_id.as_deref()?;
        self.assignee_candidates
            .iter()
            .find(|candidate| candidate.agent_member_id == id)
    }

    /// The registered agent this launch reserves. `None` for every launch
    /// that does not go through the three stages.
    pub fn issue_plan_assignee_id(&self) -> Option<&str> {
        self.issue_plan_active()
            .then_some(self.selected_assignee_id.as_deref())
            .flatten()
    }

    pub(super) fn issue_plan_blocks_submit(&self) -> bool {
        self.issue_plan_active() && self.selected_assignee().is_none()
    }

    pub(super) fn select_assignee(&mut self, agent_member_id: &str) {
        let Some(candidate) = self
            .assignee_candidates
            .iter()
            .find(|candidate| candidate.agent_member_id == agent_member_id)
        else {
            return;
        };
        if let Some(reason) = candidate.unavailable_reason.clone() {
            self.error = Some(reason);
            return;
        }
        self.selected_assignee_id = Some(agent_member_id.to_string());
        self.apply_assignee_profile();
    }

    /// Start the selected agent as its traits and the project's execution
    /// environment say. The plan offers nothing that overrides them
    /// (SPEC #7007 item 26).
    pub(super) fn apply_assignee_profile(&mut self) {
        let Some(profile) = self
            .selected_assignee()
            .and_then(|candidate| candidate.profile.clone())
        else {
            return;
        };
        if self.agent_id != profile.agent_id {
            self.set_agent_id(&profile.agent_id);
        }
        self.apply_previous_agent_preferences(profile);
    }

    pub(super) fn issue_plan_stage(&self) -> Option<&'static str> {
        if !self.issue_plan_active() {
            return None;
        }
        match self.current_phase() {
            WizardPhase::Confirm => Some(STAGE_CONFIRM),
            WizardPhase::Runtime => Some(STAGE_PLAN),
            WizardPhase::Settings if self.runtime_resolution_pending => Some(STAGE_PLAN),
            WizardPhase::Settings => Some(STAGE_ASSIGNEE),
            WizardPhase::Path => None,
        }
    }

    /// Whether the progress rail names the three stages: while they are
    /// shown, and on the start-method screen in front of them.
    pub(super) fn issue_plan_rail(&self) -> bool {
        self.issue_plan_active() || (self.issue_plan_enabled() && !self.start_method_selected)
    }

    pub(super) fn issue_plan_primary_action_label(&self) -> Option<&'static str> {
        self.issue_plan_stage().map(|stage| match stage {
            STAGE_ASSIGNEE => "Continue to launch plan",
            STAGE_PLAN => "Continue to confirm",
            _ => "Start",
        })
    }

    pub(super) fn issue_plan_progress_steps_view(&self) -> Vec<LaunchWizardProgressStepView> {
        let stage = self.issue_plan_stage();
        let assignee = match stage {
            Some(STAGE_ASSIGNEE) => "active",
            Some(_) => "done",
            None => "pending",
        };
        let plan = match stage {
            Some(STAGE_PLAN) => "active",
            Some(STAGE_CONFIRM) => "done",
            Some(_) if self.runtime_context_resolved => "done",
            _ => "pending",
        };
        let confirm = if stage == Some(STAGE_CONFIRM) || self.launch_materialization_pending {
            "active"
        } else {
            "pending"
        };
        let step = |key: &str, label: &str, state: &str, detail: Option<String>| {
            LaunchWizardProgressStepView {
                key: key.to_string(),
                label: label.to_string(),
                state: state.to_string(),
                detail,
            }
        };
        vec![
            step(STAGE_ASSIGNEE, "Issue & assignee", assignee, None),
            step(
                STAGE_PLAN,
                "Launch plan",
                plan,
                self.runtime_resolution_message.clone(),
            ),
            step(
                STAGE_CONFIRM,
                "Confirm",
                confirm,
                self.launch_materialization_message.clone(),
            ),
        ]
    }

    pub(super) fn issue_plan_view(&self) -> Option<LaunchWizardIssuePlanView> {
        if !self.issue_plan_enabled() {
            return None;
        }
        let selected = self.selected_assignee_id.as_deref();
        Some(LaunchWizardIssuePlanView {
            stage: self.issue_plan_stage().map(str::to_string),
            issue_number: self.linked_issue_number?,
            assignees: self
                .assignee_candidates
                .iter()
                .map(|candidate| {
                    let is_selected = selected == Some(candidate.agent_member_id.as_str());
                    LaunchWizardAssigneeView {
                        agent_member_id: candidate.agent_member_id.clone(),
                        name: candidate.name.clone(),
                        selected: is_selected,
                        available: candidate.unavailable_reason.is_none(),
                        status: candidate.unavailable_reason.clone().unwrap_or_else(|| {
                            if is_selected {
                                "Assigned to this launch".to_string()
                            } else {
                                "Can take this launch".to_string()
                            }
                        }),
                        reasoning: reasoning_label(&candidate.traits)
                            .unwrap_or("Tool default")
                            .to_string(),
                        speed: speed_label(&candidate.traits)
                            .unwrap_or("Tool default")
                            .to_string(),
                        personality: personality_excerpt(&candidate.traits),
                    }
                })
                .collect(),
            branch: self.branch_name.clone(),
            branch_detail: if self.is_new_branch {
                format!(
                    "Created from {}",
                    self.base_branch_name
                        .as_deref()
                        .unwrap_or(&self.context.selected_branch.name)
                )
            } else {
                "Existing branch".to_string()
            },
            permissions: if self.effective_skip_permissions() {
                "Skips permission prompts".to_string()
            } else {
                "Asks before risky actions".to_string()
            },
        })
    }

    /// The Issue and its assignee, which lead the confirm stage's summary.
    pub(super) fn issue_plan_summary_view(&self) -> Vec<LaunchWizardSummaryView> {
        if !self.issue_plan_active() {
            return Vec::new();
        }
        let mut summary = Vec::new();
        if let Some(number) = self.linked_issue_number {
            summary.push(LaunchWizardSummaryView {
                label: "Issue".to_string(),
                value: format!("#{number}"),
            });
        }
        if let Some(candidate) = self.selected_assignee() {
            let traits = &candidate.traits;
            let parts = [
                Some(candidate.name.clone()),
                reasoning_label(traits).map(|label| format!("Reasoning {label}")),
                speed_label(traits).map(|label| format!("Speed {label}")),
                personality_excerpt(traits),
            ];
            summary.push(LaunchWizardSummaryView {
                label: "Assignee".to_string(),
                value: parts.into_iter().flatten().collect::<Vec<_>>().join(" · "),
            });
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::test_support::*;
    use super::*;
    use crate::agent_names::{AgentTraits, TraitReasoningLevel, TraitSpeed};

    fn profile(agent_id: &str, reasoning: Option<&str>) -> LaunchWizardPreviousProfile {
        LaunchWizardPreviousProfile {
            agent_id: agent_id.to_string(),
            model: None,
            reasoning: reasoning.map(str::to_string),
            session_mode: gwt_agent::SessionMode::Normal,
            skip_permissions: false,
            fast_mode: false,
            codex_speed: gwt_agent::CodexSpeed::Off,
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: Default::default(),
            windows_shell: None,
            hermes: Default::default(),
        }
    }

    fn candidates() -> Vec<LaunchWizardAssigneeCandidate> {
        vec![
            LaunchWizardAssigneeCandidate {
                agent_member_id: "busy".to_string(),
                name: "Busy Agent".to_string(),
                traits: AgentTraits::default(),
                unavailable_reason: Some("Working on #7007 now.".to_string()),
                profile: Some(profile("claude", None)),
            },
            LaunchWizardAssigneeCandidate {
                agent_member_id: "oda".to_string(),
                name: "Oda Yohei".to_string(),
                traits: AgentTraits {
                    reasoning_level: Some(TraitReasoningLevel::Max),
                    speed: Some(TraitSpeed::Fastest),
                    personality: Some("Focus on implementation.\nSecond line.".to_string()),
                },
                unavailable_reason: None,
                profile: Some(LaunchWizardPreviousProfile {
                    codex_speed: gwt_agent::CodexSpeed::Ultrafast,
                    ..profile("codex", Some("high"))
                }),
            },
            LaunchWizardAssigneeCandidate {
                agent_member_id: "ando".to_string(),
                name: "Ando Kenji".to_string(),
                traits: AgentTraits {
                    reasoning_level: Some(TraitReasoningLevel::Standard),
                    speed: None,
                    personality: None,
                },
                unavailable_reason: None,
                profile: Some(profile("claude", None)),
            },
        ]
    }

    fn issue_wizard() -> LaunchWizardState {
        let mut state = LaunchWizardState::open_knowledge_launch_with_previous_profiles(
            context_with_linked_issue(
                branch("develop"),
                "work/issue-5561",
                LinkedIssueKind::Issue,
                5561,
            ),
            "develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            Default::default(),
        );
        state.mark_runtime_context_unresolved();
        state.enable_issue_plan(candidates());
        state
    }

    fn hydration(docker: bool) -> LaunchWizardHydration {
        LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "work/issue-5561".to_string(),
            worktree_path: None,
            quick_start_root: PathBuf::from("/tmp/repo"),
            docker_context: docker.then(|| DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: if docker {
                gwt_docker::ComposeServiceStatus::Running
            } else {
                gwt_docker::ComposeServiceStatus::NotFound
            },
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: None,
            open_branch_candidates: Vec::new(),
        }
    }

    fn configure(state: &mut LaunchWizardState) {
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
    }

    /// Stage 1 → stage 2: the submit asks for the place to run, and the
    /// answer arrives without a worktree having been created.
    fn to_plan_stage(state: &mut LaunchWizardState, docker: bool) {
        configure(state);
        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion.take(),
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));
        state.apply_runtime_context(hydration(docker));
    }

    fn stage(state: &LaunchWizardState) -> Option<String> {
        state.view().issue_plan.and_then(|plan| plan.stage)
    }

    fn rail(state: &LaunchWizardState) -> Vec<(String, String, String)> {
        state
            .view()
            .progress_steps
            .into_iter()
            .map(|step| (step.key, step.label, step.state))
            .collect()
    }

    #[test]
    fn an_issue_launch_completes_through_the_three_stages() {
        let mut state = issue_wizard();
        // The start-method screen stays in front of the three stages.
        let view = state.view();
        assert!(view.show_start_methods);
        assert_eq!(stage(&state), None);
        assert_eq!(
            rail(&state),
            vec![
                (
                    "assignee".into(),
                    "Issue & assignee".into(),
                    "pending".into()
                ),
                ("plan".into(), "Launch plan".into(), "pending".into()),
                ("confirm".into(), "Confirm".into(), "pending".into()),
            ]
        );

        configure(&mut state);
        assert_eq!(stage(&state).as_deref(), Some("assignee"));
        let view = state.view();
        assert_eq!(view.primary_action_label, "Continue to launch plan");
        assert!(view.primary_action_enabled);
        // The settings form is replaced, not shown next to the stage.
        assert!(!view.show_agent_settings);
        assert!(!view.show_reasoning);
        assert_eq!(rail(&state)[0].2, "active");

        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion.take(),
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));
        state.apply_runtime_context(hydration(false));
        assert_eq!(stage(&state).as_deref(), Some("plan"));
        let view = state.view();
        assert_eq!(view.primary_action_label, "Continue to confirm");
        assert_eq!(
            rail(&state)
                .into_iter()
                .map(|step| step.2)
                .collect::<Vec<_>>(),
            vec!["done", "active", "pending"]
        );
        let plan = view.issue_plan.expect("issue plan");
        assert_eq!(plan.issue_number, 5561);
        assert_eq!(plan.branch, "work/issue-5561");
        assert_eq!(plan.branch_detail, "Created from develop");

        state.apply(LaunchWizardAction::Submit);
        assert!(state.completion.is_none(), "the plan stage does not launch");
        assert_eq!(stage(&state).as_deref(), Some("confirm"));
        let view = state.view();
        assert!(view.show_confirm);
        assert_eq!(view.primary_action_label, "Start");
        assert_eq!(
            rail(&state)
                .into_iter()
                .map(|step| step.2)
                .collect::<Vec<_>>(),
            vec!["done", "done", "active"]
        );
        let labels: Vec<_> = view
            .launch_summary
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(labels[..2], ["Issue", "Assignee"]);
        assert_eq!(view.launch_summary[0].value, "#5561");
        assert_eq!(
            view.launch_summary[1].value,
            "Oda Yohei · Reasoning Max · Speed Fastest · Focus on implementation."
        );

        state.apply(LaunchWizardAction::Submit);
        let Some(LaunchWizardCompletion::Launch(request)) = state.completion.take() else {
            panic!("the confirm stage launches");
        };
        let LaunchWizardLaunchRequest::Agent(config) = *request else {
            panic!("an Issue plan launches an agent");
        };
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
        assert_eq!(config.linked_issue_number, Some(5561));
        assert_eq!(config.branch.as_deref(), Some("work/issue-5561"));
        assert_eq!(state.issue_plan_assignee_id(), Some("oda"));
    }

    #[test]
    fn stage_one_offers_registered_agents_with_their_traits() {
        let mut state = issue_wizard();
        configure(&mut state);
        let plan = state.view().issue_plan.expect("issue plan");
        // Roster order is the default: the first agent that can take the
        // launch is chosen, and a busy one ahead of it is not.
        assert_eq!(
            plan.assignees
                .iter()
                .map(|agent| (agent.name.as_str(), agent.selected, agent.available))
                .collect::<Vec<_>>(),
            vec![
                ("Busy Agent", false, false),
                ("Oda Yohei", true, true),
                ("Ando Kenji", false, true),
            ]
        );
        assert_eq!(plan.assignees[0].status, "Working on #7007 now.");
        let oda = &plan.assignees[1];
        assert_eq!(oda.reasoning, "Max");
        assert_eq!(oda.speed, "Fastest");
        assert_eq!(oda.personality.as_deref(), Some("Focus on implementation."));
        let ando = &plan.assignees[2];
        assert_eq!(ando.reasoning, "Standard");
        assert_eq!(ando.speed, "Tool default");
        assert_eq!(ando.personality, None);
        // The agent's traits on the project's environment decide the tool
        // and its tuning; the plan has nothing that overrides them.
        assert_eq!(state.agent_id, "codex");
        assert_eq!(state.reasoning, "high");
        assert_eq!(state.codex_speed, gwt_agent::CodexSpeed::Ultrafast);

        state.apply(LaunchWizardAction::SetAgentMember {
            agent_member_id: "ando".to_string(),
        });
        assert_eq!(state.issue_plan_assignee_id(), Some("ando"));
        assert_eq!(state.agent_id, "claude");
        assert_eq!(state.codex_speed, gwt_agent::CodexSpeed::Off);

        state.apply(LaunchWizardAction::SetAgentMember {
            agent_member_id: "busy".to_string(),
        });
        assert_eq!(state.issue_plan_assignee_id(), Some("ando"));
        assert_eq!(state.error.as_deref(), Some("Working on #7007 now."));
    }

    #[test]
    fn stage_one_cannot_continue_without_an_agent_that_can_take_the_launch() {
        let mut state = issue_wizard();
        state.enable_issue_plan(candidates().into_iter().take(1).collect());
        configure(&mut state);
        assert!(!state.view().primary_action_enabled);
        state.apply(LaunchWizardAction::Submit);
        assert!(state.completion.is_none());
        assert_eq!(
            state.error.as_deref(),
            Some("Choose a registered agent that can take this launch")
        );
    }

    #[test]
    fn the_place_to_run_is_chosen_in_the_plan_stage() {
        let mut state = issue_wizard();
        to_plan_stage(&mut state, true);
        let view = state.view();
        assert_eq!(stage(&state).as_deref(), Some("plan"));
        assert!(view.show_runtime_confirmation);
        assert!(view.show_runtime_target, "Docker is offered when detected");
        assert!(view.runtime_target_note.is_some());

        let mut host_only = issue_wizard();
        to_plan_stage(&mut host_only, false);
        assert!(!host_only.view().show_runtime_target);
    }

    #[test]
    fn the_stages_are_walked_back_and_forth_without_asking_again() {
        let mut state = issue_wizard();
        to_plan_stage(&mut state, false);
        state.apply(LaunchWizardAction::Back);
        assert_eq!(stage(&state).as_deref(), Some("assignee"));
        // Changing the assignee does not change where the session runs.
        state.apply(LaunchWizardAction::SetAgentMember {
            agent_member_id: "ando".to_string(),
        });
        state.apply(LaunchWizardAction::Submit);
        assert!(state.completion.is_none());
        assert_eq!(stage(&state).as_deref(), Some("plan"));

        state.apply(LaunchWizardAction::Submit);
        assert_eq!(stage(&state).as_deref(), Some("confirm"));
        state.apply(LaunchWizardAction::GotoStep {
            phase: WizardPhase::Settings,
        });
        assert_eq!(stage(&state).as_deref(), Some("assignee"));
        state.apply(LaunchWizardAction::Back);
        assert!(state.view().show_start_methods);
        assert_eq!(stage(&state), None);
    }

    #[test]
    fn a_branch_changed_in_the_plan_stage_asks_for_the_place_to_run_again() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(
                branch("feature/current"),
                "feature/current",
                LinkedIssueKind::Issue,
                5561,
            ),
            sample_agent_options(),
            Vec::new(),
        );
        state.mark_runtime_context_unresolved();
        state.enable_issue_plan(candidates());
        configure(&mut state);
        assert!(!state.view().show_branch_controls);
        state.apply(LaunchWizardAction::Submit);
        state.completion = None;
        state.apply_runtime_context(LaunchWizardHydration {
            normalized_branch_name: "feature/current".to_string(),
            ..hydration(false)
        });
        assert!(state.view().show_branch_controls);

        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchName {
            value: "feature/next".to_string(),
        });
        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion.take(),
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));
        state.apply_runtime_context(LaunchWizardHydration {
            normalized_branch_name: "feature/next".to_string(),
            ..hydration(false)
        });
        assert_eq!(stage(&state).as_deref(), Some("plan"));
        state.apply(LaunchWizardAction::Submit);
        assert_eq!(stage(&state).as_deref(), Some("confirm"));
    }

    #[test]
    fn the_other_start_methods_keep_working_from_the_start_method_screen() {
        let state = issue_wizard();
        let kinds: Vec<_> = state
            .view()
            .start_methods
            .into_iter()
            .map(|method| method.kind)
            .collect();
        for kind in [
            "configure_and_start",
            "start_with_last_settings",
            "continue_last_session",
            "focus_running_session",
            "open_shell",
        ] {
            assert!(kinds.contains(&kind.to_string()), "{kind} is offered");
        }

        // A shell goes through the settings form, as before.
        let mut shell = issue_wizard();
        shell.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::OpenShell,
        });
        let view = shell.view();
        assert_eq!(view.selected_launch_target, "shell");
        assert!(view.show_manual_setup);
        assert_eq!(view.issue_plan.and_then(|plan| plan.stage), None);
        assert_eq!(
            rail(&shell)
                .into_iter()
                .map(|step| step.0)
                .collect::<Vec<_>>(),
            vec!["path", "setup", "runtime", "start"]
        );
        shell.apply(LaunchWizardAction::Submit);
        let Some(LaunchWizardCompletion::ResolveRuntime(request)) = shell.completion.take() else {
            panic!("a shell asks for the place to run");
        };
        assert!(matches!(*request, LaunchWizardLaunchRequest::Shell(_)));
    }

    #[test]
    fn a_launch_without_an_issue_or_a_roster_keeps_the_settings_form() {
        let mut no_issue = LaunchWizardState::open_with(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
        );
        no_issue.enable_issue_plan(candidates());
        let mut no_roster = issue_wizard();
        no_roster.enable_issue_plan(Vec::new());
        let mut start_work = LaunchWizardState::open_start_work_with_previous_profile(
            context(branch("develop"), "develop"),
            "develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            None,
        );
        start_work.enable_issue_plan(candidates());

        for mut state in [no_issue, no_roster, start_work] {
            state.mark_runtime_context_unresolved();
            assert!(state
                .view()
                .start_methods
                .iter()
                .all(|method| method.kind != "open_shell"));
            configure(&mut state);
            let view = state.view();
            assert!(view.issue_plan.is_none());
            assert!(view.show_agent_settings);
            assert_eq!(view.progress_steps.len(), 4);
            assert_eq!(state.issue_plan_assignee_id(), None);
        }
    }
}
