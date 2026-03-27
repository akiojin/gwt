<script lang="ts">
  import type {
    CloneProgress,
    OpenProjectResult,
    ProbePathResult,
  } from "../types";
  import { invoke, listen } from "$lib/tauriInvoke";
  import { isBrowserDevMode } from "$lib/tauriMock";
  import MigrationModal from "./MigrationModal.svelte";
  import { startupProfilingTracker } from "$lib/startupProfiling";

  interface RecentProject {
    path: string;
    lastOpened: string;
    repo_name?: string;
  }

  let { onOpen }: { onOpen: (path: string, startupToken?: string) => void } = $props();

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
      const projects = await invoke<RecentProject[]>("get_recent_projects");
      recentProjects = projects;
    } catch (err) {
      console.error("Failed to load recent projects:", err);
      recentProjects = [];
    }
  }

  async function openFolder() {
    if (isBrowserDevMode()) {
      // In browser dev mode, simulate opening a project
      onOpen("/Users/demo/projects/my-app");
      return;
    }
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
    const startupToken = startupProfilingTracker.start("open_project");
    try {
      const result = await invoke<OpenProjectResult>("open_project", {
        path: projectPath,
      });
      if (result.action === "opened") {
        onOpen(result.info.path, startupToken);
      } else {
        startupProfilingTracker.discard(startupToken);
      }
    } catch (err) {
      startupProfilingTracker.discard(startupToken);
      errorMessage = normalizeOpenProjectError(toErrorMessage(err));
    } finally {
      opening = false;
    }
  }

  async function probeAndOpen(path: string, fromRecent: boolean) {
    opening = true;
    errorMessage = null;
    try {
      const probe = await invoke<ProbePathResult>("probe_path", { path });

      if (probe.kind === "gwtProject" && probe.projectPath) {
        await openGwtProject(probe.projectPath, fromRecent);
        return;
      }

      if (probe.kind === "migrationRequired" && probe.migrationSourceRoot) {
        migrationSourceRoot = probe.migrationSourceRoot;
        migrationOpen = true;
        return;
      }

      if (probe.kind === "emptyDir" && probe.projectPath) {
        showNewProject = true;
        parentDir = probe.projectPath;
        return;
      }

      errorMessage = normalizeProbeError(probe);
    } catch (err) {
      errorMessage = normalizeOpenProjectError(toErrorMessage(err));
    } finally {
      opening = false;
    }
  }

  async function chooseParentDir() {
    if (isBrowserDevMode()) return;
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
      unlisten = await listen<CloneProgress>("clone-progress", (event) => {
        cloneProgress = event.payload;
      });
      const result = await invoke<OpenProjectResult>("create_project", {
        request: { repoUrl, parentDir, shallow: shallowClone },
      });
      if (result.action === "opened") {
        onOpen(result.info.path);
      }
    } catch (err) {
      const msg = toErrorMessage(err);
      errorMessage = msg.includes("Invalid repository URL")
        ? "Invalid repository URL"
        : `Failed to create project: ${msg}`;
    } finally {
      if (unlisten) unlisten();
      creating = false;
    }
  }

  function formatRelativeTime(timestamp: string | number): string {
    const ms = typeof timestamp === "number" ? timestamp : new Date(timestamp).getTime();
    const diff = Date.now() - ms;
    const minutes = Math.floor(diff / 60000);
    if (minutes < 1) return "Just now";
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 30) return `${days}d ago`;
    return new Date(ms).toLocaleDateString();
  }

  function getProjectIcon(name: string): string {
    if (name.includes("api") || name.includes("backend")) return "{}";
    if (name.includes("ui") || name.includes("frontend") || name.includes("gui")) return "◇";
    if (name.includes("docs") || name.includes("doc")) return "≡";
    return "⬡";
  }
</script>

<div class="landing">
  <div class="landing-content">
    <!-- Hero section -->
    <div class="hero">
      <div class="logo">
        <svg width="48" height="48" viewBox="0 0 48 48" fill="none">
          <rect x="4" y="4" width="40" height="40" rx="12" fill="var(--accent-muted)" stroke="var(--accent)" stroke-width="1.5"/>
          <path d="M16 24 L24 16 L32 24 L24 32Z" fill="var(--accent)" opacity="0.6"/>
          <path d="M24 14 L24 34 M14 24 L34 24" stroke="var(--accent)" stroke-width="2" stroke-linecap="round"/>
        </svg>
      </div>
      <h1 class="hero-title">gwt</h1>
      <p class="hero-subtitle">Git Worktree Manager</p>
    </div>

    <!-- Action buttons -->
    <div class="action-row">
      <button
        class="btn btn-primary btn-lg action-open"
        onclick={openFolder}
        disabled={opening || creating}
      >
        <span class="action-icon">+</span>
        {opening ? "Opening..." : "Open Project"}
      </button>
      <button
        class="btn btn-secondary btn-lg"
        onclick={() => (showNewProject = !showNewProject)}
        disabled={opening || creating}
      >
        Clone Repository
      </button>
    </div>

    {#if errorMessage}
      <div class="error-banner" role="alert">
        <span class="error-icon">!</span>
        <span>{errorMessage}</span>
      </div>
    {/if}

    <!-- New project form -->
    {#if showNewProject}
      <div class="card clone-form">
        <h3 class="form-title">Clone Repository</h3>

        <label class="form-field">
          <span class="form-label">Repository URL</span>
          <input
            class="input"
            type="text"
            autocapitalize="off"
            autocorrect="off"
            autocomplete="off"
            spellcheck="false"
            placeholder="https://github.com/owner/repo"
            bind:value={repoUrl}
            disabled={creating}
          />
        </label>

        <div class="form-field">
          <span class="form-label">Parent Directory</span>
          <div class="input-group">
            <input
              class="input"
              type="text"
              autocapitalize="off"
              autocorrect="off"
              autocomplete="off"
              spellcheck="false"
              placeholder="Choose a folder..."
              value={parentDir}
              readonly
              disabled={creating}
            />
            <button
              class="btn btn-secondary"
              onclick={chooseParentDir}
              disabled={creating}
            >
              Browse
            </button>
          </div>
        </div>

        <div class="form-field">
          <span class="form-label">Clone Mode</span>
          <div class="toggle-group">
            <button
              class="toggle-option"
              class:active={shallowClone}
              type="button"
              onclick={() => (shallowClone = true)}
              disabled={creating}
            >
              Shallow
            </button>
            <button
              class="toggle-option"
              class:active={!shallowClone}
              type="button"
              onclick={() => (shallowClone = false)}
              disabled={creating}
            >
              Full
            </button>
          </div>
          <span class="form-hint">Shallow clone downloads only recent history (faster)</span>
        </div>

        <button
          class="btn btn-primary"
          style="width: 100%"
          onclick={createProject}
          disabled={creating || !repoUrl || !parentDir}
        >
          {creating ? "Cloning..." : "Clone & Open"}
        </button>

        {#if cloneProgress}
          <div class="progress-section">
            <div class="progress-meta">
              <span>{progressLabel(cloneProgress)}</span>
              <span class="mono">{cloneProgress.percent}%</span>
            </div>
            <div class="progress-track">
              <div class="progress-bar" style={`width: ${cloneProgress.percent}%`}></div>
            </div>
          </div>
        {/if}
      </div>
    {/if}

    <!-- Recent projects -->
    {#if recentProjects.length > 0}
      <div class="recent-section">
        <h3 class="section-title">Recent Projects</h3>
        <div class="recent-list">
          {#each recentProjects as project}
            <button
              class="recent-card card-interactive"
              onclick={() => probeAndOpen(project.path, true)}
              disabled={opening || creating}
            >
              <span class="project-icon">{getProjectIcon(project.path.split("/").pop() || "")}</span>
              <div class="project-info">
                <span class="project-name">{project.repo_name || project.path.split("/").pop()}</span>
                <span class="project-path">{project.path}</span>
              </div>
              <span class="project-time">{formatRelativeTime(project.lastOpened)}</span>
            </button>
          {/each}
        </div>
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
  .landing {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    background: radial-gradient(ellipse at 50% 20%, var(--bg-elevated) 0%, var(--bg-primary) 70%);
  }

  .landing-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-5);
    max-width: 520px;
    width: 100%;
    padding: var(--space-12);
    animation: slide-up var(--transition-slow) ease;
  }

  /* Hero */
  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .logo {
    margin-bottom: var(--space-2);
  }

  .hero-title {
    font-size: var(--ui-font-4xl);
    font-weight: var(--font-weight-bold);
    color: var(--accent);
    letter-spacing: -1.5px;
    line-height: 1;
  }

  .hero-subtitle {
    color: var(--text-muted);
    font-size: var(--ui-font-lg);
    font-weight: var(--font-weight-normal);
  }

  /* Actions */
  .action-row {
    width: 100%;
    display: flex;
    gap: var(--space-3);
  }

  .action-open {
    flex: 1;
  }

  .action-icon {
    font-size: var(--ui-font-xl);
    line-height: 1;
  }

  /* Error banner */
  .error-banner {
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--red-muted);
    border: 1px solid rgba(243, 139, 168, 0.3);
    border-radius: var(--radius-md);
    color: var(--red);
    font-size: var(--ui-font-md);
    line-height: var(--line-height-normal);
  }

  .error-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-full);
    background: var(--red);
    color: var(--bg-primary);
    font-size: var(--ui-font-xs);
    font-weight: var(--font-weight-bold);
    flex-shrink: 0;
  }

  /* Clone form */
  .clone-form {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    animation: slide-up var(--transition-normal) ease;
  }

  .form-title {
    color: var(--text-secondary);
    font-size: var(--ui-font-sm);
    font-weight: var(--font-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .form-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .form-label {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    font-weight: var(--font-weight-medium);
  }

  .form-hint {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .input-group {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .toggle-group {
    display: flex;
    gap: 0;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .toggle-option {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-sm);
    font-weight: var(--font-weight-medium);
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .toggle-option:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .toggle-option.active {
    background: var(--accent-muted);
    color: var(--accent);
  }

  .toggle-option:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Progress */
  .progress-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .progress-meta {
    display: flex;
    justify-content: space-between;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .mono {
    font-family: var(--font-mono);
  }

  .progress-track {
    height: 4px;
    border-radius: var(--radius-full);
    background: var(--bg-inset);
    overflow: hidden;
  }

  .progress-bar {
    height: 100%;
    background: var(--accent);
    border-radius: var(--radius-full);
    transition: width var(--transition-normal);
  }

  /* Recent projects */
  .recent-section {
    width: 100%;
    margin-top: var(--space-2);
  }

  .section-title {
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
    font-weight: var(--font-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-bottom: var(--space-3);
  }

  .recent-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .recent-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    cursor: pointer;
    text-align: left;
    font-family: inherit;
    color: inherit;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    transition: border-color var(--transition-fast), background var(--transition-fast);
  }

  .recent-card:hover:not(:disabled) {
    background: var(--bg-surface);
    border-color: var(--accent);
  }

  .recent-card:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .project-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: var(--radius-md);
    background: var(--accent-subtle);
    color: var(--accent);
    font-size: var(--ui-font-lg);
    flex-shrink: 0;
  }

  .project-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .project-name {
    font-size: var(--ui-font-md);
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);
  }

  .project-path {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .project-time {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    white-space: nowrap;
    flex-shrink: 0;
  }
</style>
