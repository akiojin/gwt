<script lang="ts">
  import type {
    AgentInfo,
    BranchInfo,
    BranchSuggestResult,
    DockerContext,
    LaunchAgentRequest,
  } from "../types";

  let {
    projectPath,
    selectedBranch = "",
    onLaunch,
    onClose,
  }: {
    projectPath: string;
    selectedBranch?: string;
    onLaunch: (request: LaunchAgentRequest) => Promise<void>;
    onClose: () => void;
  } = $props();

  type BranchMode = "existing" | "new";
  type SessionMode = "normal" | "continue" | "resume";
  type RuntimeTarget = "host" | "docker";

  type AgentVersionsInfo = {
    agentId: string;
    package: string;
    tags: string[];
    versions: string[];
    source: "cache" | "registry" | "fallback";
  };

  type SelectOption = { value: string; label: string };

  let agents: AgentInfo[] = $state([]);
  let selectedAgent: string = $state("");
  let branchMode: BranchMode = $state("existing" as BranchMode);
  let sessionMode: SessionMode = $state("normal" as SessionMode);

  let model: string = $state("");
  let agentVersion: string = $state("latest");
  let modelByAgent: Record<string, string> = $state({});
  let agentVersionByAgent: Record<string, string> = $state({});
  let lastAgent: string = $state("");

  let resumeSessionId: string = $state("");
  let skipPermissions: boolean = $state(false);
  let reasoningLevel: string = $state("");
  let collaborationModes: boolean = $state(false);

  let showAdvanced: boolean = $state(false);
  let extraArgsText: string = $state("");
  let envOverridesText: string = $state("");

  let dockerContext: DockerContext | null = $state(null as DockerContext | null);
  let dockerLoading: boolean = $state(false);
  let dockerError: string | null = $state(null);
  let dockerContextKey: string = $state("");

  let runtimeTarget: RuntimeTarget = $state("host" as RuntimeTarget);
  let dockerService: string = $state("");
  let dockerBuild: boolean = $state(false);
  let dockerRecreate: boolean = $state(false);
  let dockerKeep: boolean = $state(false);

  let versionsLoading: boolean = $state(false);
  let versionTags: string[] = $state([]);
  let versionOptions: string[] = $state([]);
  let versionsError: string | null = $state(null);

  // Capture the branch at open-time. "Existing Branch" is read-only.
  const existingBranch: string = (() => selectedBranch)();
  // "New Branch" fields are editable by the user.
  let baseBranch: string = $state(existingBranch);

  type BranchPrefix = "feature/" | "bugfix/" | "hotfix/" | "release/";
  const BRANCH_PREFIXES: BranchPrefix[] = ["feature/", "bugfix/", "hotfix/", "release/"];

  let newBranchPrefix: BranchPrefix = $state("feature/" as BranchPrefix);
  let newBranchSuffix: string = $state("");
  let newBranchFullName = $derived(buildNewBranchName(newBranchPrefix, newBranchSuffix));

  // Base Branch options (Worktree + Remote)
  let baseBranchLocalOptions: string[] = $state([]);
  let baseBranchRemoteOptions: string[] = $state([]);
  let baseBranchOptionsLoading: boolean = $state(false);
  let baseBranchOptionsError: string | null = $state(null);

  // AI Branch Suggest modal (TUI parity)
  let suggestOpen: boolean = $state(false);
  let suggestDescription: string = $state("");
  let suggestLoading: boolean = $state(false);
  let suggestError: string | null = $state(null);
  let suggestSuggestions: string[] = $state([]);

  let loading: boolean = $state(true);
  let launching: boolean = $state(false);
  let errorMessage: string | null = $state(null);

  let selectedAgentInfo = $derived(
    agents.find((a) => a.id === selectedAgent) ?? null
  );
  let agentNotInstalled = $derived(
    selectedAgentInfo?.version === "bunx" || selectedAgentInfo?.version === "npx"
  );
  let composeDetected = $derived(
    dockerContext?.file_type === "compose" && !dockerContext.force_host
  );
  let dockerSelectable = $derived(
    composeDetected &&
      (dockerContext?.docker_available ?? false) &&
      (dockerContext?.compose_available ?? false)
  );
  function supportsModelFor(agentId: string): boolean {
    return agentId === "codex" || agentId === "claude" || agentId === "gemini";
  }

  let supportsModel = $derived(supportsModelFor(selectedAgent));
  let supportsReasoning = $derived(selectedAgent === "codex");
  let supportsCollaboration = $derived(selectedAgent === "codex");
  let needsResumeSessionId = $derived(
    selectedAgent === "opencode" && sessionMode === "resume"
  );

  let modelOptions = $derived(
    selectedAgent === "codex"
      ? [
          "gpt-5.3-codex",
          "gpt-5.2-codex",
          "gpt-5.1-codex-max",
          "gpt-5.2",
          "gpt-5.1-codex-mini",
        ]
      : selectedAgent === "claude"
        ? ["opus", "sonnet", "haiku"]
        : selectedAgent === "gemini"
          ? [
              "gemini-3-pro-preview",
              "gemini-3-flash-preview",
              "gemini-2.5-pro",
              "gemini-2.5-flash",
              "gemini-2.5-flash-lite",
            ]
          : []
  );

  let versionSelectOptions = $derived(
    (() => {
      const opts: SelectOption[] = [];
      if (!selectedAgent) return opts;

      const seen = new Set<string>();

      if (!agentNotInstalled) {
        const ver = selectedAgentInfo?.version?.trim() || "installed";
        opts.push({ value: "installed", label: `Installed (${ver})` });
        seen.add("installed");
      }

      opts.push({ value: "latest", label: "latest" });
      seen.add("latest");

      for (const t of versionTags) {
        const tag = t.trim();
        if (!tag || seen.has(tag)) continue;
        opts.push({ value: tag, label: tag });
        seen.add(tag);
      }

      for (const v of versionOptions) {
        const ver = v.trim();
        if (!ver || seen.has(ver)) continue;
        opts.push({ value: ver, label: ver });
        seen.add(ver);
      }

      return opts;
    })()
  );

  $effect(() => {
    detectAgents();
  });

  $effect(() => {
    void projectPath;
    void branchMode;
    void existingBranch;
    void baseBranch;

    const refBranch = (branchMode === "existing" ? existingBranch : baseBranch).trim();
    if (!projectPath || !refBranch) {
      dockerContext = null;
      dockerError = null;
      dockerLoading = false;
      dockerContextKey = "";
      runtimeTarget = "host" as RuntimeTarget;
      dockerService = "";
      return;
    }

    const key = `${projectPath}::${refBranch}`;
    if (dockerContextKey === key) return;
    dockerContextKey = key;
    loadDockerContext(refBranch);
  });

  $effect(() => {
    if (selectedAgent === lastAgent) return;

    if (lastAgent && supportsModelFor(lastAgent)) {
      modelByAgent = { ...modelByAgent, [lastAgent]: model };
    }
    if (lastAgent) {
      agentVersionByAgent = { ...agentVersionByAgent, [lastAgent]: agentVersion };
    }

    lastAgent = selectedAgent;
    model = modelByAgent[selectedAgent] ?? "";

    const storedVersion = agentVersionByAgent[selectedAgent];
    if (storedVersion) {
      agentVersion =
        storedVersion === "installed" && agentNotInstalled
          ? "latest"
          : storedVersion;
    } else {
      agentVersion = agentNotInstalled ? "latest" : "installed";
    }
  });

  $effect(() => {
    if (!selectedAgent) {
      versionsLoading = false;
      versionsError = null;
      versionTags = [];
      versionOptions = [];
      return;
    }
    loadAgentVersions(selectedAgent);
  });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function parseExtraArgs(text: string): string[] {
    return text
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
  }

  function parseEnvOverrides(
    text: string
  ): { env: Record<string, string>; error: string | null } {
    const env: Record<string, string> = {};
    const lines = text.split("\n");
    for (let i = 0; i < lines.length; i++) {
      const raw = lines[i].trim();
      if (!raw || raw.startsWith("#")) continue;
      const idx = raw.indexOf("=");
      if (idx <= 0) {
        return {
          env: {},
          error: `Invalid env override at line ${i + 1}. Use KEY=VALUE.`,
        };
      }
      const key = raw.slice(0, idx).trim();
      const value = raw.slice(idx + 1).trimStart();
      if (!key) {
        return { env: {}, error: `Invalid env override at line ${i + 1}. Key is required.` };
      }
      env[key] = value;
    }
    return { env, error: null };
  }

  function buildNewBranchName(prefix: BranchPrefix, suffix: string): string {
    const s = suffix.trim();
    if (!s) return "";
    return `${prefix}${s}`;
  }

  function splitBranchNamePrefix(input: string): { prefix: BranchPrefix; suffix: string } | null {
    const trimmed = input.trim();
    for (const p of BRANCH_PREFIXES) {
      if (trimmed.startsWith(p)) {
        return { prefix: p, suffix: trimmed.slice(p.length) };
      }
    }
    return null;
  }

  function setNewBranchFromFullName(fullName: string): boolean {
    const parsed = splitBranchNamePrefix(fullName);
    if (!parsed) {
      suggestError = "Invalid suggestion prefix.";
      return false;
    }
    suggestError = null;
    newBranchPrefix = parsed.prefix;
    newBranchSuffix = parsed.suffix;
    return true;
  }

  function handleNewBranchSuffixInput(raw: string) {
    // When users paste a full branch name (e.g., "feature/foo"), split it and keep suffix editable.
    const parsed = splitBranchNamePrefix(raw);
    if (parsed) {
      newBranchPrefix = parsed.prefix;
      newBranchSuffix = parsed.suffix;
      return;
    }
    newBranchSuffix = raw;
  }

  async function loadBaseBranchOptions() {
    baseBranchOptionsLoading = true;
    baseBranchOptionsError = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const [local, remote] = await Promise.all([
        invoke<BranchInfo[]>("list_worktree_branches", { projectPath }),
        invoke<BranchInfo[]>("list_remote_branches", { projectPath }),
      ]);
      baseBranchLocalOptions = (local ?? []).map((b) => b.name);
      baseBranchRemoteOptions = (remote ?? []).map((b) => b.name);
    } catch (err) {
      baseBranchOptionsError = `Failed to load base branches: ${toErrorMessage(err)}`;
      baseBranchLocalOptions = [];
      baseBranchRemoteOptions = [];
    } finally {
      baseBranchOptionsLoading = false;
    }
  }

  $effect(() => {
    void projectPath;
    void branchMode;
    if (!projectPath || branchMode !== "new") return;
    void loadBaseBranchOptions();
  });

  function openSuggestModal() {
    suggestError = null;
    suggestSuggestions = [];
    suggestLoading = false;
    suggestOpen = true;
  }

  function closeSuggestModal() {
    suggestOpen = false;
  }

  async function generateBranchSuggestions() {
    suggestError = null;
    suggestSuggestions = [];
    const description = suggestDescription.trim();
    if (!description) {
      suggestError = "Description is required.";
      return;
    }

    suggestLoading = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<BranchSuggestResult>("suggest_branch_names", {
        description,
      });

      if (result.status === "ok") {
        const suggestions = (result.suggestions ?? [])
          .map((s) => s.trim())
          .filter((s) => s.length > 0);
        if (suggestions.length !== 3) {
          suggestSuggestions = [];
          suggestError = "Failed to generate suggestions.";
          return;
        }
        suggestSuggestions = suggestions;
      } else if (result.status === "ai-not-configured") {
        suggestError = "AI suggestions are unavailable.";
      } else {
        suggestError = result.error || "Failed to generate suggestions.";
      }
    } catch (err) {
      suggestError = toErrorMessage(err);
    } finally {
      suggestLoading = false;
    }
  }

  async function loadAgentVersions(agentId: string) {
    versionsLoading = true;
    versionsError = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const info = await invoke<AgentVersionsInfo>("list_agent_versions", { agentId });
      if (selectedAgent !== agentId) return;
      versionTags = info.tags ?? [];
      versionOptions = (info.versions ?? []).slice(0, 10);
    } catch (err) {
      if (selectedAgent !== agentId) return;
      versionsError = toErrorMessage(err);
      versionTags = [];
      versionOptions = [];
    } finally {
      if (selectedAgent === agentId) {
        versionsLoading = false;
      }
    }
  }

  async function loadDockerContext(refBranch: string) {
    dockerLoading = true;
    dockerError = null;
    try {
      const key = `${projectPath}::${refBranch}`;
      const { invoke } = await import("@tauri-apps/api/core");
      const ctx = await invoke<DockerContext>("detect_docker_context", {
        projectPath,
        branch: refBranch,
      });
      if (dockerContextKey !== key) return;

      dockerContext = ctx;

      if (!ctx || ctx.force_host || ctx.file_type !== "compose") {
        runtimeTarget = "host" as RuntimeTarget;
        dockerService = "";
        return;
      }

      runtimeTarget =
        ctx.docker_available && ctx.compose_available
          ? ("docker" as RuntimeTarget)
          : ("host" as RuntimeTarget);

      const services = ctx.compose_services ?? [];
      if (services.length === 0) {
        dockerService = "";
        return;
      }
      if (!services.includes(dockerService)) {
        dockerService = services[0];
      }
    } catch (err) {
      const key = `${projectPath}::${refBranch}`;
      if (dockerContextKey !== key) return;
      dockerContext = null;
      dockerError = toErrorMessage(err);
      runtimeTarget = "host" as RuntimeTarget;
      dockerService = "";
    } finally {
      const key = `${projectPath}::${refBranch}`;
      if (dockerContextKey === key) {
        dockerLoading = false;
      }
    }
  }

  async function detectAgents() {
    loading = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      agents = await invoke<AgentInfo[]>("detect_agents");
      const available = agents.find((a) => a.available);
      if (available) selectedAgent = available.id;
    } catch (err) {
      console.error("Failed to detect agents:", err);
      agents = [];
    }
    loading = false;
  }

  async function handleLaunch() {
    errorMessage = null;
    if (!selectedAgent) return;
    launching = true;
    try {
      const request: LaunchAgentRequest = {
        agentId: selectedAgent,
        branch: "",
        mode: sessionMode,
        skipPermissions,
      };

      if (supportsModel && model.trim()) {
        request.model = model.trim();
      }

      if (agentVersion.trim()) {
        request.agentVersion = agentVersion.trim();
      }

      if (sessionMode !== "normal" && resumeSessionId.trim()) {
        request.resumeSessionId = resumeSessionId.trim();
      }

      if (needsResumeSessionId && !request.resumeSessionId) {
        errorMessage = "Session ID is required for OpenCode resume.";
        return;
      }

      if (supportsReasoning && reasoningLevel.trim()) {
        request.reasoningLevel = reasoningLevel.trim();
      }

      if (supportsCollaboration) {
        request.collaborationModes = collaborationModes;
      }

      const extraArgs = parseExtraArgs(extraArgsText);
      if (extraArgs.length > 0) {
        request.extraArgs = extraArgs;
      }

      const envParsed = parseEnvOverrides(envOverridesText);
      if (envParsed.error) {
        errorMessage = envParsed.error;
        return;
      }
      if (Object.keys(envParsed.env).length > 0) {
        request.envOverrides = envParsed.env;
      }

      if (composeDetected) {
        if (runtimeTarget === "host") {
          request.dockerForceHost = true;
        } else if (runtimeTarget === "docker") {
          if (!dockerSelectable) {
            errorMessage = "Docker is not available on this system.";
            return;
          }
          const service = dockerService.trim();
          if (!service) {
            errorMessage = "Docker service is required.";
            return;
          }
          request.dockerService = service;
          request.dockerForceHost = false;
          request.dockerBuild = dockerBuild;
          request.dockerRecreate = dockerRecreate;
          request.dockerKeep = dockerKeep;
        }
      }

      if (branchMode === "existing") {
        if (!existingBranch.trim()) return;
        request.branch = existingBranch.trim();
        await onLaunch({
          ...request,
        });
        onClose();
        return;
      }

      const fullName = newBranchFullName.trim();
      if (!baseBranch.trim() || !fullName) return;
      request.branch = fullName;
      await onLaunch({
        ...request,
        createBranch: { name: fullName, base: baseBranch.trim() },
      });
      onClose();
    } catch (err) {
      errorMessage = `Failed to launch agent: ${toErrorMessage(err)}`;
    } finally {
      launching = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (suggestOpen) {
        closeSuggestModal();
        return;
      }
      onClose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_interactive_supports_focus -->
<div class="overlay" onclick={onClose} onkeydown={handleKeydown} role="dialog" aria-modal="true" aria-label="Launch Agent">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <div class="dialog-header">
      <h2>Launch Agent</h2>
      <button class="close-btn" onclick={onClose}>[x]</button>
    </div>

    {#if loading}
      <div class="loading">Detecting agents...</div>
    {:else}
      <div class="dialog-body">
        {#if errorMessage}
          <div class="error">{errorMessage}</div>
        {/if}

        <div class="field">
          <label for="agent-select">Agent</label>
          <select id="agent-select" bind:value={selectedAgent}>
            <option value="" disabled>Select an agent...</option>
            {#each agents as agent (agent.id)}
              <option value={agent.id} disabled={!agent.available}>
                {agent.name} ({agent.version}{agent.available ? "" : ", Unavailable"})
              </option>
            {/each}
          </select>
          {#if selectedAgentInfo}
            {#if !selectedAgentInfo.available}
              <span class="field-hint warn">Unavailable</span>
            {:else if selectedAgentInfo.version === "bunx" || selectedAgentInfo.version === "npx"}
              <span class="field-hint warn">
                Not installed. Launch will use {selectedAgentInfo.version}.
              </span>
            {:else if !selectedAgentInfo.authenticated}
              <span class="field-hint warn">Not authenticated</span>
            {/if}
          {/if}
        </div>

        {#if supportsModel}
          <div class="field">
            <label for="model-select">Model</label>
            <select id="model-select" bind:value={model}>
              <option value="">Default</option>
              {#each modelOptions as opt (opt)}
                <option value={opt}>{opt}</option>
              {/each}
            </select>
          </div>
        {/if}

        <div class="field">
          <label for="agent-version-select">Agent Version</label>
          <select id="agent-version-select" bind:value={agentVersion}>
            {#each versionSelectOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
          {#if agentNotInstalled}
            <span class="field-hint">
              Installed binary not found. Launch will use bunx/npx.
            </span>
          {/if}
          {#if versionsLoading}
            <span class="field-hint">Loading versions...</span>
          {:else if versionsError}
            <span class="field-hint warn">
              Failed to load version list from registry.
            </span>
          {/if}
        </div>

        {#if supportsReasoning}
          <div class="field">
            <label for="reasoning-select">Reasoning</label>
            <select id="reasoning-select" bind:value={reasoningLevel}>
              <option value="">Default</option>
              <option value="low">low</option>
              <option value="medium">medium</option>
              <option value="high">high</option>
              <option value="xhigh">xhigh</option>
            </select>
          </div>
        {/if}

        <div class="field">
          <span class="field-label" id="session-mode-label">Session</span>
          <div class="mode-toggle" role="group" aria-labelledby="session-mode-label">
            <button
              class="mode-btn"
              class:active={sessionMode === "normal"}
              onclick={() => (sessionMode = "normal")}
            >
              Normal
            </button>
            <button
              class="mode-btn"
              class:active={sessionMode === "continue"}
              onclick={() => (sessionMode = "continue")}
            >
              Continue
            </button>
            <button
              class="mode-btn"
              class:active={sessionMode === "resume"}
              onclick={() => (sessionMode = "resume")}
            >
              Resume
            </button>
          </div>
        </div>

        {#if sessionMode !== "normal"}
          <div class="field">
            <label for="resume-session-input">Session ID</label>
            <input
              id="resume-session-input"
              type="text"
              bind:value={resumeSessionId}
              placeholder={needsResumeSessionId ? "Required" : "Optional"}
            />
            {#if needsResumeSessionId}
              <span class="field-hint">OpenCode resume requires a session id.</span>
            {/if}
          </div>
        {/if}

        <div class="field">
          <span class="field-label">Permissions</span>
          <label class="check-row">
            <input type="checkbox" bind:checked={skipPermissions} />
            <span>Skip Permissions</span>
          </label>
        </div>

        {#if supportsCollaboration}
          <div class="field">
            <span class="field-label">Codex</span>
            <label class="check-row">
              <input type="checkbox" bind:checked={collaborationModes} />
              <span>Enable collaboration_modes</span>
            </label>
          </div>
        {/if}

        <div class="field">
          <button
            class="advanced-btn"
            type="button"
            onclick={() => (showAdvanced = !showAdvanced)}
          >
            {showAdvanced ? "Hide Advanced" : "Advanced"}
          </button>
        </div>

        {#if showAdvanced}
          <div class="field">
            <label for="extra-args-input">Extra Args</label>
            <textarea
              id="extra-args-input"
              rows="3"
              bind:value={extraArgsText}
              placeholder="One argument per line"
            ></textarea>
          </div>

          <div class="field">
            <label for="env-overrides-input">Env Overrides</label>
            <textarea
              id="env-overrides-input"
              rows="4"
              bind:value={envOverridesText}
              placeholder="KEY=VALUE (one per line)"
            ></textarea>
            <span class="field-hint">
              These variables are applied only for this launch.
            </span>
          </div>
        {/if}

        <div class="field">
          <span class="field-label" id="branch-mode-label">Branch</span>
          <div class="mode-toggle" role="group" aria-labelledby="branch-mode-label">
            <button
              class="mode-btn"
              class:active={branchMode === "existing"}
              onclick={() => (branchMode = "existing")}
            >
              Existing Branch
            </button>
            <button
              class="mode-btn"
              class:active={branchMode === "new"}
              onclick={() => (branchMode = "new")}
            >
              New Branch
            </button>
          </div>
        </div>

        {#if branchMode === "existing"}
          <div class="field">
            <label for="branch-input">Branch</label>
            <input id="branch-input" type="text" value={existingBranch} readonly />
            {#if !existingBranch.trim()}
              <span class="field-hint warn">No branch selected.</span>
            {/if}
          </div>
        {:else}
          <div class="field">
            <label for="base-branch-select">Base Branch</label>
            <select
              id="base-branch-select"
              bind:value={baseBranch}
              disabled={baseBranchOptionsLoading}
            >
              {#if !baseBranch.trim()}
                <option value="" disabled>Select base branch...</option>
              {/if}
              {#if baseBranch.trim() &&
                !baseBranchLocalOptions.includes(baseBranch) &&
                !baseBranchRemoteOptions.includes(baseBranch)}
                <option value={baseBranch}>{baseBranch}</option>
              {/if}
              <optgroup label="Local (Worktrees)">
                {#each baseBranchLocalOptions as name (name)}
                  <option value={name}>{name}</option>
                {/each}
              </optgroup>
              <optgroup label="Remote">
                {#each baseBranchRemoteOptions as name (name)}
                  <option value={name}>{name}</option>
                {/each}
              </optgroup>
            </select>
            {#if baseBranchOptionsLoading}
              <span class="field-hint">Loading branches...</span>
            {:else if baseBranchOptionsError}
              <span class="field-hint warn">{baseBranchOptionsError}</span>
            {/if}
          </div>
          <div class="field">
            <label for="new-branch-suffix-input">New Branch Name</label>
            <div class="branch-name-row">
              <select id="new-branch-prefix-select" bind:value={newBranchPrefix}>
                {#each BRANCH_PREFIXES as p (p)}
                  <option value={p}>{p}</option>
                {/each}
              </select>
              <input
                id="new-branch-suffix-input"
                type="text"
                value={newBranchSuffix}
                oninput={(e) =>
                  handleNewBranchSuffixInput((e.target as HTMLInputElement).value)}
                placeholder="e.g., my-change"
              />
              <button class="suggest-btn" type="button" onclick={openSuggestModal}>
                Suggest...
              </button>
            </div>
            <span class="field-hint">
              Full name: {newBranchFullName.trim() ? newBranchFullName : "(empty)"}
            </span>
          </div>
        {/if}

        {#if dockerLoading}
          <div class="field">
            <span class="field-hint">Detecting Docker context...</span>
          </div>
        {/if}

        {#if dockerError}
          <div class="field">
            <span class="field-hint warn">Docker detection failed: {dockerError}</span>
          </div>
        {/if}

        {#if composeDetected}
          <div class="field">
            <span class="field-label" id="runtime-label">Runtime</span>
            <div class="mode-toggle" role="group" aria-labelledby="runtime-label">
              <button
                class="mode-btn"
                class:active={runtimeTarget === "host"}
                onclick={() => (runtimeTarget = "host")}
              >
                HostOS
              </button>
              <button
                class="mode-btn"
                class:active={runtimeTarget === "docker"}
                disabled={!dockerSelectable}
                onclick={() => (runtimeTarget = "docker")}
              >
                Docker
              </button>
            </div>
            {#if dockerContext && !dockerContext.docker_available}
              <span class="field-hint warn">Docker is not available on PATH.</span>
            {:else if dockerContext && !dockerContext.compose_available}
              <span class="field-hint warn">docker compose is not available.</span>
            {:else if dockerContext && !dockerContext.daemon_running}
              <span class="field-hint warn">
                Docker daemon is not running. gwt will try to start it on launch.
              </span>
            {/if}
          </div>

          {#if runtimeTarget === "docker"}
            <div class="field">
              <label for="docker-service-select">Service</label>
              <select id="docker-service-select" bind:value={dockerService}>
                {#each (dockerContext?.compose_services ?? []) as svc (svc)}
                  <option value={svc}>{svc}</option>
                {/each}
              </select>
              {#if (dockerContext?.compose_services?.length ?? 0) === 0}
                <span class="field-hint warn">No services found in compose file.</span>
              {/if}
            </div>

            <div class="field">
              <span class="field-label">Docker</span>
              <label class="check-row">
                <input type="checkbox" bind:checked={dockerBuild} />
                <span>Build images</span>
              </label>
              <label class="check-row">
                <input type="checkbox" bind:checked={dockerRecreate} />
                <span>Force recreate</span>
              </label>
              <label class="check-row">
                <input type="checkbox" bind:checked={dockerKeep} />
                <span>Keep containers running after exit</span>
              </label>
            </div>
          {/if}
        {/if}
      </div>

      <div class="dialog-footer">
        <button class="btn btn-cancel" onclick={onClose}>Cancel</button>
        <button
          class="btn btn-launch"
          disabled={
            launching ||
            !selectedAgent ||
            (needsResumeSessionId && !resumeSessionId.trim()) ||
            (branchMode === "existing"
              ? !existingBranch.trim()
              : !baseBranch.trim() || !newBranchFullName.trim())
          }
          onclick={handleLaunch}
        >
          {launching ? "Launching..." : "Launch"}
        </button>
      </div>
    {/if}
  </div>

  {#if suggestOpen}
    <!-- Nested modal: stop propagation to avoid closing the Launch Agent dialog -->
    <div
      class="overlay suggest-overlay"
      onclick={(e) => {
        e.stopPropagation();
        if (e.target !== e.currentTarget) return;
        closeSuggestModal();
      }}
      role="dialog"
      aria-modal="true"
      aria-label="Suggest Branch Name"
    >
      <div class="dialog suggest-dialog">
        <div class="dialog-header">
          <h2>Suggest Branch Name</h2>
          <button class="close-btn" type="button" onclick={closeSuggestModal}>[x]</button>
        </div>

        <div class="dialog-body">
          {#if suggestError}
            <div class="error">{suggestError}</div>
          {/if}

          <div class="field">
            <label for="suggest-desc-input">Description</label>
            <textarea
              id="suggest-desc-input"
              rows="3"
              bind:value={suggestDescription}
              placeholder="What is this branch for?"
            ></textarea>
          </div>

          {#if suggestSuggestions.length > 0}
            <div class="field">
              <span class="field-label">Suggestions</span>
              <div class="suggestion-list">
                {#each suggestSuggestions as s (s)}
                  <button
                    class="suggestion-item"
                    type="button"
                    onclick={() => {
                      if (setNewBranchFromFullName(s)) {
                        closeSuggestModal();
                      }
                    }}
                  >
                    <span class="mono">{s}</span>
                  </button>
                {/each}
              </div>
            </div>
          {/if}
        </div>

        <div class="dialog-footer">
          <button class="btn btn-cancel" type="button" onclick={closeSuggestModal}>
            Close
          </button>
          <button
            class="btn btn-launch"
            type="button"
            disabled={suggestLoading}
            onclick={generateBranchSuggestions}
          >
            {suggestLoading ? "Generating..." : "Generate"}
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    width: 560px;
    max-width: 90vw;
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-color);
  }

  .dialog-header h2 {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    font-family: monospace;
    padding: 2px 4px;
  }

  .close-btn:hover {
    color: var(--text-primary);
  }

  .loading {
    padding: 40px;
    text-align: center;
    color: var(--text-muted);
  }

  .dialog-body {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow: auto;
  }

  .error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
    line-height: 1.4;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .field-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .field-hint {
    font-size: 11px;
    color: var(--text-muted);
    line-height: 1.4;
  }

  .field-hint.warn {
    color: rgb(255, 160, 160);
  }

  .mono {
    font-family: monospace;
  }

  .field input,
  .field textarea,
  .field select {
    padding: 8px 12px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: 13px;
    font-family: monospace;
    outline: none;
  }

  .field input:focus,
  .field textarea:focus,
  .field select:focus {
    border-color: var(--accent);
  }

  .field textarea {
    resize: vertical;
    line-height: 1.35;
  }

  .branch-name-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .branch-name-row select {
    width: 120px;
    flex: 0 0 auto;
  }

  .branch-name-row input {
    flex: 1;
    min-width: 0;
  }

  .suggest-btn {
    flex: 0 0 auto;
    padding: 8px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background 0.15s;
  }

  .suggest-btn:hover {
    border-color: var(--accent);
    background: var(--bg-surface);
  }

  .suggest-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .suggest-overlay {
    z-index: 1100;
  }

  .suggest-dialog {
    width: 520px;
    max-width: 92vw;
  }

  .suggestion-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .suggestion-item {
    padding: 10px 12px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    cursor: pointer;
    text-align: left;
    color: var(--text-primary);
    font-family: inherit;
    transition: border-color 0.15s, background 0.15s;
  }

  .suggestion-item:hover {
    border-color: var(--accent);
    background: var(--bg-surface);
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-primary);
    user-select: none;
  }

  .check-row input {
    accent-color: var(--accent);
  }

  .advanced-btn {
    width: 100%;
    padding: 8px 10px;
    background: transparent;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background 0.15s;
  }

  .advanced-btn:hover {
    border-color: var(--accent);
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .mode-toggle {
    display: flex;
    gap: 6px;
  }

  .mode-btn {
    flex: 1;
    padding: 8px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background 0.15s;
  }

  .mode-btn:hover {
    border-color: var(--accent);
  }

  .mode-btn.active {
    border-color: var(--accent);
    background: var(--bg-surface);
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 20px;
    border-top: 1px solid var(--border-color);
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
    transition: background-color 0.15s;
  }

  .btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
  }

  .btn-launch {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .btn-launch:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-launch:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
