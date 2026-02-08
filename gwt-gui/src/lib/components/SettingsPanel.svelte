<script lang="ts">
  import type { SettingsData } from "../types";

  let { onClose }: { onClose: () => void } = $props();

  let settings: SettingsData = $state({
    log_retention_days: 30,
    protected_branches: ["main", "develop"],
  });
  let loading: boolean = $state(true);
  let saving: boolean = $state(false);
  let newBranch: string = $state("");
  let saveMessage: string = $state("");

  $effect(() => {
    loadSettings();
  });

  async function loadSettings() {
    loading = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      settings = await invoke<SettingsData>("get_settings");
    } catch {
      // Dev mode fallback - use defaults
    }
    loading = false;
  }

  async function saveSettings() {
    saving = true;
    saveMessage = "";
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("save_settings", { settings });
      saveMessage = "Settings saved.";
    } catch {
      saveMessage = "Saved (dev mode).";
    }
    saving = false;
    setTimeout(() => {
      saveMessage = "";
    }, 2000);
  }

  function addBranch() {
    const trimmed = newBranch.trim();
    if (trimmed && !settings.protected_branches.includes(trimmed)) {
      settings.protected_branches = [...settings.protected_branches, trimmed];
      newBranch = "";
    }
  }

  function removeBranch(branch: string) {
    settings.protected_branches = settings.protected_branches.filter(
      (b) => b !== branch
    );
  }

  function handleBranchKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      addBranch();
    }
  }
</script>

<div class="settings-panel">
  <div class="settings-header">
    <h2>Settings</h2>
    <button class="close-btn" onclick={onClose}>[x]</button>
  </div>

  {#if loading}
    <div class="loading">Loading settings...</div>
  {:else}
    <div class="settings-body">
      <div class="field">
        <label for="log-retention">Log Retention (days)</label>
        <input
          id="log-retention"
          type="number"
          min="1"
          max="365"
          bind:value={settings.log_retention_days}
        />
        <span class="field-hint">
          Logs older than this will be cleaned up automatically.
        </span>
      </div>

      <div class="field">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>Protected Branches</label>
        <div class="branch-tags">
          {#each settings.protected_branches as branch}
            <span class="branch-tag">
              {branch}
              <button class="tag-remove" onclick={() => removeBranch(branch)}>
                x
              </button>
            </span>
          {/each}
        </div>
        <div class="branch-input-row">
          <input
            type="text"
            bind:value={newBranch}
            placeholder="Add branch..."
            onkeydown={handleBranchKeydown}
          />
          <button class="btn btn-add" onclick={addBranch}>Add</button>
        </div>
        <span class="field-hint">
          Branches that cannot be deleted or force-pushed.
        </span>
      </div>
    </div>

    <div class="settings-footer">
      {#if saveMessage}
        <span class="save-message">{saveMessage}</span>
      {/if}
      <button class="btn btn-cancel" onclick={onClose}>Close</button>
      <button
        class="btn btn-save"
        disabled={saving}
        onclick={saveSettings}
      >
        {saving ? "Saving..." : "Save"}
      </button>
    </div>
  {/if}
</div>

<style>
  .settings-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-primary);
  }

  .settings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 24px;
    border-bottom: 1px solid var(--border-color);
  }

  .settings-header h2 {
    font-size: 16px;
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

  .settings-body {
    flex: 1;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 24px;
    overflow-y: auto;
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

  .field input[type="number"],
  .field input[type="text"] {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: 13px;
    font-family: monospace;
    outline: none;
    max-width: 200px;
  }

  .field input:focus {
    border-color: var(--accent);
  }

  .field-hint {
    font-size: 11px;
    color: var(--text-muted);
  }

  .branch-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .branch-tag {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    font-size: 12px;
    font-family: monospace;
    color: var(--text-primary);
  }

  .tag-remove {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 11px;
    font-family: monospace;
    padding: 0 2px;
    line-height: 1;
  }

  .tag-remove:hover {
    color: var(--red);
  }

  .branch-input-row {
    display: flex;
    gap: 6px;
    max-width: 300px;
  }

  .branch-input-row input {
    flex: 1;
    padding: 6px 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: 12px;
    font-family: monospace;
    outline: none;
  }

  .branch-input-row input:focus {
    border-color: var(--accent);
  }

  .settings-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 24px;
    border-top: 1px solid var(--border-color);
  }

  .save-message {
    font-size: 12px;
    color: var(--green);
    margin-right: auto;
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

  .btn-add {
    padding: 6px 12px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: 12px;
  }

  .btn-add:hover {
    background: var(--bg-hover);
  }

  .btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
  }

  .btn-save {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .btn-save:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-save:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
