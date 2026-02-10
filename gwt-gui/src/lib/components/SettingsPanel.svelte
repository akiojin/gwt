<script lang="ts">
  import type { ProfilesConfig, Profile, SettingsData } from "../types";

  let { onClose }: { onClose: () => void } = $props();

  let settings: SettingsData | null = $state(null);
  let profiles: ProfilesConfig | null = $state(null);

  let loadingSettings: boolean = $state(true);
  let loadingProfiles: boolean = $state(true);
  let saving: boolean = $state(false);
  let errorMessage: string | null = $state(null);
  let newBranch: string = $state("");
  let saveMessage: string = $state("");

  let selectedProfileKey: string = $state("");
  let newProfileName: string = $state("");
  let newEnvKey: string = $state("");
  let newEnvValue: string = $state("");

  let savedUiFontSize: number = $state(13);
  let savedTerminalFontSize: number = $state(13);

  function getCurrentProfile(cfg: ProfilesConfig | null, key: string): Profile | null {
    if (!cfg) return null;
    if (!key) return null;
    const p = cfg.profiles?.[key];
    return p ?? null;
  }

  let currentProfile = $derived(getCurrentProfile(profiles, selectedProfileKey));

  $effect(() => {
    loadAll();
  });

  $effect(() => {
    if (!settings) return;
    applyUiFontSize(settings.ui_font_size ?? 13);
    applyTerminalFontSize(settings.terminal_font_size ?? 13);
  });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  async function loadAll() {
    errorMessage = null;
    loadingSettings = true;
    loadingProfiles = true;

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const [loadedSettings, loadedProfiles] = await Promise.all([
        invoke<SettingsData>("get_settings"),
        invoke<ProfilesConfig>("get_profiles"),
      ]);
      settings = loadedSettings;
      savedUiFontSize = loadedSettings.ui_font_size ?? 13;
      savedTerminalFontSize = loadedSettings.terminal_font_size ?? 13;
      profiles = loadedProfiles;

      const keys = Object.keys(loadedProfiles.profiles ?? {});
      const nextSelected = loadedProfiles.active ?? keys[0] ?? "";
      selectedProfileKey = nextSelected;
    } catch (err) {
      console.error("Failed to load settings/profiles:", err);
      errorMessage = `Failed to load settings: ${toErrorMessage(err)}`;
    }

    loadingSettings = false;
    loadingProfiles = false;
  }

  async function saveAll() {
    if (!settings) return;
    saving = true;
    saveMessage = "";
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("save_settings", { settings });
      if (profiles) {
        await invoke("save_profiles", { config: profiles });
      }
      saveMessage = "Settings saved.";
      savedUiFontSize = settings.ui_font_size ?? 13;
      savedTerminalFontSize = settings.terminal_font_size ?? 13;
    } catch (err) {
      console.error("Failed to save settings/profiles:", err);
      saveMessage = `Failed to save settings: ${toErrorMessage(err)}`;
    }
    saving = false;
    setTimeout(() => {
      saveMessage = "";
    }, 2000);
  }

  function addBranch() {
    const trimmed = newBranch.trim();
    if (settings && trimmed && !settings.protected_branches.includes(trimmed)) {
      settings.protected_branches = [...settings.protected_branches, trimmed];
      newBranch = "";
    }
  }

  function removeBranch(branch: string) {
    if (!settings) return;
    settings.protected_branches = settings.protected_branches.filter((b) => b !== branch);
  }

  function handleBranchKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      addBranch();
    }
  }

  function sortedProfileKeys(cfg: ProfilesConfig): string[] {
    const keys = Object.keys(cfg.profiles ?? {});
    keys.sort((a, b) => a.localeCompare(b));
    return keys;
  }

  function clampFontSize(v: number): number {
    return Math.max(8, Math.min(24, Math.round(v)));
  }

  function applyUiFontSize(size: number) {
    document.documentElement.style.setProperty("--ui-font-base", size + "px");
  }

  function applyTerminalFontSize(size: number) {
    window.dispatchEvent(new CustomEvent("gwt-terminal-font-size", { detail: size }));
  }

  function adjustFontSize(field: "ui_font_size" | "terminal_font_size", delta: number) {
    if (!settings) return;
    const current = settings[field] ?? 13;
    const next = clampFontSize(current + delta);
    settings = { ...settings, [field]: next };
  }

  function handleClose() {
    applyUiFontSize(savedUiFontSize);
    applyTerminalFontSize(savedTerminalFontSize);
    onClose();
  }

  function setActiveProfile(name: string | null) {
    if (!profiles) return;
    profiles = { ...profiles, active: name };
    selectedProfileKey = name ?? "";
  }

  function createProfile() {
    if (!profiles) return;
    const name = newProfileName.trim();
    if (!name) return;
    if (!/^[a-z0-9-]+$/.test(name)) {
      saveMessage = "Profile name must be lowercase letters, numbers, or hyphens.";
      return;
    }
    if (profiles.profiles?.[name]) {
      saveMessage = "Profile already exists.";
      return;
    }

    const next: Profile = {
      name,
      description: "",
      env: {},
      disabled_env: [],
      ai: null,
    };

    profiles = {
      ...profiles,
      profiles: { ...profiles.profiles, [name]: next },
      active: profiles.active ?? name,
    };
    selectedProfileKey = name;
    newProfileName = "";
  }

  function deleteSelectedProfile() {
    if (!profiles) return;
    if (!selectedProfileKey) return;
    const copy = { ...(profiles.profiles ?? {}) };
    if (!copy[selectedProfileKey]) return;
    delete copy[selectedProfileKey];
    const nextKeys = Object.keys(copy).sort((a, b) => a.localeCompare(b));
    const nextActive =
      profiles.active === selectedProfileKey ? (nextKeys[0] ?? null) : profiles.active ?? null;
    profiles = { ...profiles, profiles: copy, active: nextActive };
    selectedProfileKey = nextActive ?? "";
  }

  function upsertEnvVar(key: string, value: string) {
    if (!profiles) return;
    const p = currentProfile;
    if (!p) return;
    if (!selectedProfileKey) return;
    const nextEnv = { ...(p.env ?? {}), [key]: value };
    const nextProfile: Profile = { ...p, env: nextEnv };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }

  function removeEnvVar(key: string) {
    if (!profiles) return;
    const p = currentProfile;
    if (!p) return;
    if (!selectedProfileKey) return;
    const nextEnv = { ...(p.env ?? {}) };
    delete nextEnv[key];
    const nextProfile: Profile = { ...p, env: nextEnv };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }

  function addEnvVar() {
    const key = newEnvKey.trim();
    if (!key) return;
    upsertEnvVar(key, newEnvValue);
    newEnvKey = "";
    newEnvValue = "";
  }

  function setAiEnabled(enabled: boolean) {
    if (!profiles) return;
    const p = currentProfile;
    if (!p) return;
    if (!selectedProfileKey) return;
    const nextProfile: Profile = enabled
      ? {
          ...p,
          ai: p.ai ?? {
            endpoint: "https://api.openai.com/v1",
            api_key: "",
            model: "",
            summary_enabled: true,
          },
        }
      : { ...p, ai: null };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }

  function updateAiField(field: "endpoint" | "api_key" | "model" | "summary_enabled", value: string | boolean) {
    if (!profiles) return;
    const p = currentProfile;
    if (!p || !p.ai) return;
    if (!selectedProfileKey) return;
    const nextAi = { ...p.ai, [field]: value } as Profile["ai"];
    const nextProfile: Profile = { ...p, ai: nextAi };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }
</script>

<div class="settings-panel">
  <div class="settings-header">
    <h2>Settings</h2>
    <button class="close-btn" onclick={handleClose}>[x]</button>
  </div>

  {#if loadingSettings || loadingProfiles}
    <div class="loading">Loading settings...</div>
  {:else if errorMessage || !settings}
    <div class="loading">{errorMessage ?? "Failed to load settings."}</div>
  {:else}
    <div class="settings-body">
      <div class="section-title">Appearance</div>

      <div class="field">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>Terminal Font Size</label>
        <div class="font-size-control">
          <button
            class="font-size-btn"
            onclick={() => adjustFontSize("terminal_font_size", -1)}
            disabled={!settings || (settings.terminal_font_size ?? 13) <= 8}
          >-</button>
          <input
            type="number"
            min="8"
            max="24"
            step="1"
            value={settings.terminal_font_size ?? 13}
            oninput={(e) => {
              if (!settings) return;
              settings = { ...settings, terminal_font_size: clampFontSize(Number((e.target as HTMLInputElement).value) || 13) };
            }}
          />
          <button
            class="font-size-btn"
            onclick={() => adjustFontSize("terminal_font_size", 1)}
            disabled={!settings || (settings.terminal_font_size ?? 13) >= 24}
          >+</button>
          <span class="font-size-unit">px</span>
        </div>
      </div>

      <div class="field">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>UI Font Size</label>
        <div class="font-size-control">
          <button
            class="font-size-btn"
            onclick={() => adjustFontSize("ui_font_size", -1)}
            disabled={!settings || (settings.ui_font_size ?? 13) <= 8}
          >-</button>
          <input
            type="number"
            min="8"
            max="24"
            step="1"
            value={settings.ui_font_size ?? 13}
            oninput={(e) => {
              if (!settings) return;
              settings = { ...settings, ui_font_size: clampFontSize(Number((e.target as HTMLInputElement).value) || 13) };
            }}
          />
          <button
            class="font-size-btn"
            onclick={() => adjustFontSize("ui_font_size", 1)}
            disabled={!settings || (settings.ui_font_size ?? 13) >= 24}
          >+</button>
          <span class="font-size-unit">px</span>
        </div>
      </div>

      <div class="divider"></div>

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

      <div class="divider"></div>

      <div class="section-title">Profiles</div>
      <div class="field">
        <label for="active-profile">Active Profile</label>
        <select
          id="active-profile"
          class="select"
          value={profiles?.active ?? ""}
          onchange={(e) => setActiveProfile((e.target as HTMLSelectElement).value || null)}
        >
          <option value="">(none)</option>
          {#if profiles}
            {#each sortedProfileKeys(profiles) as key}
              <option value={key}>{key}</option>
            {/each}
          {/if}
        </select>
        <span class="field-hint">Saved in ~/.gwt/profiles.toml</span>
      </div>

      <div class="field">
        <label for="profile-edit">Edit Profile</label>
        <div class="row">
          <select
            id="profile-edit"
            class="select"
            bind:value={selectedProfileKey}
            disabled={!profiles}
          >
            {#if profiles}
              {#each sortedProfileKeys(profiles) as key}
                <option value={key}>{key}</option>
              {/each}
            {/if}
          </select>
          <button class="btn btn-danger" onclick={deleteSelectedProfile} disabled={!profiles || !selectedProfileKey}>
            Delete
          </button>
        </div>
      </div>

      <div class="field">
        <label for="new-profile">New Profile</label>
        <div class="row">
          <input
            id="new-profile"
            type="text"
            bind:value={newProfileName}
            placeholder="e.g. development"
          />
          <button class="btn btn-add" onclick={createProfile} disabled={!profiles || !newProfileName.trim()}>
            Create
          </button>
        </div>
        <span class="field-hint">Name must be lowercase letters, numbers, or hyphens.</span>
      </div>

      <div class="field">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>Environment Variables</label>
        {#if profiles && selectedProfileKey && currentProfile}
          <div class="env-table">
            {#each Object.keys(currentProfile.env ?? {}).sort((a, b) => a.localeCompare(b)) as key (key)}
              <div class="env-row">
                <span class="env-key mono">{key}</span>
                <input
                  class="env-value"
                  type="text"
                  value={currentProfile.env[key]}
                  oninput={(e) => upsertEnvVar(key, (e.target as HTMLInputElement).value)}
                />
                <button class="btn btn-ghost" onclick={() => removeEnvVar(key)}>Remove</button>
              </div>
            {/each}
          </div>

          <div class="env-add-row">
            <input
              class="env-key-input"
              type="text"
              bind:value={newEnvKey}
              placeholder="KEY"
            />
            <input
              class="env-value-input"
              type="text"
              bind:value={newEnvValue}
              placeholder="value"
            />
            <button class="btn btn-add" onclick={addEnvVar} disabled={!newEnvKey.trim()}>
              Add
            </button>
          </div>
        {:else}
          <div class="field-hint">Create a profile to edit environment variables.</div>
        {/if}
      </div>

      <div class="field">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>AI Settings (per profile)</label>
        {#if profiles && selectedProfileKey && currentProfile}
          <div class="ai-toggle">
            <input
              id="ai-enabled"
              type="checkbox"
              checked={!!currentProfile.ai}
              onchange={(e) => setAiEnabled((e.target as HTMLInputElement).checked)}
            />
            <label for="ai-enabled" class="ai-enabled-label">Enable AI settings</label>
          </div>

          {#if currentProfile.ai}
            <div class="ai-grid">
              <div class="ai-field">
                <span class="ai-label">Endpoint</span>
                <input
                  type="text"
                  value={currentProfile.ai.endpoint}
                  oninput={(e) => updateAiField("endpoint", (e.target as HTMLInputElement).value)}
                />
              </div>
              <div class="ai-field">
                <span class="ai-label">API Key</span>
                <input
                  type="text"
                  value={currentProfile.ai.api_key}
                  oninput={(e) => updateAiField("api_key", (e.target as HTMLInputElement).value)}
                />
              </div>
              <div class="ai-field">
                <span class="ai-label">Model</span>
                <input
                  type="text"
                  value={currentProfile.ai.model}
                  placeholder="e.g. gpt-5.2-codex"
                  oninput={(e) => updateAiField("model", (e.target as HTMLInputElement).value)}
                />
              </div>
              <div class="ai-field">
                <span class="ai-label">Session Summary</span>
                <div class="ai-checkbox">
                  <input
                    id="ai-summary"
                    type="checkbox"
                    checked={currentProfile.ai.summary_enabled}
                    onchange={(e) => updateAiField("summary_enabled", (e.target as HTMLInputElement).checked)}
                  />
                  <label for="ai-summary">Enabled</label>
                </div>
              </div>
            </div>
          {/if}
        {:else}
          <div class="field-hint">Create a profile to configure AI settings.</div>
        {/if}
      </div>
    </div>

    <div class="settings-footer">
      {#if saveMessage}
        <span class="save-message">{saveMessage}</span>
      {/if}
      <button class="btn btn-cancel" onclick={handleClose}>Close</button>
      <button
        class="btn btn-save"
        disabled={saving || !settings}
        onclick={saveAll}
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
    font-size: var(--ui-font-xl);
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: var(--ui-font-lg);
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

  .divider {
    height: 1px;
    background: var(--border-color);
    opacity: 0.7;
  }

  .section-title {
    font-size: var(--ui-font-md);
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: 0.6px;
    text-transform: uppercase;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    font-size: var(--ui-font-md);
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
    font-size: var(--ui-font-base);
    font-family: monospace;
    outline: none;
    max-width: 200px;
  }

  .select {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-base);
    font-family: monospace;
    outline: none;
    max-width: 320px;
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }

  .env-table {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
  }

  .env-row {
    display: grid;
    grid-template-columns: 1fr 2fr auto;
    gap: 8px;
    align-items: center;
  }

  .env-key {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .env-value,
  .env-key-input,
  .env-value-input {
    padding: 6px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: monospace;
    outline: none;
    width: 100%;
  }

  .env-add-row {
    display: grid;
    grid-template-columns: 1fr 2fr auto;
    gap: 8px;
    align-items: center;
    max-width: 760px;
  }

  .ai-toggle {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    width: fit-content;
  }

  .ai-enabled-label {
    font-size: var(--ui-font-md);
    color: var(--text-secondary);
  }

  .ai-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-top: 10px;
    max-width: 760px;
  }

  .ai-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .ai-label {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .ai-field input[type="text"] {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: monospace;
    outline: none;
    max-width: none;
  }

  .ai-checkbox {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    width: fit-content;
  }

  .field input:focus {
    border-color: var(--accent);
  }

  .field-hint {
    font-size: var(--ui-font-sm);
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
    font-size: var(--ui-font-md);
    font-family: monospace;
    color: var(--text-primary);
  }

  .tag-remove {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: var(--ui-font-sm);
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
    font-size: var(--ui-font-md);
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
    font-size: var(--ui-font-md);
    color: var(--green);
    margin-right: auto;
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: var(--ui-font-base);
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
    transition: background-color 0.15s;
  }

  .btn-add {
    padding: 6px 12px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
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

  .btn-danger {
    background: var(--red);
    color: var(--bg-primary);
  }

  .btn-danger:hover:not(:disabled) {
    filter: brightness(1.05);
  }

  .btn-ghost {
    background: none;
    border: 1px solid var(--border-color);
    color: var(--text-secondary);
    padding: 6px 10px;
    font-size: var(--ui-font-md);
  }

  .btn-ghost:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .mono {
    font-family: monospace;
  }

  .font-size-control {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 200px;
  }

  .font-size-control input[type="number"] {
    width: 60px;
    text-align: center;
    padding: 6px 4px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-base);
    font-family: monospace;
    outline: none;
    -moz-appearance: textfield;
  }

  .font-size-control input[type="number"]::-webkit-inner-spin-button,
  .font-size-control input[type="number"]::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .font-size-control input[type="number"]:focus {
    border-color: var(--accent);
  }

  .font-size-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-lg);
    font-family: monospace;
    cursor: pointer;
    flex-shrink: 0;
  }

  .font-size-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    border-color: var(--accent);
  }

  .font-size-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .font-size-unit {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  @media (max-width: 800px) {
    .env-row,
    .env-add-row {
      grid-template-columns: 1fr;
    }
    .ai-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
