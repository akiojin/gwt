<script lang="ts">
  import type { TerminalAnsiProbe } from "../types";

  let {
    open,
    loading,
    error,
    diagnostics,
    onclose,
  }: {
    open: boolean;
    loading: boolean;
    error: string | null;
    diagnostics: TerminalAnsiProbe | null;
    onclose: () => void;
  } = $props();
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay modal-overlay" onclick={onclose}>
    <div class="diag-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
      <h2>Terminal Diagnostics</h2>

      {#if loading}
        <p class="diag-muted">Probing output...</p>
      {:else if error}
        <p class="diag-error">{error}</p>
      {:else if diagnostics}
        <div class="diag-grid">
          <div class="diag-item"><span class="diag-label">Pane</span><span class="diag-value mono">{diagnostics.pane_id}</span></div>
          <div class="diag-item"><span class="diag-label">Bytes</span><span class="diag-value mono">{diagnostics.bytes_scanned}</span></div>
          <div class="diag-item"><span class="diag-label">ESC</span><span class="diag-value mono">{diagnostics.esc_count}</span></div>
          <div class="diag-item"><span class="diag-label">SGR</span><span class="diag-value mono">{diagnostics.sgr_count}</span></div>
          <div class="diag-item"><span class="diag-label">Color SGR</span><span class="diag-value mono">{diagnostics.color_sgr_count}</span></div>
          <div class="diag-item"><span class="diag-label">256-color</span><span class="diag-value mono">{diagnostics.has_256_color ? "yes" : "no"}</span></div>
          <div class="diag-item"><span class="diag-label">TrueColor</span><span class="diag-value mono">{diagnostics.has_true_color ? "yes" : "no"}</span></div>
        </div>

        {#if diagnostics.color_sgr_count === 0}
          <div class="diag-hint">
            <p>No color SGR codes were detected in the tail of the scrollback. This usually means the program did not emit ANSI colors.</p>
            <p class="diag-muted">Try forcing color output:</p>
            <pre class="diag-code mono">git -c color.ui=always diff</pre>
            <pre class="diag-code mono">rg --color=always PATTERN</pre>
          </div>
        {:else}
          <div class="diag-hint">
            <p>Color SGR codes were detected. If you still do not see colors, the issue is likely in the terminal rendering path.</p>
          </div>
        {/if}
      {:else}
        <p class="diag-muted">No data.</p>
      {/if}

      <button class="about-close" onclick={onclose}>Close</button>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-modal-base);
  }

  .mono {
    font-family: monospace;
  }

  .about-close {
    padding: 6px 20px;
    border-radius: 999px;
    border: 1px solid var(--border-color);
    background: var(--bg-tertiary);
    color: var(--text-primary);
    cursor: pointer;
    align-self: flex-end;
  }

  .about-close:hover {
    background: var(--bg-hover);
  }

  .diag-dialog {
    background: var(--bg-secondary);
    color: var(--text-primary);
    border-radius: var(--radius-lg);
    width: min(680px, 92vw);
    max-height: min(520px, 84vh);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .diag-dialog h2 {
    font-size: var(--ui-font-xl);
    margin: 0;
  }

  .diag-muted {
    color: var(--text-muted);
  }

  .diag-error {
    color: rgb(255, 160, 160);
  }

  .diag-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 12px;
  }

  .diag-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px;
    border-radius: var(--radius-md);
    background: var(--bg-tertiary);
  }

  .diag-label {
    color: var(--text-muted);
    font-size: 12px;
  }

  .diag-value {
    color: var(--text-primary);
  }

  .diag-hint {
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    padding: 12px;
    background: var(--bg-tertiary);
  }

  .diag-hint p {
    margin: 0 0 8px;
  }

  .diag-code {
    margin: 8px 0;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    overflow-x: auto;
  }
</style>
