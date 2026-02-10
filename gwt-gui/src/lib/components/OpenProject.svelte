<script lang="ts">
  import type { CloneProgress, ProbePathResult, ProjectInfo } from "../types";
  import MigrationModal from "./MigrationModal.svelte";

  interface RecentProject {
    path: string;
    name: string;
    lastOpened: string;
  }

  let { onOpen }: { onOpen: (path: string) => void } = $props();

  let recentProjects: RecentProject[] = $state([]);
  let opening: boolean = $state(false);
  let creating: boolean = $state(false);
  let errorMessage: string | null = $state(null);

  let showNewProject: boolean = $state(false);
  let repoUrl: string = $state("");
  let parentDir: string = $state("");
  let shallowClone: boolean = $state(true);
  let cloneProgress: CloneProgress | null = $state(null);

  let migrationOpen: boolean = $state(false);
  let migrationSourceRoot: string = $state("");

  $effect(() => {
    loadRecentProjects();
  });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    try {
      return JSON.stringify(err);
    } catch {
      return String(err);
    }
  }

  function normalizeOpenProjectError(msg: string): string {
    if (msg.includes("Migration required")) return "Migration required.";
    if (msg.includes("Path does not exist")) return "Path does not exist.";
    if (msg.includes("Not a git repository")) return "Not a git repository.";
    if (msg.includes("Not a gwt project")) return "Not a gwt project.";
    return msg;
  }

  function normalizeProbeError(probe: ProbePathResult): string {
    if (probe.kind === "notFound") return "Path does not exist.";
    if (probe.kind === "invalid") return "Invalid path.";
    if (probe.kind === "notGwtProject") return "Not a gwt project.";
    return probe.message || "Failed to open project.";
  }

  async function loadRecentProjects() {
    try {
      const { load } = await import("@tauri-apps/plugin-store");
      const store = await load("recent-projects.json", { defaults: {} });
      const saved = await store.get<RecentProject[]>("projects");
      if (saved) {
        recentProjects = saved;
      }
    } catch (err) {
      console.error("Failed to load recent projects:", err);
      recentProjects = [];
    }
  }

  async function saveToRecent(path: string) {
    const name = path.split("/").pop() || path;
    const now = new Date().toLocaleDateString();
    const entry: RecentProject = { path, name, lastOpened: now };

    // Remove duplicate, add to front
    const filtered = recentProjects.filter((p) => p.path !== path);
    const updated = [entry, ...filtered].slice(0, 10);

    try {
      const { load } = await import("@tauri-apps/plugin-store");
      const store = await load("recent-projects.json", { defaults: {} });
      await store.set("projects", updated);
      await store.save();
    } catch {
      // Dev mode: ignore
    }

    recentProjects = updated;
  }

  async function removeFromRecent(path: string) {
    const updated = recentProjects.filter((p) => p.path !== path);
    try {
      const { load } = await import("@tauri-apps/plugin-store");
      const store = await load("recent-projects.json", { defaults: {} });
      await store.set("projects", updated);
      await store.save();
    } catch {
      // Dev mode: ignore
    }
    recentProjects = updated;
  }

  async function openFolder() {
    opening = true;
    errorMessage = null;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        await probeAndOpen(selected as string, false);
      }
    } catch (err) {
      errorMessage = `Failed to open folder dialog: ${toErrorMessage(err)}`;
    }
    opening = false;
  }

  async function openGwtProject(projectPath: string, fromRecent: boolean) {
    opening = true;
    errorMessage = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const info = await invoke<ProjectInfo>("open_project", { path: projectPath });
      await saveToRecent(info.path);
      onOpen(info.path);
    } catch (err) {
      const msg = toErrorMessage(err);
      if (fromRecent && msg.includes("Path does not exist")) {
        await removeFromRecent(projectPath);
      }
      errorMessage = normalizeOpenProjectError(msg);
    } finally {
      opening = false;
    }
  }

  async function probeAndOpen(path: string, fromRecent: boolean) {
    opening = true;
    errorMessage = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const probe = await invoke<ProbePathResult>("probe_path", { path });

      if (probe.kind === "gwtProject" && probe.project_path) {
        await openGwtProject(probe.project_path, fromRecent);
        return;
      }

      if (probe.kind === "migrationRequired" && probe.migration_source_root) {
        migrationSourceRoot = probe.migration_source_root;
        migrationOpen = true;
        return;
      }

      if (probe.kind === "emptyDir" && probe.project_path) {
        showNewProject = true;
        parentDir = probe.project_path;
        return;
      }

      if (fromRecent && probe.kind === "notFound") {
        await removeFromRecent(path);
      }

      errorMessage = normalizeProbeError(probe);
    } catch (err) {
      const msg = toErrorMessage(err);
      if (fromRecent && msg.includes("Path does not exist")) {
        await removeFromRecent(path);
      }
      errorMessage = normalizeOpenProjectError(msg);
    } finally {
      opening = false;
    }
  }

  async function chooseParentDir() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        parentDir = selected as string;
      }
    } catch (err) {
      errorMessage = `Failed to open folder dialog: ${toErrorMessage(err)}`;
    }
  }

  function progressLabel(p: CloneProgress | null): string {
    if (!p) return "";
    if (p.stage === "receiving") return "Receiving objects";
    if (p.stage === "resolving") return "Resolving deltas";
    return "Cloning";
  }

  async function createProject() {
    if (creating) return;

    errorMessage = null;
    cloneProgress = null;
    creating = true;

    let unlisten: null | (() => void) = null;
    try {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen<CloneProgress>("clone-progress", (event) => {
        cloneProgress = event.payload;
      });

      const { invoke } = await import("@tauri-apps/api/core");
      const info = await invoke<ProjectInfo>("create_project", {
        request: { repoUrl, parentDir, shallow: shallowClone },
      });
      await saveToRecent(info.path);
      onOpen(info.path);
    } catch (err) {
      const msg = toErrorMessage(err);
      errorMessage = msg.includes("Invalid repository URL")
        ? "Invalid repository URL"
        : `Failed to create project: ${msg}`;
    } finally {
      if (unlisten) {
        unlisten();
      }
      creating = false;
    }
  }
</script>

<div class="open-project">
  <div class="content">
    <h1 class="title">gwt</h1>
    <p class="subtitle">Git Worktree Manager</p>

    <div class="actions">
      <button
        class="open-btn"
        onclick={openFolder}
        disabled={opening || creating}
      >
        {opening ? "Opening..." : "Open Project..."}
      </button>
      <button
        class="new-btn"
        onclick={() => (showNewProject = !showNewProject)}
        disabled={opening || creating}
      >
        New Project
      </button>
    </div>

    {#if errorMessage}
      <div class="error">{errorMessage}</div>
    {/if}

    {#if showNewProject}
      <div class="new-project">
        <h3>New Project</h3>
        <label class="field">
          <span class="label">Repository URL</span>
          <input
            class="input"
            type="text"
            placeholder="https://github.com/owner/repo"
            bind:value={repoUrl}
            disabled={creating}
          />
        </label>

        <div class="field">
          <span class="label">Parent Directory</span>
          <div class="row">
            <input
              class="input"
              type="text"
              placeholder="Choose a folder..."
              value={parentDir}
              readonly
              disabled={creating}
            />
            <button
              class="choose-btn"
              onclick={chooseParentDir}
              disabled={creating}
            >
              Choose...
            </button>
          </div>
        </div>

        <div class="field">
          <span class="label">Clone Mode</span>
          <div class="row">
            <button
              class="mode-btn"
              class:active={shallowClone}
              type="button"
              onclick={() => (shallowClone = true)}
              disabled={creating}
            >
              Shallow (Recommended)
            </button>
            <button
              class="mode-btn"
              class:active={!shallowClone}
              type="button"
              onclick={() => (shallowClone = false)}
              disabled={creating}
            >
              Full
            </button>
          </div>
          <span class="mode-hint">
            Shallow clone uses --depth=1 (faster).
          </span>
        </div>

        <button
          class="create-btn"
          onclick={createProject}
          disabled={creating || !repoUrl || !parentDir}
        >
          {creating ? "Creating..." : "Create"}
        </button>

        {#if cloneProgress}
          <div class="progress">
            <div class="progress-row">
              <span class="progress-label">{progressLabel(cloneProgress)}</span>
              <span class="progress-percent">{cloneProgress.percent}%</span>
            </div>
            <div class="progress-bar">
              <div
                class="progress-fill"
                style={`width: ${cloneProgress.percent}%`}
              ></div>
            </div>
          </div>
        {/if}
      </div>
    {/if}

    {#if recentProjects.length > 0}
      <div class="recent">
        <h3>Recent Projects</h3>
        {#each recentProjects as project}
          <button
            class="recent-item"
            onclick={() => probeAndOpen(project.path, true)}
            disabled={opening || creating}
          >
            <span class="recent-name">{project.name}</span>
            <span class="recent-path">{project.path}</span>
            <span class="recent-time">{project.lastOpened}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<MigrationModal
  open={migrationOpen}
  sourceRoot={migrationSourceRoot}
  onCompleted={async (p) => {
    migrationOpen = false;
    migrationSourceRoot = "";
    await openGwtProject(p, false);
  }}
  onDismiss={() => {
    migrationOpen = false;
    migrationSourceRoot = "";
  }}
/>

<style>
  .open-project {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    background-color: var(--bg-primary);
  }

  .content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    max-width: 480px;
    width: 100%;
    padding: 48px;
  }

  .title {
    font-size: 48px;
    font-weight: 700;
    color: var(--accent);
    letter-spacing: -2px;
  }

  .subtitle {
    color: var(--text-muted);
    font-size: 14px;
    margin-bottom: 24px;
  }

  .actions {
    width: 100%;
    display: flex;
    gap: 10px;
  }

  .open-btn {
    flex: 1;
    padding: 12px 24px;
    background-color: var(--accent);
    color: var(--bg-primary);
    border: none;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: background-color 0.15s;
  }

  .open-btn:hover:not(:disabled) {
    background-color: var(--accent-hover);
  }

  .open-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .new-btn {
    width: 140px;
    padding: 12px 14px;
    background: none;
    color: var(--text-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background-color 0.15s;
  }

  .new-btn:hover:not(:disabled) {
    border-color: var(--accent);
    background-color: var(--bg-surface);
  }

  .new-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .error {
    width: 100%;
    padding: 10px 12px;
    border: 1px solid rgba(255, 90, 90, 0.35);
    background: rgba(255, 90, 90, 0.08);
    color: rgb(255, 160, 160);
    border-radius: 8px;
    font-size: 12px;
    line-height: 1.4;
  }

  .new-project {
    width: 100%;
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    border-radius: 12px;
    padding: 14px 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .new-project h3 {
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin: 0;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label {
    font-size: 11px;
    color: var(--text-muted);
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .mode-btn {
    flex: 1;
    padding: 8px 10px;
    background: none;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    cursor: pointer;
    font-size: 12px;
    font-family: inherit;
    font-weight: 600;
  }

  .mode-btn:hover:not(:disabled) {
    border-color: var(--accent);
    background-color: var(--bg-surface);
  }

  .mode-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .mode-btn.active {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.06) inset;
  }

  .mode-hint {
    font-size: 11px;
    color: var(--text-muted);
    line-height: 1.4;
  }

  .input {
    width: 100%;
    padding: 8px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: 12px;
    font-family: inherit;
    outline: none;
  }

  .input:focus {
    border-color: var(--accent);
  }

  .choose-btn {
    width: 110px;
    padding: 8px 10px;
    background: none;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    cursor: pointer;
    font-size: 12px;
    font-family: inherit;
  }

  .choose-btn:hover:not(:disabled) {
    border-color: var(--accent);
    background-color: var(--bg-surface);
  }

  .choose-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .create-btn {
    width: 100%;
    padding: 10px 12px;
    background-color: var(--accent);
    color: var(--bg-primary);
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    font-family: inherit;
  }

  .create-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .progress {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 4px;
  }

  .progress-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    font-size: 11px;
    color: var(--text-muted);
  }

  .progress-label {
    color: var(--text-secondary);
  }

  .progress-percent {
    font-family: monospace;
  }

  .progress-bar {
    height: 6px;
    border-radius: 999px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: var(--accent);
  }

  .recent {
    width: 100%;
    margin-top: 16px;
  }

  .recent h3 {
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 8px;
  }

  .recent-item {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-rows: auto auto;
    gap: 2px 12px;
    width: 100%;
    padding: 10px 12px;
    background: none;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    cursor: pointer;
    text-align: left;
    margin-bottom: 4px;
    font-family: inherit;
    color: inherit;
  }

  .recent-item:hover {
    background-color: var(--bg-surface);
    border-color: var(--accent);
  }

  .recent-name {
    font-size: 13px;
    color: var(--text-primary);
    font-weight: 500;
  }

  .recent-time {
    font-size: 11px;
    color: var(--text-muted);
    grid-row: 1;
    grid-column: 2;
  }

  .recent-path {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
    grid-column: 1 / -1;
  }
</style>
