<script lang="ts">
  let {
    open = false,
    title = "Confirm",
    message = "",
    confirmLabel = "Delete",
    confirmDanger = true,
    onClose,
    onConfirm,
  }: {
    open: boolean;
    title?: string;
    message?: string;
    confirmLabel?: string;
    confirmDanger?: boolean;
    onClose: () => void;
    onConfirm: () => void;
  } = $props();

  let dialogEl: HTMLDivElement | undefined = $state();
  let cancelButtonEl: HTMLButtonElement | undefined = $state();
  let confirmButtonEl: HTMLButtonElement | undefined = $state();
  let previousFocus: HTMLElement | null = $state(null);

  const FOCUSABLE_SELECTOR =
    'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  $effect(() => {
    if (open) {
      previousFocus = document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
      queueMicrotask(() => {
        confirmButtonEl?.focus();
      });
      return;
    }

    const target = previousFocus;
    previousFocus = null;
    if (target) {
      queueMicrotask(() => {
        target.focus();
      });
    }
  });

  function focusableElements(): HTMLElement[] {
    if (!dialogEl) return [];
    return Array.from(dialogEl.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (el) => !el.hidden && el.getAttribute("aria-hidden") !== "true"
    );
  }

  function handleDialogKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key !== "Tab") return;

    const focusables = focusableElements();
    if (focusables.length === 0) {
      e.preventDefault();
      return;
    }

    const current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const currentIndex = current ? focusables.indexOf(current) : -1;
    let nextIndex = currentIndex;

    if (e.shiftKey) {
      nextIndex = currentIndex <= 0 ? focusables.length - 1 : currentIndex - 1;
    } else {
      nextIndex = currentIndex === -1 || currentIndex >= focusables.length - 1 ? 0 : currentIndex + 1;
    }

    e.preventDefault();
    focusables[nextIndex]?.focus();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="modal-overlay"
    onclick={onClose}
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      bind:this={dialogEl}
      class="dialog modal-dialog-shell"
      onclick={(e) => e.stopPropagation()}
      onkeydown={handleDialogKeydown}
    >
      <div class="dialog-header">
        <h2>{title}</h2>
        <button class="close-btn" onclick={onClose} aria-label="Close">&times;</button>
      </div>

      <div class="dialog-body">
        <p class="message">{message}</p>
      </div>

      <div class="dialog-footer">
        <button bind:this={cancelButtonEl} class="btn btn-cancel" onclick={onClose}>Cancel</button>
        <button
          bind:this={confirmButtonEl}
          class="btn {confirmDanger ? 'btn-danger' : 'btn-save'}"
          onclick={onConfirm}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog {
    max-width: 480px;
    width: 90vw;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-color);
  }

  .dialog-header h2 {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 20px;
    padding: 4px 8px;
    border-radius: 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .dialog-body {
    padding: 16px 20px;
  }

  .message {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 20px;
    border-top: 1px solid var(--border-color);
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
  }

  .btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
  }

  .btn-danger {
    background: var(--red);
    color: var(--bg-primary);
  }

  .btn-danger:hover:not(:disabled) {
    filter: brightness(1.05);
  }

  .btn-danger:disabled {
    background: var(--bg-surface);
    color: var(--text-muted);
    cursor: not-allowed;
    filter: none;
  }

  .btn-save {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .btn-save:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-save:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
