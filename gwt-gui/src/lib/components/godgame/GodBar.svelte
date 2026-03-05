<script lang="ts">
  interface Props {
    sessionName: string;
    leadStatus: string;
    llmCallCount: number;
    estimatedTokens: number;
    sessionId?: string | null;
  }

  let { sessionName, leadStatus, llmCallCount, estimatedTokens, sessionId = null }: Props = $props();
</script>

<header class="god-bar" aria-label="Session status">
  <div class="god-bar-session">{sessionName}</div>
  <div class="god-bar-stats">
    {#if leadStatus}
      <span class="stat" aria-label="Lead status">Lead: {leadStatus}</span>
    {/if}
    <span class="stat" aria-label="LLM calls">LLM: {llmCallCount}</span>
    <span class="stat" aria-label="Estimated tokens">Tk: {estimatedTokens >= 1000 ? `${(estimatedTokens / 1000).toFixed(1)}k` : estimatedTokens}</span>
    {#if sessionId}
      <span class="stat session-id" aria-label="Session ID">{sessionId}</span>
    {/if}
  </div>
</header>

<style>
  .god-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 16px;
    background: rgba(45, 43, 85, 0.95);
    border-bottom: 1px solid rgba(180, 190, 254, 0.2);
    backdrop-filter: blur(8px);
    flex-shrink: 0;
  }

  .god-bar-session {
    font-weight: 700;
    font-size: var(--ui-font-lg, 14px);
    color: #cdd6f4;
    letter-spacing: 0.02em;
  }

  .god-bar-stats {
    display: flex;
    gap: 16px;
    font-size: var(--ui-font-xs, 10px);
    font-family: var(--font-mono, monospace);
  }

  .stat {
    color: rgba(180, 190, 254, 0.8);
  }

  .session-id {
    max-width: 80px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0.6;
  }
</style>
