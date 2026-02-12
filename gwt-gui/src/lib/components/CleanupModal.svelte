<script lang="ts">
  import type { WorktreeInfo, CleanupResult } from "../types";
  import { untrack } from "svelte";
  import { flip } from "svelte/animate";

  let {
    open = false,
    preselectedBranch = null,
    refreshKey = 0,
    agentTabBranches = [],
    projectPath,
    onClose,
  }: {
    open: boolean;
    preselectedBranch?: string | null;
    refreshKey?: number;
    agentTabBranches?: string[];
    projectPath: string;
    onClose: () => void;
  } = $props();

  function normalizeTabBranch(name: string): string {
    const trimmed = name.trim();
    return trimmed.startsWith("origin/") ? trimmed.slice("origin/".length) : trimmed;
  }

  let agentTabBranchSet = $derived(
    new Set(
      agentTabBranches
        .map((b) => normalizeTabBranch(b))
        .filter((b) => b && b !== "Worktree" && b !== "Agent")
    )
  );

  let worktrees: WorktreeInfo[] = $state([]);
  let loading: boolean = $state(false);
  let errorMessage: string | null = $state(null);
  let checked: Set<string> = $state(new Set());
  type ConfirmMode = "unsafe" | "active" | "both";
  let confirmMode: ConfirmMode | null = $state(null);
  let cleaning: boolean = $state(false);

  // Failure re-open state
  let failures: CleanupResult[] = $state([]);
  let showFailures: boolean = $state(false);

  const SAFETY_ORDER: Record<string, number> = {
    safe: 0,
    warning: 1,
    danger: 2,
    disabled: 3,
  };

  const isVitest = typeof (import.meta as unknown as { vitest?: unknown }).vitest !== "undefined";

  const flipEnabled =
    !isVitest &&
    typeof navigator !== "undefined" &&
    typeof navigator.userAgent === "string" &&
    !navigator.userAgent.toLowerCase().includes("jsdom") &&
    typeof (Element.prototype as unknown as { animate?: unknown }).animate === "function" &&
    typeof (Element.prototype as unknown as { getAnimations?: unknown }).getAnimations === "function";

  let sortedWorktrees = $derived(
    [...worktrees].sort(
      (a, b) =>
        (isDisabled(a) ? 1 : 0) - (isDisabled(b) ? 1 : 0) ||
        (a.branch && agentTabBranchSet.has(normalizeTabBranch(a.branch)) ? -1 : 0) -
          (b.branch && agentTabBranchSet.has(normalizeTabBranch(b.branch)) ? -1 : 0) ||
        (SAFETY_ORDER[a.safety_level] ?? 99) - (SAFETY_ORDER[b.safety_level] ?? 99) ||
        (a.branch ?? a.path).localeCompare(b.branch ?? b.path)
    )
  );

  let checkedCount = $derived(checked.size);

  let hasUnsafeChecked = $derived(
    sortedWorktrees.some(
      (w) =>
        w.branch &&
        checked.has(w.branch) &&
        (w.safety_level === "warning" || w.safety_level === "danger")
    )
  );

  let unsafeCheckedCount = $derived(
    sortedWorktrees.filter(
      (w) =>
        w.branch &&
        checked.has(w.branch) &&
        (w.safety_level === "warning" || w.safety_level === "danger")
    ).length
  );

  let activeTabCheckedCount = $derived(
    sortedWorktrees.filter(
      (w) =>
        w.branch &&
        checked.has(w.branch) &&
        agentTabBranchSet.has(normalizeTabBranch(w.branch))
    ).length
  );

  let hasActiveTabChecked = $derived(activeTabCheckedCount > 0);

  let wasOpen = false;
  let lastRefreshKey = -1;

  $effect(() => {
    if (!open) {
      wasOpen = false;
      lastRefreshKey = -1;
      return;
    }

    // Depend on refreshKey while open so the list updates when worktrees change.
    const rk = refreshKey;

    if (!wasOpen) {
      wasOpen = true;
      lastRefreshKey = rk;
      showFailures = false;
      failures = [];
      untrack(() => {
        fetchWorktrees({ preserveChecked: false });
      });
      return;
    }

    if (rk === lastRefreshKey) return;
    lastRefreshKey = rk;
    untrack(() => {
      fetchWorktrees({ preserveChecked: true });
    });
  });

  async function fetchWorktrees({ preserveChecked }: { preserveChecked: boolean }) {
    loading = true;
    errorMessage = null;
    const previouslyChecked = new Set(checked);
    if (!preserveChecked) checked = new Set();
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      worktrees = await invoke<WorktreeInfo[]>("list_worktrees", {
        projectPath,
      });

      // Pre-select branch if provided
      if (!preserveChecked && preselectedBranch) {
        const wt = worktrees.find((w) => w.branch === preselectedBranch);
        if (wt && wt.safety_level !== "disabled") {
          checked = new Set([preselectedBranch]);
        }
      } else if (preserveChecked) {
        const allowed = new Set(
          worktrees
            .filter((w) => w.branch && w.safety_level !== "disabled")
            .map((w) => w.branch as string)
        );
        checked = new Set([...previouslyChecked].filter((b) => allowed.has(b)));
      }
    } catch (err) {
      errorMessage = `Failed to list worktrees: ${toErrorMessage(err)}`;
      worktrees = [];
    } finally {
      loading = false;
    }
  }

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function isDisabled(w: WorktreeInfo): boolean {
    return w.safety_level === "disabled";
  }

  function toggleCheck(branch: string) {
    const next = new Set(checked);
    if (next.has(branch)) {
      next.delete(branch);
    } else {
      next.add(branch);
    }
    checked = next;
  }

  function selectAllSafe() {
    const next = new Set<string>();
    for (const w of worktrees) {
      if (w.branch && w.safety_level === "safe") {
        next.add(w.branch);
      }
    }
    checked = next;
  }

  function handleCleanup() {
    if (checkedCount === 0) return;
    if (hasUnsafeChecked && hasActiveTabChecked) {
      confirmMode = "both";
      return;
    }
    if (hasUnsafeChecked) {
      confirmMode = "unsafe";
      return;
    }
    if (hasActiveTabChecked) {
      confirmMode = "active";
      return;
    }
    executeCleanup(false);
  }

  function confirmCleanup() {
    const force = hasUnsafeChecked;
    confirmMode = null;
    executeCleanup(force);
  }

  function cancelConfirm() {
    confirmMode = null;
  }

  async function executeCleanup(force: boolean) {
    cleaning = true;
    const branches = Array.from(checked);
    try {
      const { invoke } = await import("@tauri-apps/api/core");

      // Close modal immediately
      onClose();

      // Listen for cleanup-completed to handle failures
      const { listen } = await import("@tauri-apps/api/event");
      const unlisten = await listen<{ results: CleanupResult[] }>(
        "cleanup-completed",
        (event) => {
          unlisten();
          const failed = (event.payload.results ?? []).filter(
            (r) => !r.success
          );
          if (failed.length > 0) {
            failures = failed;
            showFailures = true;
          }
        }
      );

      await invoke("cleanup_worktrees", {
        projectPath,
        branches,
        force,
      });
    } catch (err) {
      // If invoke itself fails, show as a single failure
      failures = branches.map((b) => ({
        branch: b,
        success: false,
        error: toErrorMessage(err),
      }));
      showFailures = true;
    } finally {
      cleaning = false;
    }
  }

  function closeFailures() {
    showFailures = false;
    failures = [];
  }

  function safetyDotClass(level: string): string {
    switch (level) {
      case "safe":
        return "dot-safe";
      case "warning":
        return "dot-warning";
      case "danger":
        return "dot-danger";
      default:
        return "dot-disabled";
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (confirmMode) {
        cancelConfirm();
        return;
      }
      onClose();
    }
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    class="overlay"
    onclick={onClose}
    onkeydown={handleKeydown}
    role="dialog"
    aria-modal="true"
    aria-label="Cleanup Worktrees"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="dialog" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-header">
        <h2>Cleanup Worktrees</h2>
        <button class="close-btn" onclick={onClose}>[x]</button>
      </div>

      {#if loading}
        <div class="loading">Loading worktrees...</div>
      {:else}
        <div class="dialog-body">
          {#if errorMessage}
            <div class="error">{errorMessage}</div>
          {/if}

          <div class="toolbar">
            <button class="toolbar-btn" onclick={selectAllSafe}>
              Select All Safe
            </button>
            <span class="toolbar-count">
              {checkedCount} selected
            </span>
          </div>

          <div class="table-wrapper">
            <table class="wt-table">
              <thead>
                <tr>
                  <th class="col-check"></th>
                  <th class="col-safety"></th>
                  <th class="col-branch">Branch</th>
                  <th class="col-status">Status</th>
                  <th class="col-markers">Changes</th>
                  <th class="col-sync">Ahead/Behind</th>
                  <th class="col-gone">Gone</th>
                  <th class="col-tool">Tool</th>
                </tr>
              </thead>
              <tbody>
                {#snippet renderRowCells(wt: WorktreeInfo, disabled: boolean)}
                  <td class="col-check">
                    {#if wt.branch && !disabled}
                      <input
                        type="checkbox"
                        checked={checked.has(wt.branch)}
                        onchange={() => toggleCheck(wt.branch!)}
                      />
                    {:else}
                      <input type="checkbox" disabled checked={false} />
                    {/if}
                  </td>
                  <td class="col-safety">
                    <span class="safety-dot {safetyDotClass(wt.safety_level)}"></span>
                  </td>
                  <td class="col-branch mono">
                    {#if wt.branch && agentTabBranchSet.has(normalizeTabBranch(wt.branch))}
                      <span class="agent-tab-icon" title="Agent tab is open for this worktree">
                        @
                      </span>
                    {/if}
                    {wt.branch ?? "(detached)"}
                  </td>
                  <td class="col-status">{wt.status}</td>
                  <td class="col-markers">
                    {#if wt.has_changes}
                      <span class="marker marker-changes" title="Uncommitted changes">M</span>
                    {/if}
                    {#if wt.has_unpushed}
                      <span class="marker marker-unpushed" title="Unpushed commits">U</span>
                    {/if}
                  </td>
                  <td class="col-sync">
                    {#if wt.ahead > 0}
                      <span class="sync-ahead">+{wt.ahead}</span>
                    {/if}
                    {#if wt.behind > 0}
                      <span class="sync-behind">-{wt.behind}</span>
                    {/if}
                  </td>
                  <td class="col-gone">
                    {#if wt.is_gone}
                      <span class="gone-badge">gone</span>
                    {/if}
                  </td>
                  <td class="col-tool">
                    {#if wt.last_tool_usage}
                      <span class="tool-label">{wt.last_tool_usage}</span>
                    {/if}
                  </td>
                {/snippet}
                {#if flipEnabled}
                  {#each sortedWorktrees as wt (wt.path)}
                    {@const disabled = isDisabled(wt)}
                    <tr class:disabled animate:flip={{ duration: 220 }}>
                      {@render renderRowCells(wt, disabled)}
                    </tr>
                  {/each}
                {:else}
                  {#each sortedWorktrees as wt (wt.path)}
                    {@const disabled = isDisabled(wt)}
                    <tr class:disabled>
                      {@render renderRowCells(wt, disabled)}
                    </tr>
                  {/each}
                {/if}
              </tbody>
            </table>
          </div>
        </div>

        <div class="dialog-footer">
          <button class="btn btn-cancel" onclick={onClose}>Cancel</button>
          <button
            class="btn btn-cleanup"
            disabled={checkedCount === 0 || cleaning}
            onclick={handleCleanup}
          >
            {cleaning ? "Cleaning..." : `Cleanup (${checkedCount})`}
          </button>
        </div>
      {/if}
    </div>

    {#if confirmMode}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="overlay confirm-overlay"
        onclick={(e) => e.stopPropagation()}
      >
        <div class="confirm-dialog">
          {#if confirmMode === "both"}
            <h3>Active Tabs and Unsafe Worktrees Selected</h3>
            <p>
              {unsafeCheckedCount} unsafe worktree{unsafeCheckedCount > 1 ? "s" : ""}
              and {activeTabCheckedCount} worktree{activeTabCheckedCount > 1 ? "s" : ""}
              with an open agent tab selected. Uncommitted changes or unpushed commits may be
              lost, and active sessions may break. Continue?
            </p>
          {:else if confirmMode === "unsafe"}
            <h3>Unsafe Worktrees Selected</h3>
            <p>
              {unsafeCheckedCount} unsafe worktree{unsafeCheckedCount > 1 ? "s" : ""}
              selected. These have uncommitted changes or unpushed commits that will be lost.
              Continue?
            </p>
          {:else}
            <h3>Active Agent Tabs Detected</h3>
            <p>
              {activeTabCheckedCount} selected worktree{activeTabCheckedCount > 1 ? "s" : ""}
              have an open agent tab. Cleaning them up may break the active session. Continue?
            </p>
          {/if}
          <div class="confirm-actions">
            <button class="btn btn-cancel" onclick={cancelConfirm}>
              Cancel
            </button>
            <button class="btn btn-danger" onclick={confirmCleanup}>
              {confirmMode === "active" ? "Continue" : "Force Cleanup"}
            </button>
          </div>
        </div>
      </div>
    {/if}
  </div>
{/if}

{#if showFailures && failures.length > 0}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    class="overlay"
    onclick={closeFailures}
    role="dialog"
    aria-modal="true"
    aria-label="Cleanup Failures"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="dialog" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-header">
        <h2>Cleanup Failed</h2>
        <button class="close-btn" onclick={closeFailures}>[x]</button>
      </div>

      <div class="dialog-body">
        <p class="failure-summary">
          {failures.length} worktree{failures.length > 1 ? "s" : ""} failed to
          delete:
        </p>
        <div class="failure-list">
          {#each failures as f (f.branch)}
            <div class="failure-item">
              <span class="mono">{f.branch}</span>
              <span class="failure-error">{f.error ?? "Unknown error"}</span>
            </div>
          {/each}
        </div>
      </div>

      <div class="dialog-footer">
        <button class="btn btn-cancel" onclick={closeFailures}>Close</button>
      </div>
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
    z-index: 1000;
  }

  .confirm-overlay {
    z-index: 1100;
  }

  .dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    width: 780px;
    max-width: 94vw;
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
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
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

  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .toolbar-btn {
    padding: 6px 14px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
  }

  .toolbar-btn:hover {
    border-color: var(--accent);
    background: var(--bg-hover);
  }

  .toolbar-count {
    font-size: 12px;
    color: var(--text-muted);
  }

  .table-wrapper {
    overflow-x: auto;
  }

  .wt-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .wt-table th {
    text-align: left;
    padding: 6px 8px;
    color: var(--text-muted);
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    border-bottom: 1px solid var(--border-color);
  }

  .wt-table td {
    padding: 7px 8px;
    border-bottom: 1px solid rgba(69, 71, 90, 0.4);
    vertical-align: middle;
  }

  .wt-table tbody tr:hover:not(.disabled) {
    background: var(--bg-hover);
  }

  .wt-table tbody tr.disabled {
    opacity: 0.45;
  }

  .wt-table tbody tr.disabled td {
    color: var(--text-muted);
  }

  .col-check {
    width: 32px;
    text-align: center;
  }

  .col-check input[type="checkbox"] {
    accent-color: var(--accent);
  }

  .col-safety {
    width: 24px;
    text-align: center;
  }

  .safety-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .dot-safe {
    background: var(--green);
  }

  .dot-warning {
    background: var(--yellow);
  }

  .dot-danger {
    background: var(--red);
  }

  .dot-disabled {
    background: var(--text-muted);
  }

  .col-branch {
    min-width: 120px;
  }

  .col-status,
  .col-markers,
  .col-sync,
  .col-gone,
  .col-tool {
    white-space: nowrap;
  }

  .mono {
    font-family: monospace;
  }

  .marker {
    display: inline-block;
    font-family: monospace;
    font-size: 11px;
    font-weight: 700;
    padding: 0 3px;
    border-radius: 3px;
    margin-right: 2px;
  }

  .marker-changes {
    color: var(--red);
  }

  .marker-unpushed {
    color: var(--yellow);
  }

  .sync-ahead {
    color: var(--green);
    font-family: monospace;
    font-size: 11px;
    margin-right: 4px;
  }

  .sync-behind {
    color: var(--yellow);
    font-family: monospace;
    font-size: 11px;
  }

  .gone-badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: rgba(243, 139, 168, 0.15);
    color: var(--red);
    border: 1px solid rgba(243, 139, 168, 0.3);
  }

  .agent-tab-icon {
    display: inline-block;
    width: 12px;
    margin-right: 4px;
    color: var(--cyan);
    font-family: monospace;
    text-align: center;
  }

  .tool-label {
    font-size: 10px;
    font-family: monospace;
    color: var(--text-muted);
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
  }

  .btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
  }

  .btn-cleanup {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .btn-cleanup:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-cleanup:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-danger {
    background: var(--red);
    color: var(--bg-primary);
  }

  .btn-danger:hover {
    background: #e77a96;
  }

  .confirm-dialog {
    background: var(--bg-secondary);
    border: 1px solid rgba(243, 139, 168, 0.4);
    border-radius: 12px;
    padding: 24px 28px;
    max-width: 440px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
  }

  .confirm-dialog h3 {
    font-size: 15px;
    font-weight: 600;
    color: var(--red);
    margin-bottom: 10px;
  }

  .confirm-dialog p {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin-bottom: 18px;
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .failure-summary {
    font-size: 13px;
    color: var(--text-secondary);
    margin-bottom: 4px;
  }

  .failure-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .failure-item {
    padding: 10px 12px;
    border: 1px solid rgba(243, 139, 168, 0.3);
    background: rgba(243, 139, 168, 0.06);
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .failure-error {
    font-size: 11px;
    color: var(--red);
    line-height: 1.4;
  }
</style>
