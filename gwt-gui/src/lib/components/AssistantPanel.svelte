<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import type { AssistantState, DashboardData } from "../types";
  import AssistantDashboard from "./AssistantDashboard.svelte";

  let assistantState: AssistantState | null = $state(null);
  let dashboard: DashboardData | null = $state(null);
  let inputText: string = $state("");
  let isComposing: boolean = $state(false);
  let messagesEndRef: HTMLDivElement | undefined = $state();

  function scrollToBottom() {
    messagesEndRef?.scrollIntoView({ behavior: "smooth" });
  }

  async function sendMessage() {
    if (isComposing) return;
    const text = inputText.trim();
    if (!text) return;
    inputText = "";
    try {
      await invoke("assistant_send_message", { message: text });
    } catch (err) {
      console.error("Failed to send assistant message:", err);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !isComposing) {
      e.preventDefault();
      sendMessage();
    }
  }

  onMount(() => {
    invoke<AssistantState>("assistant_get_state")
      .then((state) => {
        assistantState = state;
      })
      .catch(() => {
        // AI backend not available (e.g. test environment)
      });

    let unlistenState: Promise<() => void> | undefined;
    let unlistenDashboard: Promise<() => void> | undefined;

    try {
      unlistenState = listen<AssistantState>(
        "assistant-state-updated",
        (event) => {
          assistantState = event.payload;
        },
      );

      unlistenDashboard = listen<DashboardData>(
        "assistant-dashboard-updated",
        (event) => {
          dashboard = event.payload;
        },
      );
    } catch {
      // Event listener setup failed (e.g. test environment)
    }

    return () => {
      unlistenState?.then((fn) => fn()).catch(() => {});
      unlistenDashboard?.then((fn) => fn()).catch(() => {});
    };
  });

  $effect(() => {
    if (assistantState?.messages) {
      // Scroll to bottom when messages change
      requestAnimationFrame(() => scrollToBottom());
    }
  });
</script>

<div class="assistant-panel">
  <AssistantDashboard {dashboard} />

  <div class="chat-area">
    <div class="messages">
      {#if assistantState}
        {#each assistantState.messages as msg}
          <div
            class="message"
            class:user={msg.role === "user"}
            class:assistant={msg.role === "assistant"}
            class:system={msg.role === "system" || msg.role === "tool"}
            class:thought={msg.kind === "thought"}
            class:action={msg.kind === "action"}
          >
            <div class="message-content">
              {#if msg.kind === "action"}
                <span class="action-icon">&#9654;</span>
              {/if}
              {msg.content}
            </div>
          </div>
        {/each}

        {#if assistantState.is_thinking}
          <div class="message assistant thinking">
            <div class="spinner"></div>
            <span>Thinking...</span>
          </div>
        {/if}
      {:else}
        <div class="placeholder-msg">Loading assistant...</div>
      {/if}
      <div bind:this={messagesEndRef}></div>
    </div>

    <div class="input-area">
      {#if assistantState && !assistantState.ai_ready}
        <div class="ai-not-ready">AI not configured</div>
      {/if}
      <div class="input-row">
        <textarea
          class="message-input"
          placeholder="Type a message..."
          bind:value={inputText}
          onkeydown={handleKeydown}
          oncompositionstart={() => (isComposing = true)}
          oncompositionend={() => (isComposing = false)}
          disabled={!assistantState?.ai_ready}
          rows={1}
        ></textarea>
        <button
          class="send-btn"
          type="button"
          onclick={sendMessage}
          disabled={!assistantState?.ai_ready || !inputText.trim()}
        >
          Send
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .assistant-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .chat-area {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .message {
    max-width: 80%;
    padding: 8px 12px;
    border-radius: 8px;
    font-size: var(--ui-font-sm);
    line-height: 1.5;
    word-break: break-word;
  }

  .message.user {
    align-self: flex-end;
    background-color: var(--accent);
    color: var(--bg-primary);
  }

  .message.assistant {
    align-self: flex-start;
    background-color: var(--bg-secondary);
    color: var(--text-primary);
  }

  .message.system {
    align-self: center;
    background-color: transparent;
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
    text-align: center;
  }

  .message.thought {
    font-style: italic;
    opacity: 0.8;
  }

  .message.action .message-content {
    font-weight: 600;
  }

  .action-icon {
    margin-right: 4px;
  }

  .message.thinking {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--border-color);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .placeholder-msg {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    text-align: center;
    padding: 24px;
  }

  .input-area {
    border-top: 1px solid var(--border-color);
    padding: 8px 12px;
  }

  .ai-not-ready {
    font-size: var(--ui-font-xs);
    color: var(--yellow);
    margin-bottom: 4px;
  }

  .input-row {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }

  .message-input {
    flex: 1;
    resize: none;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 10px;
    font-size: var(--ui-font-sm);
    font-family: inherit;
    background-color: var(--bg-primary);
    color: var(--text-primary);
    outline: none;
  }

  .message-input:focus {
    border-color: var(--accent);
  }

  .message-input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .send-btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    background-color: var(--accent);
    color: var(--bg-primary);
    font-size: var(--ui-font-sm);
    font-family: inherit;
    cursor: pointer;
    white-space: nowrap;
  }

  .send-btn:hover:not(:disabled) {
    opacity: 0.9;
  }

  .send-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
