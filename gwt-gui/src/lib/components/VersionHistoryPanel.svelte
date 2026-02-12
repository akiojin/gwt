<script lang="ts">
  import { onMount } from "svelte";
  import type { ProjectVersions, VersionHistoryResult, VersionItem } from "../types";

  let { projectPath }: { projectPath: string } = $props();

  let versions: VersionItem[] = $state([]);
  let results: Record<string, VersionHistoryResult> = $state({});

  let loading: boolean = $state(false);
  let error: string | null = $state(null);

  // Ensure we register the backend update listener before kicking off generation.
  let mounted: boolean = $state(false);

  let generatingIndex: number = $state(-1);
  let expanded: Record<string, boolean> = $state({ unreleased: true });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function setResult(result: VersionHistoryResult) {
    results = { ...results, [result.version_id]: result };
  }

  function isExpanded(id: string): boolean {
    return !!expanded[id];
  }

  function toggleExpanded(id: string) {
    expanded = { ...expanded, [id]: !expanded[id] };
  }

  async function loadVersions() {
    loading = true;
    error = null;
    versions = [];
    results = {};
    expanded = { unreleased: true };
    generatingIndex = -1;

    const key = projectPath;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const out = await invoke<ProjectVersions>("list_project_versions", {
        projectPath,
        limit: 10,
      });
      if (projectPath !== key) return;
      versions = out.items ?? [];
      generatingIndex = 0;
      await stepGenerate(key);
    } catch (e) {
      if (projectPath !== key) return;
      error = `Failed to load versions: ${toErrorMessage(e)}`;
    } finally {
      if (projectPath === key) loading = false;
    }
  }

  async function stepGenerate(key: string) {
    if (projectPath !== key) return;

    while (generatingIndex >= 0 && generatingIndex < versions.length) {
      const item = versions[generatingIndex];
      if (!item) return;

      const existing = results[item.id];
      if (existing && existing.status === "ok") {
        generatingIndex++;
        continue;
      }

      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const res = await invoke<VersionHistoryResult>("get_project_version_history", {
          projectPath,
          versionId: item.id,
        });
        if (projectPath !== key) return;
        setResult(res);

        if (res.status === "generating") {
          // Wait for event update before continuing.
          return;
        }

        generatingIndex++;
        continue;
      } catch (e) {
        if (projectPath !== key) return;
        setResult({
          status: "error",
          version_id: item.id,
          label: item.label,
          range_to: item.range_to,
          commit_count: item.commit_count,
          range_from: item.range_from ?? null,
          summary_markdown: null,
          changelog_markdown: null,
          error: `Failed to generate history: ${toErrorMessage(e)}`,
        });
        generatingIndex++;
      }
    }
  }

  $effect(() => {
    void projectPath;
    void mounted;
    if (!mounted) return;
    if (!projectPath) return;
    void loadVersions();
  });

  onMount(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen<{
          projectPath: string;
          versionId: string;
          result: VersionHistoryResult;
        }>("project-version-history-updated", (event) => {
          if (cancelled) return;
          if (event.payload.projectPath !== projectPath) return;
          setResult(event.payload.result);

          // Continue sequential generation when the currently generating item updates.
          const idx = versions.findIndex((v) => v.id === event.payload.versionId);
          if (idx === generatingIndex) {
            generatingIndex++;
            void stepGenerate(projectPath);
          }
        });
      } catch {
        // Ignore outside Tauri runtime.
      } finally {
        mounted = true;
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });
</script>

<section class="vh-panel">
  <header class="vh-header">
    <div class="vh-title">
      <h2>Version History</h2>
      <div class="vh-subtitle">AI summary per version tag</div>
    </div>
    <button class="vh-btn" onclick={() => loadVersions()} disabled={loading}>
      {loading ? "Loading..." : "Refresh"}
    </button>
  </header>

  {#if error}
    <div class="vh-error">{error}</div>
  {/if}

  {#if !loading && versions.length === 0 && !error}
    <div class="vh-empty">No version tags found. Showing Unreleased only.</div>
  {/if}

  <div class="vh-list">
    {#each versions as v (v.id)}
      {@const res = results[v.id] ?? null}
      <div class="vh-card">
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="vh-card-header" onclick={() => toggleExpanded(v.id)}>
          <div class="vh-card-left">
            <span class="vh-label">{v.label}</span>
            <span class="vh-meta mono">
              {v.range_from ? `${v.range_from}..${v.range_to}` : v.range_to}
              · {v.commit_count} commit{v.commit_count === 1 ? "" : "s"}
            </span>
          </div>
          <div class="vh-card-right">
            {#if res?.status === "ok"}
              <span class="vh-status ok">ok</span>
            {:else if res?.status === "error"}
              <span class="vh-status err">error</span>
            {:else if res?.status === "disabled"}
              <span class="vh-status warn">ai disabled</span>
            {:else}
              <span class="vh-status gen">generating</span>
            {/if}
            <span class="vh-chevron">{isExpanded(v.id) ? "v" : ">"}</span>
          </div>
        </div>

        {#if isExpanded(v.id)}
          <div class="vh-body">
            {#if res?.status === "disabled"}
              <div class="vh-note">
                AI is not configured. Enable AI settings in Settings to use Version
                History.
              </div>
            {:else if res?.status === "error"}
              <div class="vh-error">{res.error ?? "Failed to generate version history."}</div>
            {:else if res?.status === "generating" || !res}
              <div class="vh-note">Generating summary...</div>
            {/if}

            {#if res?.summary_markdown}
              <h3>Summary</h3>
              <pre class="vh-pre mono">{res.summary_markdown}</pre>
            {/if}

            {#if res?.changelog_markdown}
              <h3>Changelog</h3>
              <pre class="vh-pre mono">{res.changelog_markdown}</pre>
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  </div>
</section>

<style>
  .vh-panel {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 18px 18px 24px;
    background: var(--bg-primary);
    overflow: hidden;
  }

  .vh-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--border-color);
  }

  .vh-title h2 {
    margin: 0;
    font-size: var(--ui-font-xl);
    font-weight: 800;
    color: var(--text-primary);
  }

  .vh-subtitle {
    margin-top: 2px;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .vh-btn {
    padding: 6px 14px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    color: var(--text-primary);
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-md);
  }

  .vh-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .vh-list {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono",
      "Courier New", monospace;
  }

  .vh-card {
    border: 1px solid var(--border-color);
    border-radius: 12px;
    background: var(--bg-secondary);
    overflow: hidden;
    flex-shrink: 0;
  }

  .vh-card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
    cursor: pointer;
    background: linear-gradient(180deg, rgba(255, 255, 255, 0.03), rgba(0, 0, 0, 0));
  }

  .vh-card-left {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .vh-label {
    color: var(--text-primary);
    font-size: var(--ui-font-lg);
    font-weight: 700;
  }

  .vh-meta {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 72vw;
  }

  .vh-card-right {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
  }

  .vh-status {
    font-size: var(--ui-font-sm);
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border-color);
    color: var(--text-secondary);
    background: var(--bg-primary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .vh-status.ok {
    border-color: rgba(120, 220, 170, 0.35);
    color: rgb(150, 235, 195);
  }

  .vh-status.err {
    border-color: rgba(255, 90, 90, 0.35);
    color: rgb(255, 160, 160);
  }

  .vh-status.warn {
    border-color: rgba(255, 200, 120, 0.35);
    color: rgb(255, 220, 170);
  }

  .vh-status.gen {
    border-color: rgba(160, 160, 255, 0.35);
    color: rgb(190, 190, 255);
  }

  .vh-chevron {
    color: var(--text-muted);
    font-size: 14px;
    min-width: 14px;
    text-align: right;
  }

  .vh-body {
    padding: 12px 14px 14px;
    border-top: 1px solid var(--border-color);
    background: var(--bg-secondary);
  }

  .vh-body h3 {
    margin: 10px 0 8px;
    font-size: var(--ui-font-md);
    font-weight: 800;
    color: var(--text-primary);
  }

  .vh-pre {
    margin: 0 0 10px;
    padding: 10px 12px;
    border: 1px solid var(--border-color);
    border-radius: 10px;
    background: var(--bg-primary);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--ui-font-md);
    color: var(--text-secondary);
  }

  .vh-note {
    color: var(--text-muted);
    font-size: var(--ui-font-md);
    margin-bottom: 10px;
  }

  .vh-error {
    color: rgb(255, 160, 160);
    font-size: var(--ui-font-md);
    white-space: pre-wrap;
    margin-bottom: 10px;
  }

  .vh-empty {
    color: var(--text-muted);
    font-size: var(--ui-font-md);
    padding: 14px 0;
  }
</style>
