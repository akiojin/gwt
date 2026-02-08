<script lang="ts">
  interface Tab {
    id: string;
    label: string;
    type: "summary" | "agent";
  }

  let tabs: Tab[] = $state([
    { id: "summary", label: "Session Summary", type: "summary" },
    { id: "agent-1", label: "Claude Code", type: "agent" },
  ]);
  let activeTabId: string = $state("summary");
</script>

<main class="main-area">
  <div class="tab-bar">
    {#each tabs as tab}
      <button
        class="tab"
        class:active={activeTabId === tab.id}
        onclick={() => (activeTabId = tab.id)}
      >
        {#if tab.type === "agent"}
          <span class="tab-dot"></span>
        {/if}
        {tab.label}
      </button>
    {/each}
  </div>
  <div class="tab-content">
    {#if activeTabId === "summary"}
      <div class="placeholder">
        <h2>Session Summary</h2>
        <p>Select a branch to view session details.</p>
      </div>
    {:else}
      <div class="terminal-placeholder">
        <p>Terminal output will appear here (xterm.js — Phase 2)</p>
      </div>
    {/if}
  </div>
</main>

<style>
  .main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background-color: var(--bg-primary);
  }

  .tab-bar {
    display: flex;
    height: var(--tab-height);
    background-color: var(--bg-secondary);
    border-bottom: 1px solid var(--border-color);
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tab-bar::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 16px;
    background: none;
    border: none;
    border-right: 1px solid var(--border-color);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
    font-family: inherit;
  }

  .tab:hover {
    color: var(--text-primary);
    background-color: var(--bg-hover);
  }

  .tab.active {
    color: var(--text-primary);
    background-color: var(--bg-primary);
    border-bottom: 2px solid var(--accent);
  }

  .tab-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: var(--green);
  }

  .tab-content {
    flex: 1;
    overflow: auto;
    padding: 24px;
  }

  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
  }

  .placeholder h2 {
    font-size: 18px;
    font-weight: 500;
    margin-bottom: 8px;
    color: var(--text-secondary);
  }

  .terminal-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
    background-color: var(--bg-secondary);
    border-radius: 8px;
    font-family: monospace;
  }
</style>
