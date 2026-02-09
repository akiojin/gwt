<script lang="ts">
  import type { AgentInfo, DockerContext, LaunchAgentRequest } from "../types";

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
  type RunnerMode = "auto" | "installed" | "bunx";
  type RuntimeTarget = "host" | "docker";

  type AgentVersionsInfo = {
    agentId: string;
    package: string;
    tags: string[];
    versions: string[];
    source: "cache" | "registry" | "fallback";
  };

		  let agents: AgentInfo[] = $state([]);
		  let selectedAgent: string = $state("");
		  let branchMode: BranchMode = $state("existing" as BranchMode);
		  let sessionMode: SessionMode = $state("normal" as SessionMode);
		  let runMethod: RunnerMode = $state("auto" as RunnerMode);
	  let runMethodByAgent: Record<string, RunnerMode> = $state({});
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

  // Intentionally capture the initial values - fields are editable by the user
  let branch: string = $state((() => selectedBranch)());
  let baseBranch: string = $state((() => selectedBranch)());
  let newBranch: string = $state("");

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
	    return (
	      agentId === "codex" ||
	      agentId === "claude" ||
	      agentId === "gemini" ||
	      agentId === "opencode"
	    );
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

  $effect(() => {
    detectAgents();
  });

  $effect(() => {
    void projectPath;
    void branchMode;
    void branch;
    void baseBranch;

    const refBranch =
      (branchMode === "existing" ? branch : baseBranch).trim();
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
	      runMethodByAgent = { ...runMethodByAgent, [lastAgent]: runMethod };
	    }

	    lastAgent = selectedAgent;
	    model = modelByAgent[selectedAgent] ?? "";
	    agentVersion = agentVersionByAgent[selectedAgent] ?? "latest";
	    runMethod =
	      runMethodByAgent[selectedAgent] ??
	      (agentNotInstalled ? "bunx" : "auto");
	  });

	  $effect(() => {
	    if (!selectedAgent || runMethod !== "bunx") {
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

	  function parseEnvOverrides(text: string): { env: Record<string, string>; error: string | null } {
	    const env: Record<string, string> = {};
	    const lines = text.split("\n");
	    for (let i = 0; i < lines.length; i++) {
	      const raw = lines[i].trim();
	      if (!raw || raw.startsWith("#")) continue;
	      const idx = raw.indexOf("=");
	      if (idx <= 0) {
	        return { env: {}, error: `Invalid env override at line ${i + 1}. Use KEY=VALUE.` };
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

	  async function loadAgentVersions(agentId: string) {
    versionsLoading = true;
    versionsError = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const info = await invoke<AgentVersionsInfo>("list_agent_versions", { agentId });
      if (selectedAgent !== agentId) return;
      versionTags = info.tags ?? [];
      versionOptions = info.versions ?? [];
    } catch (err) {
      if (selectedAgent !== agentId) return;
      versionsError = toErrorMessage(err);
      versionTags = ["latest"];
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

	      if (runMethod === "installed") {
	        request.agentVersion = "installed";
	      } else if (runMethod === "bunx") {
	        request.agentVersion = agentVersion.trim() || "latest";
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
	        if (!branch.trim()) return;
	        request.branch = branch.trim();
	        await onLaunch({
	          ...request,
	        });
	        onClose();
	        return;
	      }

	      if (!baseBranch.trim() || !newBranch.trim()) return;
	      request.branch = newBranch.trim();
	      await onLaunch({
	        ...request,
	        createBranch: { name: newBranch.trim(), base: baseBranch.trim() },
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
          <div class="agent-cards">
            {#each agents as agent}
              <button
                class="agent-card"
                class:selected={selectedAgent === agent.id}
                class:unavailable={!agent.available}
                disabled={!agent.available}
                onclick={() => (selectedAgent = agent.id)}
              >
                <span class="agent-name">{agent.name}</span>
                <span class="agent-type">{agent.version}</span>
                {#if !agent.available}
                  <span class="agent-status">Unavailable</span>
                {:else if agent.version === "bunx" || agent.version === "npx"}
                  <span class="agent-status">
                    Not installed ({agent.version})
                  </span>
                {:else if !agent.authenticated}
                  <span class="agent-status">Not authenticated</span>
                {/if}
              </button>
            {/each}
          </div>
        </div>

        {#if supportsModel}
          <div class="field">
            <label for="model-input">Model</label>
            <input
              id="model-input"
              type="text"
              list="model-options"
              bind:value={model}
              placeholder="Optional (e.g., gpt-5.3-codex, opus, gemini-2.5-pro, provider/model)"
            />
            <datalist id="model-options">
              {#each modelOptions as opt (opt)}
                <option value={opt}></option>
              {/each}
            </datalist>
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
          <span class="field-label" id="run-method-label">Run Method</span>
          <div class="mode-toggle" role="group" aria-labelledby="run-method-label">
            <button
              class="mode-btn"
              class:active={runMethod === "auto"}
              onclick={() => (runMethod = "auto")}
            >
              Auto
            </button>
            <button
              class="mode-btn"
              class:active={runMethod === "installed"}
              onclick={() => (runMethod = "installed")}
            >
              Installed
            </button>
            <button
              class="mode-btn"
              class:active={runMethod === "bunx"}
              onclick={() => (runMethod = "bunx")}
            >
              bunx/npx
            </button>
          </div>
          {#if agentNotInstalled}
            <span class="field-hint">
              Installed binary not found. Auto will use bunx/npx.
            </span>
          {/if}
        </div>

        {#if runMethod === "bunx"}
          <div class="field">
            <label for="agent-version-input">Agent Version</label>
            <input
              id="agent-version-input"
              type="text"
              list="agent-version-options"
              bind:value={agentVersion}
              placeholder="latest"
            />
            <datalist id="agent-version-options">
              {#each versionTags as tag (tag)}
                <option value={tag}></option>
              {/each}
              {#each versionOptions as ver (ver)}
                <option value={ver}></option>
              {/each}
            </datalist>
            {#if versionsLoading}
              <span class="field-hint">Loading versions...</span>
            {:else if versionsError}
              <span class="field-hint">
                Failed to load versions. You can still type a version or dist-tag.
              </span>
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
          <input
            id="branch-input"
            type="text"
            bind:value={branch}
            placeholder="Enter branch name..."
          />
        </div>
        {:else}
          <div class="field">
            <label for="base-branch-input">Base Branch</label>
            <input
              id="base-branch-input"
              type="text"
              bind:value={baseBranch}
              placeholder="e.g., develop or origin/develop"
            />
          </div>
          <div class="field">
            <label for="new-branch-input">New Branch Name</label>
            <input
              id="new-branch-input"
              type="text"
              bind:value={newBranch}
              placeholder="e.g., feature/my-change"
            />
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
              ? !branch.trim()
              : !baseBranch.trim() || !newBranch.trim())
          }
          onclick={handleLaunch}
        >
          {launching ? "Launching..." : "Launch"}
        </button>
      </div>
    {/if}
  </div>
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

  .agent-cards {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .agent-card {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    cursor: pointer;
    font-family: inherit;
    color: var(--text-primary);
    text-align: left;
    transition: border-color 0.15s;
  }

  .agent-card:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .agent-card.selected {
    border-color: var(--accent);
    background: var(--bg-surface);
  }

  .agent-card.unavailable {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .agent-name {
    font-size: 13px;
    font-weight: 500;
  }

  .agent-type {
    font-size: 11px;
    color: var(--text-muted);
    margin-left: auto;
  }

  .agent-status {
    font-size: 10px;
    color: var(--red);
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
