<script lang="ts">
  import { PROFILE_NAME_PATTERN } from "./settingsPanelHelpers";

  let {
    open = false,
    onClose,
    onCreate,
  }: {
    open: boolean;
    onClose: () => void;
    onCreate: (name: string) => void;
  } = $props();

  let name = $state("");
  let inputEl: HTMLInputElement | undefined = $state();

  let isValid = $derived(name.length > 0 && PROFILE_NAME_PATTERN.test(name));

  $effect(() => {
    if (open && inputEl) {
      inputEl.focus();
    }
  });

  $effect(() => {
    if (!open) {
      name = "";
    }
  });

  function handleWindowKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      handleClose();
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && isValid) {
      e.preventDefault();
      handleCreate();
    }
  }

  function handleCreate() {
    if (!isValid) return;
    onCreate(name);
  }

  function handleClose() {
    name = "";
    onClose();
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="modal-overlay"
    onclick={handleClose}
    role="dialog"
    aria-modal="true"
    aria-label="Create Profile"
    tabindex="-1"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-header">
        <h2>Create Profile</h2>
        <button class="close-btn" onclick={handleClose} aria-label="Close">&times;</button>
      </div>

      <div class="dialog-body">
        <label class="field-label" for="profile-name-input">Profile Name</label>
        <input
          bind:this={inputEl}
          bind:value={name}
          id="profile-name-input"
          type="text"
          class="field-input"
          class:invalid={name.length > 0 && !PROFILE_NAME_PATTERN.test(name)}
          placeholder="e.g. work-project"
          onkeydown={handleKeydown}
          autocomplete="off"
          spellcheck="false"
        />
        <span class="field-hint">Lowercase letters, numbers, and hyphens only.</span>
      </div>

      <div class="dialog-footer">
        <button class="btn btn-cancel" onclick={handleClose}>Cancel</button>
        <button
          class="btn btn-save"
          disabled={!isValid}
          onclick={handleCreate}
        >
          Create
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
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .field-input {
    padding: 8px 12px;
    font-size: 13px;
    font-family: inherit;
    background: var(--bg-primary);
    color: var(--text-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    outline: none;
  }

  .field-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px rgba(88, 166, 255, 0.15);
  }

  .field-input.invalid {
    border-color: var(--red);
  }

  .field-hint {
    font-size: 11px;
    color: var(--text-muted);
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
