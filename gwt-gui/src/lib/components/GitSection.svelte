<script lang="ts">
  import type { GitChangeSummary } from "../types";
  import GitChangesTab from "./GitChangesTab.svelte";
  import GitCommitsTab from "./GitCommitsTab.svelte";
  import GitStashTab from "./GitStashTab.svelte";

  let {
    projectPath,
    branch,
  }: {
    projectPath: string;
    branch: string;
  } = $props();

  let collapsed: boolean = $state(true);
  let loading: boolean = $state(false);
  let error: string | null = $state(null);
  let summary = $state<GitChangeSummary | null>(null);

  let baseBranchCandidates: string[] = $state([]);
  let baseBranch: string = $state("");

  type TabId = "changes" | "commits" | "stash";
  let activeTab: TabId = $state("changes");

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function normalizeBranchName(name: string): string {
    return name.startsWith("origin/") ? name.slice("origin/".length) : name;
  }

  async function loadSummary() {
    loading = true;
    error = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");

      const [summaryResult, candidates] = await Promise.all([
        invoke<GitChangeSummary>("get_git_change_summary", {
          projectPath,
          branch: normalizeBranchName(branch),
          baseBranch: baseBranch || undefined,
        }),
        invoke<string[]>("get_base_branch_candidates", { projectPath }),
      ]);

      summary = summaryResult;
      baseBranchCandidates = candidates;

      if (!baseBranch && summaryResult.base_branch) {
        baseBranch = summaryResult.base_branch;
      }
    } catch (err) {
      error = toErrorMessage(err);
      summary = null;
    } finally {
      loading = false;
    }
  }

  async function refresh() {
    await loadSummary();
  }

  function handleBaseBranchChange(e: Event) {
    const select = e.target as HTMLSelectElement;
    baseBranch = select.value;
    loadSummary();
  }

  $effect(() => {
    void projectPath;
    void branch;
    loadSummary();
  });

  let summaryText = $derived.by(() => {
    if (!summary) return "";
    const parts: string[] = [];
    parts.push(`${summary.file_count} file${summary.file_count === 1 ? "" : "s"}`);
    parts.push(`${summary.commit_count} commit${summary.commit_count === 1 ? "" : "s"}`);
    if (summary.stash_count > 0) {
      parts.push(`${summary.stash_count} stash`);
    }
    return parts.join(", ");
  });

  let showStashTab = $derived((summary?.stash_count ?? 0) > 0);
</script>

<div class="git-section">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="git-header" onclick={() => (collapsed = !collapsed)}>
    <div class="git-header-left">
      <span class="collapse-icon">{collapsed ? ">" : "v"}</span>
      <span class="git-title">Git</span>
      {#if loading}
        <span class="git-summary-text">Loading git info...</span>
      {:else if summaryText}
        <span class="git-summary-text">{summaryText}</span>
      {/if}
    </div>
    <button
      class="refresh-btn"
      title="Refresh"
      onclick={(e) => {
        e.stopPropagation();
        refresh();
      }}
    >
      [R]
    </button>
  </div>

  {#if !collapsed}
    <div class="git-body">
      {#if error}
        <div class="git-error">{error}</div>
      {:else if loading}
        <div class="git-loading">Loading git info...</div>
      {:else}
        <!-- Base branch selector -->
        {#if baseBranchCandidates.length > 0}
          <div class="base-branch-row">
            <label class="base-label" for="base-branch-select">Base:</label>
            <select
              id="base-branch-select"
              class="base-select"
              value={baseBranch}
              onchange={handleBaseBranchChange}
            >
              {#each baseBranchCandidates as candidate}
                <option value={candidate}>{candidate}</option>
              {/each}
            </select>
          </div>
        {/if}

        <!-- Tab bar -->
        <div class="git-tabs">
          <button
            class="git-tab-btn"
            class:active={activeTab === "changes"}
            onclick={() => (activeTab = "changes")}
          >
            Changes
          </button>
          <button
            class="git-tab-btn"
            class:active={activeTab === "commits"}
            onclick={() => (activeTab = "commits")}
          >
            Commits
          </button>
          {#if showStashTab}
            <button
              class="git-tab-btn"
              class:active={activeTab === "stash"}
              onclick={() => (activeTab = "stash")}
            >
              Stash
            </button>
          {/if}
        </div>

        <!-- Tab content -->
        <div class="git-tab-content">
          {#if activeTab === "changes"}
            <GitChangesTab {projectPath} branch={normalizeBranchName(branch)} {baseBranch} />
          {:else if activeTab === "commits"}
            <GitCommitsTab {projectPath} branch={normalizeBranchName(branch)} {baseBranch} />
          {:else if activeTab === "stash"}
            <GitStashTab {projectPath} />
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .git-section {
    margin-top: 16px;
    border: 1px solid var(--border-color);
    border-radius: 12px;
    background: var(--bg-secondary);
    overflow: hidden;
  }

  .git-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    cursor: pointer;
    user-select: none;
  }

  .git-header:hover {
    background: var(--bg-hover);
  }

  .git-header-left {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .collapse-icon {
    font-family: monospace;
    font-size: 11px;
    color: var(--text-muted);
    width: 10px;
    flex-shrink: 0;
  }

  .git-title {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    color: var(--text-secondary);
  }

  .git-summary-text {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .refresh-btn {
    background: none;
    border: 1px solid var(--border-color);
    border-radius: 4px;
    color: var(--text-muted);
    font-size: 11px;
    font-family: monospace;
    cursor: pointer;
    padding: 2px 6px;
    flex-shrink: 0;
  }

  .refresh-btn:hover {
    color: var(--text-primary);
    border-color: var(--accent);
  }

  .git-body {
    padding: 0 14px 14px;
  }

  .git-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
  }

  .git-loading {
    font-size: 12px;
    color: var(--text-muted);
    padding: 8px 0;
  }

  .base-branch-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }

  .base-label {
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 600;
  }

  .base-select {
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    color: var(--text-primary);
    font-size: 12px;
    font-family: monospace;
    padding: 3px 8px;
    cursor: pointer;
  }

  .base-select:focus {
    outline: none;
    border-color: var(--accent);
  }

  .git-tabs {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border-color);
    margin-bottom: 4px;
  }

  .git-tab-btn {
    padding: 6px 14px;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
  }

  .git-tab-btn:hover {
    color: var(--text-secondary);
  }

  .git-tab-btn.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
  }

  .git-tab-content {
    min-height: 0;
  }
</style>
