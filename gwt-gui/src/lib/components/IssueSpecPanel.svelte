<script lang="ts">
  import { onMount } from "svelte";

  type SpecIssueSectionsData = {
    spec: string;
    plan: string;
    tasks: string;
    tdd: string;
    research: string;
    dataModel: string;
    quickstart: string;
    contracts: string;
    checklists: string;
  };

  type SpecIssueDetail = {
    number: number;
    title: string;
    url: string;
    updatedAt: string;
    specId?: string | null;
    etag: string;
    body: string;
    sections: SpecIssueSectionsData;
  };

  let {
    projectPath,
    issueNumber,
    specId,
  }: { projectPath: string; issueNumber: number; specId?: string } = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);
  let detail = $state<SpecIssueDetail | null>(null);

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const message = (err as { message?: unknown }).message;
      if (typeof message === "string") return message;
    }
    return String(err);
  }

  async function loadDetail() {
    if (!projectPath || !issueNumber) return;
    loading = true;
    error = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      detail = await invoke<SpecIssueDetail>("get_spec_issue_detail_cmd", {
        projectPath,
        issueNumber,
      });
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void loadDetail();
  });

  $effect(() => {
    void projectPath;
    void issueNumber;
    void loadDetail();
  });

  function sectionText(value: string | undefined): string {
    const text = (value ?? "").trim();
    return text.length > 0 ? text : "_TODO_";
  }
</script>

<section class="issue-spec-panel">
  <header class="issue-spec-header">
    <div class="issue-spec-title">Issue Spec</div>
    <div class="issue-spec-meta">
      <span>#{issueNumber}</span>
      {#if specId}
        <span>{specId}</span>
      {/if}
    </div>
  </header>

  {#if loading}
    <div class="issue-spec-loading">Loading issue spec...</div>
  {:else if error}
    <div class="issue-spec-error">{error}</div>
  {:else if detail}
    <div class="issue-spec-content">
      <h2>{detail.title}</h2>
      <div class="meta-row">
        <a href={detail.url} target="_blank" rel="noreferrer">Open on GitHub</a>
        <span>Updated: {detail.updatedAt}</span>
        <span>ETag: {detail.etag}</span>
      </div>

      <article class="section">
        <h3>Spec</h3>
        <pre>{sectionText(detail.sections.spec)}</pre>
      </article>
      <article class="section">
        <h3>Plan</h3>
        <pre>{sectionText(detail.sections.plan)}</pre>
      </article>
      <article class="section">
        <h3>Tasks</h3>
        <pre>{sectionText(detail.sections.tasks)}</pre>
      </article>
      <article class="section">
        <h3>TDD</h3>
        <pre>{sectionText(detail.sections.tdd)}</pre>
      </article>
      <article class="section">
        <h3>Research</h3>
        <pre>{sectionText(detail.sections.research)}</pre>
      </article>
      <article class="section">
        <h3>Data Model</h3>
        <pre>{sectionText(detail.sections.dataModel)}</pre>
      </article>
      <article class="section">
        <h3>Quickstart</h3>
        <pre>{sectionText(detail.sections.quickstart)}</pre>
      </article>
      <article class="section">
        <h3>Contracts</h3>
        <pre>{sectionText(detail.sections.contracts)}</pre>
      </article>
      <article class="section">
        <h3>Checklists</h3>
        <pre>{sectionText(detail.sections.checklists)}</pre>
      </article>
    </div>
  {:else}
    <div class="issue-spec-loading">Issue not found.</div>
  {/if}
</section>

<style>
  .issue-spec-panel {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .issue-spec-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-surface);
  }

  .issue-spec-title {
    font-weight: 600;
    color: var(--text-primary);
  }

  .issue-spec-meta {
    display: flex;
    gap: 10px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .issue-spec-loading {
    color: var(--text-muted);
  }

  .issue-spec-error {
    color: var(--red);
    white-space: pre-wrap;
  }

  .issue-spec-content {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .issue-spec-content h2 {
    margin: 0;
    color: var(--text-primary);
  }

  .meta-row {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--text-muted);
  }

  .section {
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-secondary);
    padding: 10px;
  }

  .section h3 {
    margin: 0 0 8px;
    font-size: var(--ui-font-md);
    color: var(--text-primary);
  }

  .section pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
  }
</style>
