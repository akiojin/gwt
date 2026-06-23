use super::*;

impl LaunchWizardState {
    pub fn build_launch_config(&self) -> Result<gwt_agent::LaunchConfig, String> {
        if self.is_hydrating {
            return Err("Launch options are still loading".to_string());
        }
        if !self.launch_target_is_agent() {
            return Err("Agent launch target is not selected".to_string());
        }
        let selected_agent = self
            .selected_agent()
            .cloned()
            .ok_or_else(|| "Agent option is unavailable".to_string())?;

        // SPEC-1921 FR-090 (2026-05-18 amendment) / T295: when a saved
        // Quick Start entry recorded `AgentId::Custom("<old-id>")` for a
        // legacy `ClaudeCodeOpenaiCompat` preset that has since been
        // migrated to `[builtinAgents.claudeCode.backends.<old-id>]`, the
        // wizard MUST relaunch through the built-in Claude Code path with
        // the matching backend profile attached. The remap is transparent
        // to the caller; no UI prompt is shown.
        let raw_agent_id = agent_id_from_key(&selected_agent.id);
        let config_path = gwt_core::paths::gwt_config_path();
        let remap_backend_id = if let gwt_agent::AgentId::Custom(_) = &raw_agent_id {
            gwt_agent::resolve_legacy_backend_remap(&raw_agent_id, &config_path)
        } else {
            None
        };
        let (agent_id, backend_profile) = if let Some(backend_id) = remap_backend_id {
            let profile = gwt_agent::load_backends_for_agent(
                &config_path,
                gwt_agent::BuiltinAgentId::ClaudeCode,
            )
            .ok()
            .and_then(|profiles| profiles.into_iter().find(|p| p.id == backend_id));
            match profile {
                Some(profile) => (gwt_agent::AgentId::ClaudeCode, Some(profile)),
                None => (raw_agent_id, None),
            }
        } else {
            (raw_agent_id, None)
        };

        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id.clone());
        // FR-090: drop the legacy `selected_agent.custom_agent` when remap
        // succeeded — the launch is now a built-in Claude Code with backend
        // profile, not a Custom Coding Agent.
        match (&backend_profile, selected_agent.custom_agent) {
            (Some(profile), _) => {
                builder = builder.backend_profile(profile.clone());
            }
            (None, Some(custom_agent)) => {
                builder = builder.custom_agent(custom_agent);
            }
            (None, None) => {}
        }

        if !self.is_new_branch {
            if let Some(worktree_path) = &self.context.worktree_path {
                builder = builder.working_dir(worktree_path.clone());
            }
        }

        if !self.branch_name.is_empty() {
            builder = builder.branch(self.branch_name.clone());
        }

        if self.is_new_branch {
            builder = builder.base_branch(
                self.base_branch_name
                    .clone()
                    .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string()),
            );
        }

        if is_explicit_model_selection(&self.model) {
            builder = builder.model(self.model.clone());
        }

        if !self.version.is_empty() {
            builder = builder.version(self.version.clone());
        }

        if let Some(reasoning_level) = self.reasoning_level_for_launch() {
            builder = builder.reasoning_level(reasoning_level.to_string());
        }

        if self.skip_permissions {
            builder = builder.skip_permissions(true);
        }

        if self.fast_mode_enabled_for_current_agent() {
            builder = builder.fast_mode(true);
        }

        // SPEC-3152: Hermes-specific launch options. Free-text/CSV fields map
        // to their CLI flags; safe-mode is a boolean toggle. Only applied for
        // the Hermes agent.
        if agent_id == gwt_agent::AgentId::Hermes {
            let trimmed = |value: &str| {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_string())
            };
            if let Some(provider) = trimmed(&self.hermes_provider) {
                builder = builder.provider(provider);
            }
            if let Some(profile) = trimmed(&self.hermes_profile) {
                builder = builder.profile(profile);
            }
            if let Some(toolsets) = trimmed(&self.hermes_toolsets) {
                builder = builder.toolsets(toolsets);
            }
            if let Some(skills) = trimmed(&self.hermes_skills) {
                builder = builder.skills(skills);
            }
            if let Some(max_turns) =
                trimmed(&self.hermes_max_turns).and_then(|value| value.parse::<u32>().ok())
            {
                if max_turns > 0 {
                    builder = builder.max_turns(max_turns);
                }
            }
            if self.hermes_safe_mode {
                builder = builder.safe_mode(true);
            }
        }

        builder = builder.runtime_target(self.runtime_target);
        if let Some(windows_shell) = self.windows_shell_for_launch() {
            builder = builder.windows_shell(windows_shell);
        }
        if let Some(docker_service) = self.docker_service.as_deref() {
            builder = builder.docker_service(docker_service.to_string());
        }
        builder = builder.docker_lifecycle_intent(self.docker_lifecycle_intent);
        // SPEC-2014 2026-05-18 amendment FR-A:
        // Execution Mode `"resume"` always maps to `SessionMode::Resume`.
        // - Quick Start Resume (with id)       → SessionMode::Resume + id
        // - Execution Mode Resume (no id)      → SessionMode::Resume (agent picker)
        // The earlier silent downgrade to Continue when id was absent has been
        // removed; UI option filtering and `normalize_execution_mode` already
        // prevent this state for picker-unsupported agents.
        builder = match self.mode.as_str() {
            "continue" => builder.session_mode(gwt_agent::SessionMode::Continue),
            "resume" => {
                let mut b = builder.session_mode(gwt_agent::SessionMode::Resume);
                if let Some(id) = self.resume_session_id.clone() {
                    b = b.resume_session_id(id);
                }
                b
            }
            _ => builder.session_mode(gwt_agent::SessionMode::Normal),
        };

        if let Some(n) = self.linked_issue_number {
            builder = builder.linked_issue_number(n);
        }

        let mut config = builder.build();
        if !self.version.is_empty() {
            config.tool_version = Some(self.version.clone());
        }
        if let Some(reasoning_level) = self.reasoning_level_for_launch() {
            config.reasoning_level = Some(reasoning_level.to_string());
        }
        Ok(config)
    }

    fn build_shell_launch_config(&self) -> Result<ShellLaunchConfig, String> {
        if self.is_hydrating {
            return Err("Launch options are still loading".to_string());
        }

        let working_dir = if self.is_new_branch {
            None
        } else {
            self.context.worktree_path.clone()
        };
        let branch = (!self.branch_name.is_empty()).then(|| self.branch_name.clone());
        let base_branch = self.is_new_branch.then(|| {
            self.base_branch_name
                .clone()
                .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string())
        });
        let mut env_vars = HashMap::new();
        if let Some(dir) = working_dir.as_ref() {
            env_vars.insert("GWT_PROJECT_ROOT".to_string(), dir.display().to_string());
        }

        Ok(ShellLaunchConfig {
            working_dir,
            branch,
            base_branch,
            display_name: "Shell".to_string(),
            runtime_target: self.runtime_target,
            docker_service: self.docker_service.clone(),
            docker_lifecycle_intent: self.docker_lifecycle_intent,
            windows_shell: self.windows_shell_for_launch(),
            env_vars,
            remove_env: Vec::new(),
            command_override: None,
            command_args_override: None,
        })
    }

    pub(super) fn build_launch_request(&self) -> Result<LaunchWizardLaunchRequest, String> {
        match self.launch_target {
            LaunchTargetKind::Agent => Ok(LaunchWizardLaunchRequest::Agent(Box::new(
                self.build_launch_config()?,
            ))),
            LaunchTargetKind::Shell => Ok(LaunchWizardLaunchRequest::Shell(Box::new(
                self.build_shell_launch_config()?,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::super::test_support::*;
    use super::*;

    #[test]
    fn build_launch_config_for_codex_resume_uses_resume_session_id() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.agent_id = "codex".to_string();
        state.model = "gpt-5.5".to_string();
        state.reasoning = "high".to_string();
        state.version = "0.110.0".to_string();
        state.mode = "resume".to_string();
        state.resume_session_id = Some("session-123".to_string());
        state.skip_permissions = true;
        state.codex_fast_mode = true;
        state.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        state.docker_service = Some("gwt".to_string());
        state.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
        assert_eq!(config.branch.as_deref(), Some("feature/gui"));
        assert_eq!(config.resume_session_id.as_deref(), Some("session-123"));
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        assert_eq!(config.reasoning_level.as_deref(), Some("high"));
        assert_eq!(config.tool_version.as_deref(), Some("0.110.0"));
        assert_eq!(config.docker_service.as_deref(), Some("gwt"));
        assert!(config.skip_permissions);
        assert!(config.codex_fast_mode);
    }

    // SPEC-2014 2026-05-18 amendment FR-A / SC-A / SC-B:
    // Execution Mode `Resume` without a resume_session_id (i.e. the picker
    // path) must reach the agent CLI as SessionMode::Resume without an id.
    // The earlier silent downgrade to SessionMode::Continue is removed.
    #[test]
    fn build_launch_config_resume_without_id_keeps_session_mode_resume_for_codex() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.agent_id = "codex".to_string();
        state.mode = "resume".to_string();
        state.resume_session_id = None;

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        assert!(config.resume_session_id.is_none());
        // Codex builder must produce `codex resume` (picker) — no `--last`.
        assert!(!config.args.contains(&"--last".to_string()));
        assert!(config.args.iter().any(|arg| arg == "resume"));
    }

    #[test]
    fn build_launch_config_resume_without_id_keeps_session_mode_resume_for_claude() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.agent_id = "claude".to_string();
        state.mode = "resume".to_string();
        state.resume_session_id = None;

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        assert!(config.resume_session_id.is_none());
        // Claude builder pushes `--resume` (no id) which opens its picker.
        assert!(config.args.contains(&"--resume".to_string()));
        assert!(!config.args.iter().any(|arg| arg == "--continue"));
    }

    #[test]
    fn claude_fast_mode_is_exposed_and_applied_to_launch_config() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        // SPEC-2014 FR-126: Settings フォーム項目は ConfigureAndStart の Settings
        // ステップ（未解決）で表示される。
        state.mark_runtime_context_unresolved();
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        state.apply(LaunchWizardAction::SetFastMode { enabled: true });

        let view = state.view();
        assert_eq!(view.selected_agent_id, "claude");
        assert!(view.show_fast_mode);
        assert!(view.fast_mode);
        assert!(!view.show_codex_fast_mode);
        assert!(!view.codex_fast_mode);
        assert!(view
            .launch_summary
            .iter()
            .any(|item| item.label == "Fast mode" && item.value == "on"));

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::ClaudeCode);
        // SPEC-2014 FR-106: host launches deliver fastMode via a materialized
        // settings file path instead of inline JSON.
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--settings" && pair[1].ends_with("claude-settings-fast.json")));
    }

    #[test]
    fn hidden_codex_fast_mode_draft_does_not_affect_claude_launch() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });
        state.apply(LaunchWizardAction::SetCodexFastMode { enabled: true });
        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        state.apply(LaunchWizardAction::SetSkipPermissions { enabled: false });

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::ClaudeCode);
        assert!(!config.codex_fast_mode);
        assert!(!config.skip_permissions);
    }

    #[test]
    fn shell_target_hides_agent_specific_controls_and_builds_shell_request() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.worktree_path = Some(PathBuf::from("/tmp/repo-feature"));
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.apply(LaunchWizardAction::SetLaunchTarget {
            target: LaunchTargetKind::Shell,
        });

        let view = state.view();
        assert_eq!(view.selected_launch_target, "shell");
        assert!(!view.show_agent_settings);
        assert!(!view.show_execution_mode);
        assert!(!view.show_skip_permissions);
        assert!(!view.show_version);
        assert!(view
            .launch_summary
            .iter()
            .any(|item| item.label == "Target" && item.value == "Shell"));
        assert!(!view.launch_summary.iter().any(|item| item.label == "Agent"));

        match state.build_launch_request().expect("shell launch request") {
            LaunchWizardLaunchRequest::Shell(config) => {
                assert_eq!(
                    config.working_dir.as_deref(),
                    Some(Path::new("/tmp/repo-feature"))
                );
                assert_eq!(config.branch.as_deref(), Some("feature/gui"));
                assert_eq!(config.display_name, "Shell");
                assert_eq!(config.runtime_target, gwt_agent::LaunchRuntimeTarget::Host);
            }
            other => panic!("expected shell launch request, got {other:?}"),
        }
    }

    #[test]
    fn windows_shell_selection_flows_to_agent_and_shell_launch_requests() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.worktree_path = Some(PathBuf::from("/tmp/repo-feature"));
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.apply(LaunchWizardAction::SetWindowsShell {
            shell: gwt_agent::WindowsShellKind::PowerShell7,
        });

        let view = state.view();
        assert_eq!(view.windows_shell_options.len(), 3);
        assert!(view
            .windows_shell_options
            .iter()
            .any(|option| option.label == "PowerShell 7"));
        if cfg!(windows) {
            assert_eq!(
                view.selected_windows_shell.as_deref(),
                Some("power_shell_7")
            );
            assert!(view
                .launch_summary
                .iter()
                .any(|item| item.label == "Shell" && item.value == "PowerShell 7"));
        } else {
            assert_eq!(view.selected_windows_shell.as_deref(), None);
        }

        let config = state.build_launch_config().expect("agent config");
        if cfg!(windows) {
            assert_eq!(
                config.windows_shell,
                Some(gwt_agent::WindowsShellKind::PowerShell7)
            );
        } else {
            assert_eq!(config.windows_shell, None);
        }

        state.apply(LaunchWizardAction::SetLaunchTarget {
            target: LaunchTargetKind::Shell,
        });

        match state.build_launch_request().expect("shell request") {
            LaunchWizardLaunchRequest::Shell(config) => {
                if cfg!(windows) {
                    assert_eq!(
                        config.windows_shell,
                        Some(gwt_agent::WindowsShellKind::PowerShell7)
                    );
                } else {
                    assert_eq!(config.windows_shell, None);
                }
            }
            other => panic!("expected shell launch request, got {other:?}"),
        }
    }

    #[test]
    fn docker_runtime_omits_windows_shell_from_launch_requests() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string()],
            suggested_service: Some("api".to_string()),
        });
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.apply(LaunchWizardAction::SetWindowsShell {
            shell: gwt_agent::WindowsShellKind::CommandPrompt,
        });
        state.apply(LaunchWizardAction::SetRuntimeTarget {
            target: gwt_agent::LaunchRuntimeTarget::Docker,
        });

        let config = state.build_launch_config().expect("agent config");
        assert_eq!(
            config.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(config.windows_shell, None);

        state.apply(LaunchWizardAction::SetLaunchTarget {
            target: LaunchTargetKind::Shell,
        });
        match state.build_launch_request().expect("shell request") {
            LaunchWizardLaunchRequest::Shell(config) => {
                assert_eq!(
                    config.runtime_target,
                    gwt_agent::LaunchRuntimeTarget::Docker
                );
                assert_eq!(config.windows_shell, None);
            }
            other => panic!("expected shell launch request, got {other:?}"),
        }
    }

    #[test]
    fn build_launch_config_maps_hermes_launch_options() {
        let mut options = sample_agent_options();
        options.push(AgentOption {
            id: "hermes".to_string(),
            name: "Hermes Agent".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            options,
            Vec::new(),
        );
        state.set_agent_id("hermes");
        state.apply(LaunchWizardAction::SetModel {
            model: "anthropic/claude-sonnet-4".to_string(),
        });
        state.apply(LaunchWizardAction::SetHermesOption {
            field: "provider".to_string(),
            value: "openrouter".to_string(),
        });
        state.apply(LaunchWizardAction::SetHermesOption {
            field: "profile".to_string(),
            value: "work".to_string(),
        });
        state.apply(LaunchWizardAction::SetHermesOption {
            field: "toolsets".to_string(),
            value: "fs,web".to_string(),
        });
        state.apply(LaunchWizardAction::SetHermesOption {
            field: "skills".to_string(),
            value: "gwt-build-spec".to_string(),
        });
        state.apply(LaunchWizardAction::SetHermesOption {
            field: "max_turns".to_string(),
            value: "40".to_string(),
        });
        state.apply(LaunchWizardAction::SetHermesSafeMode { enabled: true });

        let config = state.build_launch_config().expect("hermes launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Hermes);
        let has_pair =
            |flag: &str, val: &str| config.args.windows(2).any(|p| p[0] == flag && p[1] == val);
        assert!(has_pair("--provider", "openrouter"));
        assert!(has_pair("--model", "anthropic/claude-sonnet-4"));
        assert!(has_pair("--profile", "work"));
        assert!(has_pair("--toolsets", "fs,web"));
        assert!(has_pair("--skills", "gwt-build-spec"));
        assert!(has_pair("--max-turns", "40"));
        assert!(config.args.contains(&"--safe-mode".to_string()));
    }

    #[test]
    fn hermes_options_are_exposed_in_settings_view_for_hermes_only() {
        let mut options = sample_agent_options();
        options.push(AgentOption {
            id: "hermes".to_string(),
            name: "Hermes Agent".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            options,
            Vec::new(),
        );
        state.mark_runtime_context_unresolved();
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "hermes".to_string(),
        });
        state.set_hermes_provider_choices(vec!["zai".to_string(), "ollama-launch".to_string()]);
        state.apply(LaunchWizardAction::SetHermesOption {
            field: "provider".to_string(),
            value: "openrouter".to_string(),
        });

        let view = state.view();
        assert!(
            view.show_hermes_options,
            "Hermes settings section must be shown for the Hermes agent"
        );
        assert_eq!(view.hermes_provider, "openrouter");
        // Provider options come from the user's config, not a hardcoded list.
        assert_eq!(view.hermes_provider_options, vec!["zai", "ollama-launch"]);

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        assert!(
            !state.view().show_hermes_options,
            "non-Hermes agents must not show the Hermes section"
        );
    }

    #[test]
    fn hermes_needs_setup_flag_is_exposed_only_for_hermes() {
        let mut options = sample_agent_options();
        options.push(AgentOption {
            id: "hermes".to_string(),
            name: "Hermes Agent".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            options,
            Vec::new(),
        );
        state.mark_runtime_context_unresolved();
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.set_hermes_needs_setup(true);

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "hermes".to_string(),
        });
        assert!(state.view().hermes_needs_setup);

        // Non-Hermes agents never surface the needs-setup hint.
        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        assert!(!state.view().hermes_needs_setup);
    }

    #[test]
    fn opencode_options_expose_freetext_model_in_settings_view_for_opencode_only() {
        // SPEC-3151 FR-008: OpenCode shows a free-text provider/model field
        // (no fixed gwt model list) only when the OpenCode agent is selected.
        let mut options = sample_agent_options();
        options.push(AgentOption {
            id: "opencode".to_string(),
            name: "OpenCode".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            options,
            Vec::new(),
        );
        state.mark_runtime_context_unresolved();
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "opencode".to_string(),
        });
        // Free-text model is accepted without a fixed list (provider/model).
        state.apply(LaunchWizardAction::SetModel {
            model: "anthropic/claude-sonnet-4".to_string(),
        });

        let view = state.view();
        assert!(
            view.show_opencode_options,
            "OpenCode settings section must be shown for the OpenCode agent"
        );
        assert_eq!(view.selected_model, "anthropic/claude-sonnet-4");
        // OpenCode is not a Hermes-style provider/profile agent.
        assert!(!view.show_hermes_options);

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        assert!(
            !state.view().show_opencode_options,
            "non-OpenCode agents must not show the OpenCode section"
        );
    }

    #[test]
    fn build_launch_config_maps_opencode_freetext_model() {
        // SPEC-3151 FR-008: the free-text model reaches the CLI as `--model`.
        let mut options = sample_agent_options();
        options.push(AgentOption {
            id: "opencode".to_string(),
            name: "OpenCode".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            options,
            Vec::new(),
        );
        state.set_agent_id("opencode");
        state.apply(LaunchWizardAction::SetModel {
            model: "anthropic/claude-sonnet-4".to_string(),
        });

        let config = state.build_launch_config().expect("opencode launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::OpenCode);
        assert!(config
            .args
            .windows(2)
            .any(|p| p[0] == "--model" && p[1] == "anthropic/claude-sonnet-4"));
    }

    #[test]
    fn opencode_needs_setup_flag_is_exposed_only_for_opencode() {
        // SPEC-3151 FR-009: the non-blocking needs-setup hint only surfaces for
        // the OpenCode agent.
        let mut options = sample_agent_options();
        options.push(AgentOption {
            id: "opencode".to_string(),
            name: "OpenCode".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            options,
            Vec::new(),
        );
        state.mark_runtime_context_unresolved();
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.set_opencode_needs_setup(true);

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "opencode".to_string(),
        });
        assert!(state.view().opencode_needs_setup);

        // Non-OpenCode agents never surface the needs-setup hint.
        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        assert!(!state.view().opencode_needs_setup);
    }

    #[test]
    fn run_opencode_setup_yields_shell_completion_with_auth_login_command() {
        // SPEC-3151 FR-010: the in-pane setup launcher produces a Host shell
        // launch running `<opencode runner> auth login`.
        let mut options = sample_agent_options();
        options.push(AgentOption {
            id: "opencode".to_string(),
            name: "OpenCode".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.worktree_path = Some(PathBuf::from("/tmp/repo-feature"));
        let mut state = LaunchWizardState::open_with(ctx, options, Vec::new());
        state.set_agent_id("opencode");
        state.version = "latest".to_string();

        state.apply(LaunchWizardAction::RunOpenCodeSetup);

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(request)) => match request.as_ref() {
                LaunchWizardLaunchRequest::Shell(config) => {
                    assert_eq!(config.display_name, "OpenCode Setup");
                    assert_eq!(config.runtime_target, gwt_agent::LaunchRuntimeTarget::Host);
                    assert_eq!(
                        config.working_dir.as_deref(),
                        Some(Path::new("/tmp/repo-feature"))
                    );
                    let runner =
                        gwt_agent::launch::resolve_runner(&gwt_agent::AgentId::OpenCode, "latest");
                    assert_eq!(
                        config.command_override.as_deref(),
                        Some(runner.executable.as_str())
                    );
                    let args = config
                        .command_args_override
                        .as_ref()
                        .expect("command args override");
                    assert_eq!(&args[args.len() - 2..], &["auth", "login"]);
                    for base_arg in &runner.base_args {
                        assert!(
                            args.contains(base_arg),
                            "expected base arg {base_arg} in {args:?}"
                        );
                    }
                }
                other => panic!("expected shell launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn run_opencode_setup_action_deserializes_from_frontend_wire_tag() {
        // SPEC-3151 FR-010: the frontend dispatches `{"kind":"run_opencode_setup"}`.
        // Default snake_case would produce `run_open_code_setup`, so the variant
        // carries an explicit serde rename; this locks the wire contract.
        let action: LaunchWizardAction =
            serde_json::from_str(r#"{"kind":"run_opencode_setup"}"#).expect("deserialize action");
        assert_eq!(action, LaunchWizardAction::RunOpenCodeSetup);
    }

    #[test]
    fn build_launch_config_preserves_linked_issue_number() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.linked_issue_number = Some(1234);

        let state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        let config = state.build_launch_config().expect("config");

        assert_eq!(config.linked_issue_number, Some(1234));
    }

    #[test]
    fn build_launch_config_for_custom_agent_uses_stored_definition() {
        let dir = tempdir().expect("tempdir");
        let custom_path = dir.path().join("custom-agent");
        std::fs::write(&custom_path, "echo custom").expect("write custom agent stub");

        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            build_agent_options(
                Vec::new(),
                &gwt_agent::VersionCache::new(),
                vec![sample_custom_agent(
                    "proxy-agent",
                    "Claude Proxy",
                    gwt_agent::custom::CustomAgentType::Path,
                    custom_path.display().to_string(),
                )],
            ),
            Vec::new(),
        );
        state.set_agent_id("proxy-agent");
        state.set_execution_mode("resume");
        state.resume_session_id = Some("resume-1".to_string());
        state.skip_permissions = true;

        let config = state.build_launch_config().expect("custom launch config");

        assert_eq!(config.command, custom_path.display().to_string());
        assert_eq!(config.display_name, "Claude Proxy");
        assert!(config.args.contains(&"--serve".to_string()));
        assert!(config.args.contains(&"--resume".to_string()));
        assert!(config.args.contains(&"--unsafe".to_string()));
        assert_eq!(
            config.env_vars.get("API_KEY").map(String::as_str),
            Some("secret")
        );
    }

    #[test]
    fn build_launch_config_allows_configured_custom_agent_without_host_detection() {
        let missing_path = PathBuf::from("/tmp/nonexistent-custom-agent");
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            build_agent_options(
                Vec::new(),
                &gwt_agent::VersionCache::new(),
                vec![sample_custom_agent(
                    "missing-agent",
                    "Missing Agent",
                    gwt_agent::custom::CustomAgentType::Path,
                    missing_path.display().to_string(),
                )],
            ),
            Vec::new(),
        );
        state.set_agent_id("missing-agent");

        let config = state
            .build_launch_config()
            .expect("configured custom agent should reach runtime preparation");
        assert_eq!(config.command, missing_path.display().to_string());
        assert_eq!(config.display_name, "Missing Agent");
    }

    #[test]
    fn build_launch_config_rejects_loading_state() {
        let state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );

        let error = state
            .build_launch_config()
            .expect_err("loading must block launch");
        assert_eq!(error, "Launch options are still loading");
    }
}
