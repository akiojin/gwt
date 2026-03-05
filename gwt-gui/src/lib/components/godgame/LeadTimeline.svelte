<script lang="ts">
  import type { ProjectModeMessage } from '$lib/types';

  interface Props {
    messages: ProjectModeMessage[];
    visible: boolean;
    onClose: () => void;
  }

  let { messages, visible, onClose }: Props = $props();

  let scrollEl: HTMLDivElement | undefined = $state();
  let lastCount = $state(0);

  $effect(() => {
    if (visible && scrollEl && messages.length !== lastCount) {
      lastCount = messages.length;
      requestAnimationFrame(() => {
        if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
      });
    }
  });

  function handleOverlayClick(e: MouseEvent) {
    if (e.target === e.currentTarget) onClose();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose();
  }

  const kindColors: Record<string, string> = {
    message: '#b4befe',
    thought: '#89b4fa',
    action: '#f9e2af',
    observation: '#a6e3a1',
    error: '#f38ba8',
    progress: '#74c7ec',
  };
</script>

{#if visible}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="timeline-overlay"
    onclick={handleOverlayClick}
    onkeydown={handleKeydown}
    role="dialog"
    aria-label="Lead timeline"
    aria-modal="true"
    tabindex="-1"
  >
    <div class="timeline-panel">
      <div class="timeline-header">
        <span class="timeline-title">Lead Timeline</span>
        <button class="timeline-close" onclick={onClose} type="button" aria-label="Close timeline">
          &times;
        </button>
      </div>
      <div class="timeline-body" bind:this={scrollEl}>
        {#if messages.length === 0}
          <div class="timeline-empty">No messages yet.</div>
        {:else}
          {#each messages as msg, i (i)}
            <div class="timeline-msg {msg.role}">
              <span class="msg-badge" style:color={kindColors[msg.kind ?? 'message'] ?? '#b4befe'}>
                {msg.kind ?? msg.role}
              </span>
              <span class="msg-content">{msg.content}</span>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .timeline-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: flex-end;
    justify-content: center;
    z-index: var(--z-modal-base, 1000);
    animation: fade-in 0.2s ease;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .timeline-panel {
    width: 100%;
    max-width: 640px;
    max-height: 60vh;
    background: #2d2b55;
    border: 1px solid rgba(180, 190, 254, 0.2);
    border-bottom: none;
    border-radius: 12px 12px 0 0;
    display: flex;
    flex-direction: column;
    animation: slide-up 0.25s ease;
  }

  @keyframes slide-up {
    from { transform: translateY(100%); }
    to { transform: translateY(0); }
  }

  .timeline-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    border-bottom: 1px solid rgba(180, 190, 254, 0.15);
    flex-shrink: 0;
  }

  .timeline-title {
    font-weight: 700;
    font-size: var(--ui-font-md, 12px);
    color: #cdd6f4;
  }

  .timeline-close {
    background: none;
    border: none;
    color: #6c7086;
    font-size: 20px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .timeline-close:hover {
    color: #cdd6f4;
  }

  .timeline-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .timeline-empty {
    color: #6c7086;
    font-size: var(--ui-font-sm, 11px);
    text-align: center;
    padding: 24px 0;
  }

  .timeline-msg {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 6px 0;
  }

  .timeline-msg.user {
    flex-direction: row-reverse;
  }

  .msg-badge {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 600;
    flex-shrink: 0;
    min-width: 60px;
  }

  .timeline-msg.user .msg-badge {
    text-align: right;
  }

  .msg-content {
    font-size: var(--ui-font-sm, 11px);
    color: rgba(205, 214, 244, 0.9);
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.5;
    min-width: 0;
  }
</style>
