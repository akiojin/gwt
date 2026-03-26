<script lang="ts">
  import type { AgentInfo } from "../types";
  import type { GpuInfo } from "../systemMonitor.svelte";
  import { formatAboutVersion, getAppVersionSafe } from "../windowTitle";
  import { renderBar, usageColorClass, formatMemory } from "./statusBarHelpers";

  type TabId = "general" | "system" | "statistics";

  interface AgentStatEntry {
    agent_id: string;
    model: string;
    count: number;
  }

  interface StatsEntryResponse {
    agents: AgentStatEntry[];
    worktrees_created: number;
  }

  interface RepoStatsEntry {
    repo_path: string;
    stats: StatsEntryResponse;
  }

  interface StatsResponse {
    global: StatsEntryResponse;
    repos: RepoStatsEntry[];
  }

  let {
    open = false,
    initialTab = "general" as TabId,
    cpuUsage = 0,
    memUsed = 0,
    memTotal = 0,
    gpuInfos = [] as GpuInfo[],
    onclose,
  }: {
    open?: boolean;
    initialTab?: TabId;
    cpuUsage?: number;
    memUsed?: number;
    memTotal?: number;
    gpuInfos?: GpuInfo[];
    onclose: () => void;
  } = $props();

  let activeTab: TabId = $state("general");
  let appVersion: string | null = $state(null);
  let agents: AgentInfo[] = $state([]);

  let statsData: StatsResponse | null = $state(null);
  let statsLoading = $state(false);
  let statsSelectedRepo = $state("all");

  const TABS: { id: TabId; label: string }[] = [
    { id: "general", label: "General" },
    { id: "system", label: "System" },
    { id: "statistics", label: "Statistics" },
  ];

  let cpuPct = $derived(Math.round(cpuUsage));
  let memPct = $derived(memTotal > 0 ? Math.round((memUsed / memTotal) * 100) : 0);

  function getDisplayAgents(data: StatsResponse | null, repo: string): AgentStatEntry[] {
    if (!data) return [];
    if (repo === "all") return data.global.agents;
    const found = data.repos.find((r) => r.repo_path === repo);
    return found ? found.stats.agents : [];
  }

  function getDisplayWorktrees(data: StatsResponse | null, repo: string): number {
    if (!data) return 0;
    if (repo === "all") return data.global.worktrees_created;
    const found = data.repos.find((r) => r.repo_path === repo);
    return found ? found.stats.worktrees_created : 0;
  }

  let displayAgents = $derived(getDisplayAgents(statsData, statsSelectedRepo));
  let displayWorktrees = $derived(getDisplayWorktrees(statsData, statsSelectedRepo));

  // Reset activeTab when dialog opens
  $effect(() => {
    if (open) {
      activeTab = initialTab;
      loadVersion();
      loadAgents();
    }
  });

  // Load stats when statistics tab becomes active
  $effect(() => {
    if (open && activeTab === "statistics") {
      loadStats();
    }
  });

  async function loadVersion() {
    appVersion = await getAppVersionSafe();
  }

  async function loadAgents() {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      agents = await invoke<AgentInfo[]>("detect_agents");
    } catch {
      agents = [];
    }
  }

  async function loadStats() {
    statsLoading = true;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      statsData = await invoke<StatsResponse>("get_stats");
    } catch {
      statsData = null;
    } finally {
      statsLoading = false;
    }
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay modal-overlay" onclick={onclose}>
    <div class="about-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
      <div class="tab-bar">
        {#each TABS as tab (tab.id)}
          <button
            class="tab-btn"
            class:active={activeTab === tab.id}
            onclick={() => (activeTab = tab.id)}
          >
            {tab.label}
          </button>
        {/each}
      </div>

      <div class="tab-content">
        {#if activeTab === "general"}
          <div class="general-tab">
            <h2>gwt</h2>
            <p>Git Worktree Manager</p>
            <p class="about-edition">GUI Edition</p>
            <p class="about-version">{formatAboutVersion(appVersion)}</p>
            {#if agents.length > 0}
              <div class="agent-list">
                <h3>Detected Agents</h3>
                {#each agents as agent (agent.id)}
                  <div class="agent-row">
                    <span class="agent-name">{agent.name}</span>
                    <span class="agent-ver">{agent.available ? (agent.version || "installed") : "not installed"}</span>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {:else if activeTab === "system"}
          <div class="system-tab">
            <div class="sys-section">
              <span class="sys-heading">CPU</span>
              <span class={`sys-bar ${usageColorClass(cpuPct)}`}>{renderBar(cpuPct)}</span>
              <span class={`sys-value ${usageColorClass(cpuPct)}`}>{cpuPct}%</span>
            </div>
            <div class="sys-section">
              <span class="sys-heading">Memory</span>
              <span class={`sys-bar ${usageColorClass(memPct)}`}>{renderBar(memPct)}</span>
              <span class={`sys-value ${usageColorClass(memPct)}`}>{formatMemory(memUsed)} / {formatMemory(memTotal)} GB ({memPct}%)</span>
            </div>
            {#if gpuInfos.length > 0}
              {#each gpuInfos as gpuInfo, index (`${gpuInfo.name}-${index}`)}
                <div class="sys-section">
                  <span class="sys-heading">{gpuInfos.length > 1 ? `GPU ${index + 1}` : "GPU"}</span>
                  <span class="sys-value">{gpuInfo.name}</span>
                  {#if gpuInfo.usage_percent != null}
                    <span class={`sys-bar ${usageColorClass(gpuInfo.usage_percent)}`}>{renderBar(gpuInfo.usage_percent)}</span>
                    <span class={`sys-value ${usageColorClass(gpuInfo.usage_percent)}`}>{Math.round(gpuInfo.usage_percent)}%</span>
                  {/if}
                  {#if gpuInfo.vram_total_bytes != null && gpuInfo.vram_used_bytes != null}
                    <span class="sys-value">VRAM: {formatMemory(gpuInfo.vram_used_bytes)} / {formatMemory(gpuInfo.vram_total_bytes)} GB</span>
                  {:else if gpuInfo.vram_total_bytes != null}
                    <span class="sys-value">VRAM: {formatMemory(gpuInfo.vram_total_bytes)} GB</span>
                  {/if}
                </div>
              {/each}
            {:else}
              <div class="sys-section">
                <span class="sys-heading">GPU</span>
                <span class="sys-value muted">No GPU detected</span>
              </div>
            {/if}
          </div>
        {:else if activeTab === "statistics"}
          <div class="statistics-tab">
            {#if statsLoading}
              <p class="stats-muted">Loading statistics...</p>
            {:else if !statsData || (statsData.global.agents.length === 0 && statsData.repos.length === 0)}
              <p class="stats-muted">No statistics yet</p>
            {:else}
              <div class="stats-filter">
                <label for="repo-filter">Repository:</label>
                <select id="repo-filter" bind:value={statsSelectedRepo}>
                  <option value="all">All repositories</option>
                  {#each statsData.repos as repo (repo.repo_path)}
                    <option value={repo.repo_path}>{repo.repo_path}</option>
                  {/each}
                </select>
              </div>
              {#if displayAgents.length > 0}
                <table class="stats-table">
                  <thead>
                    <tr>
                      <th>Agent</th>
                      <th>Model</th>
                      <th>Count</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each displayAgents as entry}
                      <tr>
                        <td>{entry.agent_id}</td>
                        <td>{entry.model}</td>
                        <td class="count">{entry.count}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              {:else}
                <p class="stats-muted">No agent launches in this scope</p>
              {/if}
              <div class="stats-wt">
                <span>Worktrees created:</span>
                <strong>{displayWorktrees}</strong>
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <button class="about-close" onclick={onclose}>Close</button>
    </div>
  </div>
{/if}

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
    z-index: var(--z-modal-base);
  }

  .about-dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    padding: var(--space-6) var(--space-8);
    box-shadow: var(--shadow-xl);
    min-width: 420px;
    max-width: 600px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
  }

  .tab-bar {
    display: flex;
    gap: var(--space-1);
    margin-bottom: var(--space-4);
    border-bottom: 1px solid var(--border-color);
    padding-bottom: var(--space-2);
  }

  .tab-btn {
    background: none;
    border: 1px solid transparent;
    border-radius: 6px 6px 0 0;
    padding: 6px 14px;
    color: var(--text-muted);
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-sm);
  }

  .tab-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .tab-btn.active {
    color: var(--accent);
    border-color: var(--border-color);
    border-bottom-color: var(--bg-secondary);
    background: var(--bg-secondary);
  }

  .tab-content {
    flex: 1;
    overflow-y: auto;
    min-height: 200px;
  }

  /* General tab */
  .general-tab {
    text-align: center;
  }

  .general-tab h2 {
    font-size: 24px;
    font-weight: 700;
    color: var(--accent);
    margin-bottom: var(--space-1);
  }

  .general-tab p {
    color: var(--text-secondary);
    font-size: var(--ui-font-base);
  }

  .about-edition {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    margin-top: var(--space-1);
  }

  .about-version {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    margin-top: var(--space-1);
    margin-bottom: var(--space-4);
  }

  .agent-list {
    text-align: left;
    border-top: 1px solid var(--border-color);
    padding-top: var(--space-3);
    margin-top: var(--space-2);
  }

  .agent-list h3 {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    margin-bottom: var(--space-2);
  }

  .agent-row {
    display: flex;
    justify-content: space-between;
    padding: var(--space-1) 0;
    font-size: var(--ui-font-sm);
  }

  .agent-name {
    color: var(--text-primary);
  }

  .agent-ver {
    color: var(--text-muted);
    font-family: monospace;
  }

  /* System tab */
  .system-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .sys-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: 10px var(--space-3);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    background: var(--bg-primary);
  }

  .sys-heading {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    font-weight: 600;
  }

  .sys-bar {
    font-family: monospace;
    font-size: var(--ui-font-md);
    white-space: pre;
  }

  .sys-value {
    font-size: var(--ui-font-sm);
  }

  .sys-bar.ok,
  .sys-value.ok {
    color: var(--green);
  }

  .sys-bar.warn,
  .sys-value.warn {
    color: var(--yellow);
  }

  .sys-bar.bad,
  .sys-value.bad {
    color: var(--red);
  }

  .sys-value.muted {
    color: var(--text-muted);
  }

  /* Statistics tab */
  .statistics-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .stats-muted {
    color: var(--text-muted);
    font-size: var(--ui-font-md);
    text-align: center;
    padding: var(--space-6) 0;
  }

  .stats-filter {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--ui-font-sm);
    color: var(--text-secondary);
  }

  .stats-filter select {
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-sm);
    padding: var(--space-1) var(--space-2);
    flex: 1;
  }

  .stats-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--ui-font-sm);
  }

  .stats-table th {
    text-align: left;
    padding: 6px var(--space-2);
    border-bottom: 1px solid var(--border-color);
    color: var(--text-muted);
    font-weight: 600;
  }

  .stats-table td {
    padding: 6px var(--space-2);
    border-bottom: 1px solid var(--border-color);
    color: var(--text-primary);
  }

  .stats-table .count {
    text-align: right;
    font-family: monospace;
  }

  .stats-wt {
    display: flex;
    justify-content: space-between;
    padding: 10px var(--space-3);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    background: var(--bg-primary);
    font-size: var(--ui-font-sm);
    color: var(--text-secondary);
  }

  .stats-wt strong {
    color: var(--text-primary);
    font-family: monospace;
  }

  /* Close button */
  .about-close {
    margin-top: var(--space-4);
    padding: 6px var(--space-5);
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-md);
    align-self: center;
  }

  .about-close:hover {
    background: var(--bg-hover);
  }
</style>
