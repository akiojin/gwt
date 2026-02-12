<script lang="ts">
  import type { AgentModeState } from "../types";
  import { onMount } from "svelte";

  let state: AgentModeState = {
    messages: [],
    ai_ready: false,
    ai_error: null,
    last_error: null,
    is_waiting: false,
    session_name: "Agent Mode",
    llm_call_count: 0,
    estimated_tokens: 0,
  };

  let input = "";
  let sending = false;
  let isComposing = false;

  function toErrorMessage(err: unknown): string {
    if (!err) return "Unknown error";
    if (typeof err === "string") return err;
    if (typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  async function refreshState() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      state = await invoke<AgentModeState>("get_agent_mode_state_cmd");
    } catch (err) {
      state = {
        ...state,
        last_error: toErrorMessage(err),
      };
    }
  }

  async function sendMessage() {
    if (sending) return;
    const text = input.trim();
    if (!text) return;
    sending = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      state = await invoke<AgentModeState>("send_agent_mode_message", { input: text });
      input = "";
    } catch (err) {
      state = {
        ...state,
        last_error: toErrorMessage(err),
      };
    } finally {
      sending = false;
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (isComposing || event.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void sendMessage();
    }
  }

  function onCompositionStart() {
    isComposing = true;
  }

  function onCompositionEnd() {
    isComposing = false;
  }

  onMount(() => {
    void refreshState();
  });
</script>

<section class="agent-mode">
  <header class="agent-header">
    <div class="agent-title">{state.session_name ?? "Agent Mode"}</div>
    <div class="agent-stats">
      <span>LLM: {state.llm_call_count}</span>
      <span>Tokens: {state.estimated_tokens}</span>
    </div>
  </header>

  <div class="agent-chat">
    {#if state.last_error}
      <div class="agent-alert warn">{state.last_error}</div>
    {/if}
    {#if !state.ai_ready}
      <div class="agent-alert warn">
        {state.ai_error ?? "AI settings are required."}
      </div>
    {/if}
    {#if state.messages.length === 0}
      <div class="agent-empty">Describe your task to start.</div>
    {:else}
      {#each state.messages as msg}
        <div class={`agent-message ${msg.role} ${msg.kind ?? "message"}`}>
          <div class="agent-role">{msg.kind ?? msg.role}</div>
          <div class="agent-content">{msg.content}</div>
        </div>
      {/each}
    {/if}
  </div>

  <footer class="agent-input">
    <textarea
      placeholder="Type a task and press Enter..."
      bind:value={input}
      onkeydown={onKeydown}
      oncompositionstart={onCompositionStart}
      oncompositionend={onCompositionEnd}
      disabled={sending}
      rows="3"
    ></textarea>
    <button class="send-btn" onclick={sendMessage} disabled={sending || !input.trim()}>
      {#if sending || state.is_waiting}
        <span class="spinner" aria-hidden="true"></span>
      {/if}
      <span>{state.is_waiting ? "Working..." : "Send"}</span>
    </button>
  </footer>
</section>

<style>
  .agent-mode {
    display: flex;
    flex-direction: column;
    height: 100%;
    gap: 12px;
  }

  .agent-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 8px;
  }

  .agent-title {
    font-weight: 600;
    color: var(--text-primary);
  }

  .agent-stats {
    display: flex;
    gap: 12px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .agent-chat {
    flex: 1;
    overflow-y: auto;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .agent-empty {
    color: var(--text-muted);
    font-size: 14px;
  }

  .agent-alert {
    padding: 8px 10px;
    border-radius: 6px;
    background: rgba(255, 201, 71, 0.15);
    color: #b08300;
    font-size: 13px;
  }

  .agent-message {
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
  }

  .agent-message.user {
    border-color: rgba(64, 160, 255, 0.4);
  }

  .agent-message.assistant {
    border-color: rgba(46, 196, 182, 0.4);
  }

  .agent-message.system {
    border-color: rgba(240, 200, 90, 0.4);
  }

  .agent-message.tool {
    border-color: rgba(166, 227, 161, 0.4);
  }

  .agent-message.thought {
    background: rgba(137, 180, 250, 0.12);
  }

  .agent-message.action {
    background: rgba(250, 227, 175, 0.12);
  }

  .agent-message.observation {
    background: rgba(166, 227, 161, 0.12);
  }

  .agent-message.error {
    background: rgba(243, 139, 168, 0.12);
  }

  .agent-role {
    text-transform: uppercase;
    font-size: 10px;
    letter-spacing: 0.08em;
    color: var(--text-muted);
    margin-bottom: 4px;
  }

  .agent-content {
    white-space: pre-wrap;
    color: var(--text-primary);
    font-size: 13px;
  }

  .agent-input {
    display: flex;
    gap: 12px;
  }

  .agent-input textarea {
    flex: 1;
    resize: none;
    border-radius: 8px;
    border: 1px solid var(--border-color);
    background: var(--bg-surface);
    color: var(--text-primary);
    padding: 10px;
    font-size: 13px;
    font-family: "JetBrains Mono", "Fira Code", "SF Mono", "Menlo", monospace;
  }

  .send-btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
    background: var(--accent);
    color: var(--text-primary);
    font-weight: 600;
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.4);
    border-top-color: var(--text-primary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .send-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
