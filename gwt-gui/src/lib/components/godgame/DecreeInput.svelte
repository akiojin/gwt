<script lang="ts">
  interface Props {
    disabled?: boolean;
    isWaiting?: boolean;
    onSend: (message: string) => void;
  }

  let { disabled = false, isWaiting = false, onSend }: Props = $props();

  let input = $state('');
  let isComposing = $state(false);
  let ignoreEnterAfterComposition = $state(false);

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      if (isComposing || ignoreEnterAfterComposition || event.isComposing) {
        event.preventDefault();
        return;
      }
      if (!event.shiftKey) {
        event.preventDefault();
        send();
      }
    }
  }

  function handleCompositionStart() {
    isComposing = true;
    ignoreEnterAfterComposition = false;
  }

  function handleCompositionEnd() {
    isComposing = false;
    ignoreEnterAfterComposition = true;
    setTimeout(() => {
      ignoreEnterAfterComposition = false;
    }, 0);
  }

  function send() {
    const text = input.trim();
    if (!text || disabled || isWaiting) return;
    onSend(text);
    input = '';
  }
</script>

<footer class="decree-input" role="form" aria-label="Send decree">
  <div class="decree-bar">
    <input
      type="text"
      class="decree-field"
      placeholder="Decree something..."
      bind:value={input}
      onkeydown={handleKeydown}
      oncompositionstart={handleCompositionStart}
      oncompositionend={handleCompositionEnd}
      disabled={disabled || isWaiting}
      aria-label="Decree input"
    />
    <button
      class="decree-send"
      onclick={send}
      disabled={disabled || isWaiting || !input.trim()}
      type="button"
    >
      {#if isWaiting}
        <span class="spinner" aria-hidden="true"></span>
      {/if}
      <span>{isWaiting ? 'Working...' : 'Send'}</span>
    </button>
  </div>
</footer>

<style>
  .decree-input {
    padding: 8px 16px 12px;
    background: rgba(45, 43, 85, 0.95);
    border-top: 1px solid rgba(180, 190, 254, 0.15);
    flex-shrink: 0;
  }

  .decree-bar {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .decree-field {
    flex: 1;
    height: 36px;
    padding: 0 12px;
    border-radius: 18px;
    border: 1px solid rgba(180, 190, 254, 0.2);
    background: rgba(30, 30, 46, 0.8);
    color: #cdd6f4;
    font-size: var(--ui-font-md, 12px);
    font-family: var(--font-mono, monospace);
    outline: none;
    transition: border-color 0.2s ease;
  }

  .decree-field:focus {
    border-color: rgba(180, 190, 254, 0.5);
    box-shadow: 0 0 0 2px rgba(180, 190, 254, 0.1);
  }

  .decree-field::placeholder {
    color: rgba(108, 112, 134, 0.7);
  }

  .decree-field:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .decree-send {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 36px;
    padding: 0 16px;
    border-radius: 18px;
    border: 1px solid rgba(180, 190, 254, 0.3);
    background: rgba(180, 190, 254, 0.15);
    color: #b4befe;
    font-weight: 600;
    font-size: var(--ui-font-sm, 11px);
    cursor: pointer;
    transition: background 0.2s ease, border-color 0.2s ease;
    flex-shrink: 0;
  }

  .decree-send:hover:not(:disabled) {
    background: rgba(180, 190, 254, 0.25);
    border-color: rgba(180, 190, 254, 0.5);
  }

  .decree-send:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(180, 190, 254, 0.3);
    border-top-color: #b4befe;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
