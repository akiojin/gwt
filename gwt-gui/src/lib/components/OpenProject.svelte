<script lang="ts">
  import type { ProjectInfo } from "../types";

  interface RecentProject {
    path: string;
    name: string;
    lastOpened: string;
  }

  let { onOpen }: { onOpen: (path: string) => void } = $props();

  let recentProjects: RecentProject[] = $state([]);
  let loading: boolean = $state(false);

  $effect(() => {
    loadRecentProjects();
  });

  async function loadRecentProjects() {
    try {
      const { load } = await import("@tauri-apps/plugin-store");
      const store = await load("recent-projects.json", { defaults: {} });
      const saved = await store.get<RecentProject[]>("projects");
      if (saved) {
        recentProjects = saved;
      }
    } catch {
      // Dev mode fallback
      recentProjects = [
        { path: "/Users/dev/my-project", name: "my-project", lastOpened: "Today" },
        {
          path: "/Users/dev/another-repo",
          name: "another-repo",
          lastOpened: "Yesterday",
        },
      ];
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

  async function openFolder() {
    loading = true;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        await openProject(selected as string);
      }
    } catch {
      // Dev mode fallback
      await openProject("/Users/dev/demo-project");
    }
    loading = false;
  }

  async function openProject(path: string) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const info = await invoke<ProjectInfo>("open_project", { path });
      await saveToRecent(info.path);
      onOpen(info.path);
    } catch {
      // Dev mode fallback: just open directly
      await saveToRecent(path);
      onOpen(path);
    }
  }
</script>

<div class="open-project">
  <div class="content">
    <h1 class="title">gwt</h1>
    <p class="subtitle">Git Worktree Manager</p>

    <button class="open-btn" onclick={openFolder} disabled={loading}>
      {loading ? "Opening..." : "Open Project..."}
    </button>

    {#if recentProjects.length > 0}
      <div class="recent">
        <h3>Recent Projects</h3>
        {#each recentProjects as project}
          <button class="recent-item" onclick={() => openProject(project.path)}>
            <span class="recent-name">{project.name}</span>
            <span class="recent-path">{project.path}</span>
            <span class="recent-time">{project.lastOpened}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

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

  .open-btn {
    width: 100%;
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
