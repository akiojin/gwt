<script lang="ts">
  import { onMount } from "svelte";
  import {
    getAgentInputProfileOrDefault,
    buildSendBytes,
    buildQueueBytes,
    buildInterruptBytes,
    buildImageReference,
    type AgentInputProfile,
  } from "../terminal/agentInputProfile";
  import { InputHistory } from "../terminal/inputHistory";
  import { registerInputFieldTarget } from "../voice/inputTargetRegistry";

  let {
    paneId,
    agentId,
    active = false,
    onFocusTerminal,
  }: {
    paneId: string;
    agentId: string;
    active?: boolean;
    onFocusTerminal?: () => void;
  } = $props();

  let textareaEl: HTMLTextAreaElement | undefined = $state(undefined);
  let input = $state("");
  let isComposing = $state(false);
  let ignoreEnterAfterComposition = $state(false);
  let attachedImages: { id: string; path: string; name: string }[] = $state([]);
  let history: InputHistory | undefined = $state(undefined);
  let profile: AgentInputProfile = $derived(getAgentInputProfileOrDefault(agentId));

  onMount(() => {
    history = new InputHistory(paneId);
  });

  // Register textarea as voice input target when it's available
  $effect(() => {
    if (!textareaEl) return;
    const unregister = registerInputFieldTarget(paneId, textareaEl);
    return () => {
      unregister();
    };
  });

  // Auto-focus input field when tab becomes active
  $effect(() => {
    if (active && textareaEl) {
      requestAnimationFrame(() => {
        textareaEl?.focus();
      });
    }
  });

  // Auto-resize textarea
  $effect(() => {
    void input;
    if (!textareaEl) return;
    textareaEl.style.height = "auto";
    textareaEl.style.height = `${textareaEl.scrollHeight}px`;
  });

  function onCompositionStart() {
    isComposing = true;
    ignoreEnterAfterComposition = false;
  }

  function onCompositionEnd() {
    isComposing = false;
    ignoreEnterAfterComposition = true;
    setTimeout(() => {
      ignoreEnterAfterComposition = false;
    }, 0);
  }

  function onKeydown(event: KeyboardEvent) {
    // IME guard
    if (isComposing || ignoreEnterAfterComposition || event.isComposing) {
      return;
    }

    // Ctrl+Enter: Send
    if (event.key === "Enter" && event.ctrlKey && !event.shiftKey) {
      event.preventDefault();
      send();
      return;
    }

    // Ctrl+Shift+Enter: Queue
    if (event.key === "Enter" && event.ctrlKey && event.shiftKey) {
      event.preventDefault();
      queue();
      return;
    }

    // Escape: Interrupt
    if (event.key === "Escape") {
      event.preventDefault();
      interrupt();
      return;
    }

    // Tab: fallback to xterm.js
    if (event.key === "Tab" && !event.ctrlKey && !event.shiftKey) {
      event.preventDefault();
      tabFallback();
      return;
    }

    // Up: history back (when at first line)
    if (event.key === "ArrowUp" && !event.ctrlKey && !event.shiftKey) {
      if (isAtFirstLine()) {
        event.preventDefault();
        navigateHistory("back");
        return;
      }
    }

    // Down: history forward (when at last line)
    if (event.key === "ArrowDown" && !event.ctrlKey && !event.shiftKey) {
      if (isAtLastLine()) {
        event.preventDefault();
        navigateHistory("forward");
        return;
      }
    }
  }

  function isAtFirstLine(): boolean {
    if (!textareaEl) return true;
    const pos = textareaEl.selectionStart;
    return !input.substring(0, pos).includes("\n");
  }

  function isAtLastLine(): boolean {
    if (!textareaEl) return true;
    const pos = textareaEl.selectionEnd;
    return !input.substring(pos).includes("\n");
  }

  function navigateHistory(direction: "back" | "forward") {
    if (!history) return;
    const entry = direction === "back" ? history.back() : history.forward();
    input = entry;
    // Move cursor to end
    requestAnimationFrame(() => {
      if (textareaEl) {
        textareaEl.selectionStart = textareaEl.selectionEnd = input.length;
      }
    });
  }

  async function send() {
    const text = buildFullText();
    if (!text.trim() && attachedImages.length === 0) return;

    const bytes = buildSendBytes(profile, text);
    await writeBytes(bytes);

    if (input.trim()) {
      history?.push(input);
    }
    input = "";
    attachedImages = [];
  }

  async function queue() {
    const text = buildFullText();
    if (!text.trim()) return;

    const bytes = buildQueueBytes(profile, text);
    if (!bytes) return;

    await writeBytes(bytes);
    if (input.trim()) {
      history?.push(input);
    }
    input = "";
    attachedImages = [];
  }

  async function interrupt() {
    const bytes = buildInterruptBytes(profile);
    await writeBytes(bytes);
  }

  function buildFullText(): string {
    let text = input;
    if (attachedImages.length > 0) {
      const refs = attachedImages
        .map((img) => buildImageReference(profile, img.path))
        .filter((r): r is string => r !== null);
      if (refs.length > 0) {
        text = text ? `${refs.join(" ")} ${text}` : refs.join(" ");
      }
    }
    return text;
  }

  async function tabFallback() {
    if (input.trim()) {
      // Write current text to PTY without send suffix
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(input));
      await writeBytes(bytes);
      input = "";
    }
    onFocusTerminal?.();
  }

  async function writeBytes(data: number[]) {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("write_terminal", { paneId, data });
    } catch (err) {
      console.error("Failed to write to terminal:", err);
    }
  }

  // Image handling
  async function handlePaste(event: ClipboardEvent) {
    const items = event.clipboardData?.items;
    if (!items) return;

    for (const item of items) {
      if (item.type.startsWith("image/")) {
        event.preventDefault();
        const blob = item.getAsFile();
        if (blob) {
          await saveAndAttachImage(blob);
        }
        return;
      }
    }
    // Text paste is handled natively by textarea
  }

  async function handleDrop(event: DragEvent) {
    event.preventDefault();
    const files = event.dataTransfer?.files;
    if (!files) return;

    for (const file of files) {
      if (file.type.startsWith("image/")) {
        await saveAndAttachImage(file);
      }
    }
  }

  function handleDragOver(event: DragEvent) {
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "copy";
    }
  }

  async function openFilePicker() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: true,
        filters: [
          { name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp"] },
        ],
      });
      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        for (const filePath of paths) {
          const name = typeof filePath === "string" ? filePath.split(/[\\/]/).pop() ?? "image" : "image";
          const path = typeof filePath === "string" ? filePath : "";
          if (path) {
            attachedImages = [
              ...attachedImages,
              { id: crypto.randomUUID(), path, name },
            ];
          }
        }
      }
    } catch (err) {
      console.error("Failed to open file picker:", err);
    }
  }

  async function saveAndAttachImage(blob: File | Blob) {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const arrayBuffer = await blob.arrayBuffer();
      const bytes = Array.from(new Uint8Array(arrayBuffer));
      const format = blob.type.split("/")[1] || "png";
      const path = await invoke<string>("save_clipboard_image", {
        paneId,
        data: bytes,
        format,
      });
      attachedImages = [
        ...attachedImages,
        { id: crypto.randomUUID(), path, name: `clipboard.${format}` },
      ];
    } catch (err) {
      console.error("Failed to save clipboard image:", err);
    }
  }

  function removeImage(id: string) {
    attachedImages = attachedImages.filter((img) => img.id !== id);
  }

  function startVoiceInput() {
    // Dispatch event to trigger the existing voice input system
    window.dispatchEvent(new CustomEvent("gwt-voice-toggle"));
  }
</script>

<div
  class="terminal-input-field"
  ondrop={handleDrop}
  ondragover={handleDragOver}
  role="form"
>
  {#if attachedImages.length > 0}
    <div class="image-thumbnails">
      {#each attachedImages as img (img.id)}
        <div class="thumbnail">
          <span class="thumbnail-name" title={img.path}>{img.name}</span>
          <button
            class="thumbnail-remove"
            type="button"
            onclick={() => removeImage(img.id)}
            title="Remove"
          >&times;</button>
        </div>
      {/each}
    </div>
  {/if}

  <div class="input-row">
    <div class="input-actions-left">
      <button
        class="action-btn"
        type="button"
        onclick={startVoiceInput}
        title="Voice input"
      >
        <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
          <path d="M8 1a2 2 0 0 0-2 2v5a2 2 0 1 0 4 0V3a2 2 0 0 0-2-2z"/>
          <path d="M4.5 7.5a.5.5 0 0 0-1 0A4.5 4.5 0 0 0 7.5 12v2H6a.5.5 0 0 0 0 1h4a.5.5 0 0 0 0-1H8.5v-2A4.5 4.5 0 0 0 12.5 7.5a.5.5 0 0 0-1 0A3.5 3.5 0 1 1 4.5 7.5z"/>
        </svg>
      </button>
      <button
        class="action-btn"
        type="button"
        onclick={openFilePicker}
        title="Attach image"
      >
        <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
          <path d="M14 4.5V14a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V2a2 2 0 0 1 2-2h5.5L14 4.5zm-3 0A1.5 1.5 0 0 1 9.5 3V1H4a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V4.5h-2z"/>
        </svg>
      </button>
    </div>

    <textarea
      bind:this={textareaEl}
      bind:value={input}
      placeholder="Type message... (Ctrl+Enter to send)"
      rows="1"
      onkeydown={onKeydown}
      oncompositionstart={onCompositionStart}
      oncompositionend={onCompositionEnd}
      onpaste={handlePaste}
    ></textarea>

    <div class="input-actions-right">
      <button
        class="action-btn send-btn"
        type="button"
        onclick={() => send()}
        disabled={!input.trim() && attachedImages.length === 0}
        title="Send (Ctrl+Enter)"
      >
        <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
          <path d="M1.724 1.053a.5.5 0 0 1 .545-.065l12 6a.5.5 0 0 1 0 .894l-12 6a.5.5 0 0 1-.723-.494V8.5h6a.5.5 0 0 0 0-1h-6V1.553a.5.5 0 0 1 .178-.5z"/>
        </svg>
      </button>
      <button
        class="action-btn stop-btn"
        type="button"
        onclick={() => interrupt()}
        title="Stop (Escape)"
      >
        <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
          <rect x="3" y="3" width="10" height="10" rx="1"/>
        </svg>
      </button>
    </div>
  </div>
</div>

<style>
  .terminal-input-field {
    background: var(--bg-secondary);
    border-top: 1px solid var(--border-color);
    padding: 6px 8px;
    flex-shrink: 0;
  }

  .image-thumbnails {
    display: flex;
    gap: 6px;
    padding: 4px 0;
    overflow-x: auto;
    flex-wrap: wrap;
  }

  .thumbnail {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    background: var(--bg-surface, var(--bg-primary));
    border: 1px solid var(--border-color);
    border-radius: 4px;
    font-size: 11px;
    color: var(--text-secondary);
    max-width: 200px;
  }

  .thumbnail-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .thumbnail-remove {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    padding: 0 2px;
    line-height: 1;
  }

  .thumbnail-remove:hover {
    color: var(--red);
  }

  .input-row {
    display: flex;
    align-items: flex-end;
    gap: 6px;
  }

  .input-actions-left,
  .input-actions-right {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
    padding-bottom: 2px;
  }

  .action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
  }

  .action-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .action-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .send-btn {
    color: var(--accent, var(--blue));
  }

  .stop-btn {
    color: var(--red, #f38ba8);
  }

  textarea {
    flex: 1;
    min-height: 28px;
    max-height: 200px;
    resize: none;
    overflow-y: auto;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-primary);
    color: var(--text-primary);
    padding: 4px 8px;
    font-size: 13px;
    font-family: "JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace;
    line-height: 1.4;
  }

  textarea:focus {
    outline: none;
    border-color: var(--accent, var(--blue));
  }

  textarea::placeholder {
    color: var(--text-muted);
  }
</style>
