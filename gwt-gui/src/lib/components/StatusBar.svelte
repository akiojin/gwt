<script lang="ts">
  import type { AgentInfo } from "../types";

  let {
    projectPath,
    currentBranch = "",
    terminalCount = 0,
    osEnvReady = true,
  }: {
    projectPath: string;
    currentBranch?: string;
    terminalCount?: number;
    osEnvReady?: boolean;
  } = $props();

  let agents: AgentInfo[] = $state([]);
  let agentsLoading: boolean = $state(false);

  const AGENT_ORDER: { id: string; label: string }[] = [
    { id: "claude", label: "Claude" },
    { id: "codex", label: "Codex" },
    { id: "gemini", label: "Gemini" },
    { id: "opencode", label: "OpenCode" },
  ];

  function getAgent(id: string): AgentInfo | null {
    return agents.find((a) => a.id === id) ?? null;
  }

  function agentStatusClass(agent: AgentInfo | null): string {
    if (!agent || !agent.available) return "bad";
    if (agent.version === "bunx" || agent.version === "npx") return "warn";
    return "ok";
  }

  function agentStatusText(agent: AgentInfo | null): string {
    if (!agent || !agent.available) return "not installed";
    const v = agent.version?.trim() ?? "";
    return v.length > 0 ? v : "installed";
  }

  async function detectAgents() {
    agentsLoading = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      agents = await invoke<AgentInfo[]>("detect_agents");
    } catch {
      agents = [];
    } finally {
      agentsLoading = false;
    }
  }

  $effect(() => {
    void projectPath;
    void osEnvReady;
    if (!osEnvReady) {
      agents = [];
      agentsLoading = false;
      return;
    }
    detectAgents();
  });
</script>

<footer class="statusbar">
  <span class="status-item">
    <span class="branch-indicator">*</span>
    {currentBranch || "---"}
  </span>
  {#if terminalCount > 0}
    <span class="status-item terminal-count">
      [{terminalCount} terminal{terminalCount !== 1 ? "s" : ""}]
    </span>
  {/if}
  {#if !osEnvReady}
    <span class="status-loading">Loading environment...</span>
  {/if}
  <span class="status-item agents">
    {#if !osEnvReady}
      <span class="agent muted">Agents: waiting</span>
    {:else if agentsLoading}
      <span class="agent muted">Agents: ...</span>
    {:else}
      {#each AGENT_ORDER as a (a.id)}
        <span class={`agent ${agentStatusClass(getAgent(a.id))}`}>
          {a.label}:{agentStatusText(getAgent(a.id))}
        </span>
      {/each}
    {/if}
  </span>
  <span class="spacer"></span>
  <span class="status-item path">{projectPath}</span>
</footer>

<style>
  .statusbar {
    display: flex;
    align-items: center;
    height: var(--statusbar-height);
    background-color: var(--bg-surface);
    border-top: 1px solid var(--border-color);
    padding: 0 12px;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    gap: 16px;
  }

  .branch-indicator {
    color: var(--green);
  }

  .terminal-count {
    color: var(--accent);
  }

  .agents {
    display: flex;
    gap: 10px;
    align-items: center;
    white-space: nowrap;
  }

  .agent {
    font-size: 10px;
    color: var(--text-muted);
  }

  .agent.ok {
    color: var(--green);
  }

  .agent.warn {
    color: var(--yellow);
  }

  .agent.bad {
    color: var(--red);
  }

  .agent.muted {
    color: var(--text-muted);
  }

  .spacer {
    flex: 1;
  }

  .status-loading {
    color: var(--text-muted);
    font-style: italic;
  }

  .path {
    font-family: monospace;
    font-size: var(--ui-font-xs);
  }
</style>
