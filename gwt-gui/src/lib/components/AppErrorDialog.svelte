<script lang="ts">
  let {
    message,
    onclose,
  }: {
    message: string | null;
    onclose: () => void;
  } = $props();
</script>

{#if message}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay modal-overlay" onclick={onclose}>
    <div class="error-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
      <h2>Error</h2>
      <p class="error-text">{message}</p>
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

  .error-dialog {
    background: var(--bg-secondary);
    color: var(--text-primary);
    border-radius: var(--radius-lg);
    width: min(560px, 90vw);
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .error-dialog h2 {
    font-size: var(--ui-font-2xl);
    margin: 0;
  }

  .error-text {
    color: var(--text-secondary);
    margin: 0;
  }
</style>
