<script lang="ts">
  import type { StructuredError } from "$lib/errorBus";

  export type ToastAction =
    | { kind: "apply-update"; latest: string }
    | { kind: "report-error"; error: StructuredError }
    | null;

  let {
    message,
    action,
    onapply,
    onreport,
    onclose,
  }: {
    message: string | null;
    action: ToastAction;
    onapply: () => void;
    onreport: (error: StructuredError) => void;
    onclose: () => void;
  } = $props();
</script>

{#if message}
  <div class="toast-container">
    <div class="toast-message">
      <span>{message}</span>
      {#if action?.kind === "apply-update"}
        <button class="toast-action" onclick={onapply}>Update</button>
      {:else if action?.kind === "report-error"}
        <button class="toast-action" onclick={() => onreport(action.error)}>Report</button>
      {/if}
      <button class="toast-close" aria-label="Close" onclick={onclose}>&times;</button>
    </div>
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    right: 20px;
    bottom: 20px;
    z-index: 1000;
    pointer-events: none;
  }

  .toast-message {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
    border-radius: 12px;
    background: rgba(20, 22, 34, 0.94);
    color: var(--text-primary);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.32);
    border: 1px solid var(--border-color);
  }

  .toast-action {
    border: 1px solid var(--border-color);
    background: transparent;
    color: var(--text-primary);
    border-radius: 999px;
    padding: 5px 10px;
    cursor: pointer;
  }

  .toast-action:hover {
    background: var(--bg-hover, rgba(255, 255, 255, 0.08));
  }

  .toast-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
  }

  .toast-close:hover {
    color: var(--text-primary);
  }
</style>
