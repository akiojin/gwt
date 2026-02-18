<script lang="ts">
  import type { FileChange, FileDiff, WorkingTreeEntry, FileChangeKind } from "../types";

  let {
    projectPath,
    branch,
    baseBranch,
    refreshToken,
  }: {
    projectPath: string;
    branch: string;
    baseBranch: string;
    refreshToken: number;
  } = $props();

  type FilterMode = "committed" | "uncommitted";
  let filterMode: FilterMode = $state("committed");

  // Committed state
  let files: FileChange[] = $state([]);
  let filesLoading: boolean = $state(true);
  let filesError: string | null = $state(null);

  // Diff expand state
  let expandedPaths: Set<string> = $state(new Set());
  let diffs: Map<string, FileDiff> = $state(new Map());
  let diffLoading: Set<string> = $state(new Set());

  // Uncommitted state
  let workingTree: WorkingTreeEntry[] = $state([]);
  let workingTreeLoading: boolean = $state(true);
  let workingTreeError: string | null = $state(null);

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  // --- Directory tree helpers ---
  interface TreeNode {
    name: string;
    fullPath: string;
    children: TreeNode[];
    file?: FileChange;
  }

  function buildTree(fileList: FileChange[]): TreeNode[] {
    const root: TreeNode = { name: "", fullPath: "", children: [] };

    for (const f of fileList) {
      const parts = f.path.split("/");
      let current = root;

      for (let i = 0; i < parts.length; i++) {
        const part = parts[i];
        const fullPath = parts.slice(0, i + 1).join("/");
        const isFile = i === parts.length - 1;

        if (isFile) {
          current.children.push({ name: part, fullPath, children: [], file: f });
        } else {
          let dir = current.children.find((c) => c.name === part && !c.file);
          if (!dir) {
            dir = { name: part, fullPath, children: [] };
            current.children.push(dir);
          }
          current = dir;
        }
      }
    }

    return root.children;
  }

  function kindLabel(kind: FileChangeKind): string {
    switch (kind) {
      case "Added": return "A";
      case "Modified": return "M";
      case "Deleted": return "D";
      case "Renamed": return "R";
    }
  }

  function kindColor(kind: FileChangeKind): string {
    switch (kind) {
      case "Added": return "var(--green)";
      case "Modified": return "var(--yellow)";
      case "Deleted": return "var(--red)";
      case "Renamed": return "var(--cyan)";
    }
  }

  // GitHub-style 5-block stat bar
  function statBlocks(additions: number, deletions: number): { color: string }[] {
    const total = additions + deletions;
    if (total === 0) return Array(5).fill({ color: "var(--text-muted)" });

    const addRatio = additions / total;
    const addBlocks = Math.round(addRatio * 5);
    const delBlocks = 5 - addBlocks;

    const blocks: { color: string }[] = [];
    for (let i = 0; i < addBlocks; i++) blocks.push({ color: "var(--green)" });
    for (let i = 0; i < delBlocks; i++) blocks.push({ color: "var(--red)" });
    return blocks;
  }

  let tree = $derived(buildTree(files));

  async function loadFiles() {
    filesLoading = true;
    filesError = null;
    expandedPaths = new Set();
    diffs = new Map();
    diffLoading = new Set();
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<FileChange[]>("get_branch_diff_files", {
        projectPath,
        branch,
        baseBranch,
      });
      files = result ?? [];
    } catch (err) {
      filesError = toErrorMessage(err);
      files = [];
    } finally {
      filesLoading = false;
    }
  }

  async function loadWorkingTree() {
    workingTreeLoading = true;
    workingTreeError = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<WorkingTreeEntry[]>("get_working_tree_status", {
        projectPath,
        branch,
      });
      workingTree = result ?? [];
    } catch (err) {
      workingTreeError = toErrorMessage(err);
      workingTree = [];
    } finally {
      workingTreeLoading = false;
    }
  }

  async function toggleDiff(filePath: string, isBinary: boolean) {
    if (isBinary) return;

    if (expandedPaths.has(filePath)) {
      const next = new Set(expandedPaths);
      next.delete(filePath);
      expandedPaths = next;
      return;
    }

    expandedPaths = new Set([...expandedPaths, filePath]);

    if (diffs.has(filePath)) return;

    diffLoading = new Set([...diffLoading, filePath]);
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const diff = await invoke<FileDiff>("get_file_diff", {
        projectPath,
        branch,
        baseBranch,
        filePath,
      });
      diffs = new Map([...diffs, [filePath, diff]]);
    } catch (err) {
      diffs = new Map([...diffs, [filePath, { content: `Error: ${toErrorMessage(err)}`, truncated: false }]]);
    } finally {
      const next = new Set(diffLoading);
      next.delete(filePath);
      diffLoading = next;
    }
  }

  $effect(() => {
    void projectPath;
    void branch;
    void baseBranch;
    void refreshToken;
    loadFiles();
  });

  $effect(() => {
    void projectPath;
    void branch;
    void refreshToken;
    if (filterMode === "uncommitted") {
      loadWorkingTree();
    }
  });

  let staged = $derived(workingTree.filter((e) => e.is_staged));
  let unstaged = $derived(workingTree.filter((e) => !e.is_staged));
</script>

<div class="changes-tab">
  <div class="filter-bar">
    <button
      class="filter-btn"
      class:active={filterMode === "committed"}
      onclick={() => (filterMode = "committed")}
    >
      Committed
    </button>
    <button
      class="filter-btn"
      class:active={filterMode === "uncommitted"}
      onclick={() => (filterMode = "uncommitted")}
    >
      Uncommitted
    </button>
  </div>

  {#if filterMode === "committed"}
    {#if filesLoading}
      <div class="changes-loading">Loading...</div>
    {:else if filesError}
      <div class="changes-error">{filesError}</div>
    {:else if files.length === 0}
      <div class="changes-empty">No changes</div>
    {:else}
      <div class="file-tree">
        {#snippet renderTree(nodes: TreeNode[], depth: number)}
          {#each nodes as node (node.fullPath)}
            {#if node.file}
              <!-- File node -->
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="file-row"
                class:binary={node.file.is_binary}
                class:expanded={expandedPaths.has(node.file.path)}
                style="padding-left: {depth * 16 + 8}px"
                onclick={() => toggleDiff(node.file!.path, node.file!.is_binary)}
              >
                <span class="file-kind" style="color: {kindColor(node.file.kind)}">{kindLabel(node.file.kind)}</span>
                <span class="file-name">{node.name}</span>
                {#if node.file.is_binary}
                  <span class="file-binary-label">Binary file changed</span>
                {:else}
                  <span class="stat-bar">
                    {#each statBlocks(node.file.additions, node.file.deletions) as block}
                      <span class="stat-block" style="background: {block.color}"></span>
                    {/each}
                  </span>
                  <span class="stat-nums">
                    <span class="stat-add">+{node.file.additions}</span>
                    <span class="stat-del">-{node.file.deletions}</span>
                  </span>
                {/if}
              </div>
              {#if expandedPaths.has(node.file.path)}
                <div class="diff-panel" style="margin-left: {depth * 16 + 8}px">
                  {#if diffLoading.has(node.file.path)}
                    <div class="diff-loading">Loading diff...</div>
                  {:else if diffs.has(node.file.path)}
                    {@const diff = diffs.get(node.file.path)!}
                    <pre class="diff-content">{#each diff.content.split("\n") as line}<span class={line.startsWith("+") ? "diff-add" : line.startsWith("-") ? "diff-del" : "diff-ctx"}>{line}
</span>{/each}</pre>
                    {#if diff.truncated}
                      <div class="diff-truncated">Too large to display</div>
                    {/if}
                  {/if}
                </div>
              {/if}
            {:else}
              <!-- Directory node -->
              <div class="dir-row" style="padding-left: {depth * 16 + 8}px">
                <span class="dir-icon">/</span>
                <span class="dir-name">{node.name}</span>
              </div>
              {@render renderTree(node.children, depth + 1)}
            {/if}
          {/each}
        {/snippet}
        {@render renderTree(tree, 0)}
      </div>
    {/if}
  {:else}
    <!-- Uncommitted view -->
    {#if workingTreeLoading}
      <div class="changes-loading">Loading...</div>
    {:else if workingTreeError}
      <div class="changes-error">{workingTreeError}</div>
    {:else if workingTree.length === 0}
      <div class="changes-empty">No uncommitted changes</div>
    {:else}
      <div class="wt-sections">
        <div class="wt-section">
          <div class="wt-section-header">Staged</div>
          {#if staged.length === 0}
            <div class="wt-empty">No staged changes</div>
          {:else}
            {#each staged as entry (entry.path)}
              <div class="wt-row">
                <span class="file-kind" style="color: {kindColor(entry.status)}">{kindLabel(entry.status)}</span>
                <span class="wt-path">{entry.path}</span>
              </div>
            {/each}
          {/if}
        </div>
        <div class="wt-section">
          <div class="wt-section-header">Unstaged</div>
          {#if unstaged.length === 0}
            <div class="wt-empty">No unstaged changes</div>
          {:else}
            {#each unstaged as entry (entry.path)}
              <div class="wt-row">
                <span class="file-kind" style="color: {kindColor(entry.status)}">{kindLabel(entry.status)}</span>
                <span class="wt-path">{entry.path}</span>
              </div>
            {/each}
          {/if}
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .changes-tab {
    padding: 8px 0;
  }

  .filter-bar {
    display: flex;
    gap: 4px;
    margin-bottom: 10px;
  }

  .filter-btn {
    padding: 4px 12px;
    border: 1px solid var(--border-color);
    border-radius: 4px;
    background: transparent;
    color: var(--text-secondary);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
  }

  .filter-btn.active {
    background: var(--bg-surface);
    color: var(--text-primary);
    border-color: var(--accent);
  }

  .filter-btn:hover:not(.active) {
    background: var(--bg-hover);
  }

  .changes-loading,
  .changes-empty {
    font-size: 12px;
    color: var(--text-muted);
    padding: 8px 0;
  }

  .changes-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
  }

  .file-tree {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .dir-row {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 4px 8px;
    font-size: 12px;
  }

  .dir-icon {
    color: var(--text-muted);
    font-family: monospace;
  }

  .dir-name {
    color: var(--text-secondary);
    font-weight: 600;
  }

  .file-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
  }

  .file-row:hover {
    background: var(--bg-hover);
  }

  .file-row.binary {
    cursor: default;
  }

  .file-row.expanded {
    background: var(--bg-surface);
  }

  .file-kind {
    font-family: monospace;
    font-size: 11px;
    font-weight: 700;
    flex-shrink: 0;
    width: 14px;
    text-align: center;
  }

  .file-name {
    color: var(--text-primary);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-binary-label {
    color: var(--text-muted);
    font-size: 11px;
    font-style: italic;
    margin-left: auto;
  }

  .stat-bar {
    display: flex;
    gap: 1px;
    margin-left: auto;
    flex-shrink: 0;
  }

  .stat-block {
    width: 8px;
    height: 8px;
    border-radius: 1px;
  }

  .stat-nums {
    font-family: monospace;
    font-size: 11px;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .stat-add {
    color: var(--green);
  }

  .stat-del {
    color: var(--red);
    margin-left: 4px;
  }

  .diff-panel {
    border-left: 2px solid var(--border-color);
    margin-bottom: 4px;
  }

  .diff-loading {
    font-size: 11px;
    color: var(--text-muted);
    padding: 8px 12px;
  }

  .diff-content {
    font-family: monospace;
    font-size: 11px;
    line-height: 1.5;
    padding: 8px 12px;
    overflow-x: auto;
    white-space: pre;
    margin: 0;
  }

  .diff-add {
    background: rgba(166, 227, 161, 0.15);
    color: var(--green);
  }

  .diff-del {
    background: rgba(243, 139, 168, 0.15);
    color: var(--red);
  }

  .diff-ctx {
    color: var(--text-secondary);
  }

  .diff-truncated {
    font-size: 11px;
    color: var(--yellow);
    padding: 6px 12px;
    font-style: italic;
  }

  /* Uncommitted view */
  .wt-sections {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .wt-section-header {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
    padding: 4px 8px;
    border-bottom: 1px solid var(--border-color);
  }

  .wt-empty {
    font-size: 12px;
    color: var(--text-muted);
    padding: 6px 8px;
  }

  .wt-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 4px 8px;
    font-size: 12px;
    border-radius: 4px;
  }

  .wt-row:hover {
    background: var(--bg-hover);
  }

  .wt-path {
    color: var(--text-primary);
    font-family: monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
</style>
