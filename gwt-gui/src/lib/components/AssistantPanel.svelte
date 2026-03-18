<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import type { AssistantMessage, AssistantState, DashboardData } from "../types";
  import AssistantDashboard from "./AssistantDashboard.svelte";
  import MarkdownRenderer from "./MarkdownRenderer.svelte";

  interface Props {
    isActive?: boolean;
    projectPath?: string | null;
  }

  let { isActive = true, projectPath = null }: Props = $props();
  let assistantState: AssistantState | null = $state(null);
  let dashboard: DashboardData | null = $state(null);
  let inputText: string = $state("");
  let isComposing: boolean = $state(false);
  let messageInputRef: HTMLTextAreaElement | undefined = $state();
  let sentInputHistory: string[] = $state([]);
  let historyIndex: number | null = $state(null);
  let draftBeforeHistory: string | null = $state(null);
  let messagesEndRef: HTMLDivElement | undefined = $state();
  let hasMounted = false;
  let previousProjectPath: string | null = null;
  let previousIsActive = false;

  function scrollToBottom() {
    messagesEndRef?.scrollIntoView({ behavior: "smooth" });
  }

  async function loadAssistantState(): Promise<AssistantState | null> {
    try {
      return await invoke<AssistantState>("assistant_get_state");
    } catch {
      return null;
    }
  }

  async function initializeAssistant(): Promise<AssistantState | null> {
    const state = await loadAssistantState();
    if (!state) {
      return null;
    }

    assistantState = state;
    if (!state.aiReady || state.sessionId) {
      return state;
    }

    try {
      assistantState = {
        ...state,
        isThinking: true,
        startupStatus: "analyzing",
        startupSummaryReady: false,
      };
      const startedState = await invoke<AssistantState>("assistant_start");
      assistantState = startedState;
      return startedState;
    } catch (err) {
      console.error("Failed to start assistant session:", err);
      return state;
    }
  }

  async function loadDashboard() {
    try {
      dashboard = await invoke<DashboardData>("assistant_get_dashboard");
    } catch {
      // AI backend not available (e.g. test environment)
    }
  }

  async function sendMessage() {
    if (isComposing || assistantState?.isThinking) return;
    const text = inputText.trim();
    if (!text) return;
    let previousState: AssistantState | null = null;

    try {
      const readyState = await initializeAssistant();
      if (!readyState?.sessionId) {
        return;
      }

      previousState = assistantState ?? readyState;
      const optimisticState: AssistantState = {
        ...previousState,
        isThinking: true,
        messages: [
          ...previousState.messages,
          createOptimisticUserMessage(text),
        ],
      };

      inputText = "";
      assistantState = optimisticState;
      assistantState = await invoke<AssistantState>("assistant_send_message", {
        input: text,
      });
      sentInputHistory = [...sentInputHistory, text];
      historyIndex = null;
      draftBeforeHistory = null;
    } catch (err) {
      if (previousState) {
        assistantState = previousState;
      }
      inputText = text;
      historyIndex = null;
      draftBeforeHistory = null;
      console.error("Failed to send assistant message:", err);
    }
  }

  function createOptimisticUserMessage(content: string): AssistantMessage {
    return {
      role: "user",
      kind: "text",
      content,
      timestamp: Date.now(),
    };
  }

  function isImeEnterKeydown(event: KeyboardEvent): boolean {
    const legacyKeyCode = event.keyCode || event.which;
    return event.isComposing || isComposing || legacyKeyCode === 229;
  }

  function scheduleCaretToEnd() {
    requestAnimationFrame(() => {
      if (!messageInputRef) return;
      const end = messageInputRef.value.length;
      messageInputRef.setSelectionRange(end, end);
    });
  }

  function isCaretOnFirstLine(value: string, position: number): boolean {
    return !value.slice(0, position).includes("\n");
  }

  function isCaretOnLastLine(value: string, position: number): boolean {
    return !value.slice(position).includes("\n");
  }

  function handleHistoryKeydown(event: KeyboardEvent): boolean {
    if (isComposing || event.isComposing || !messageInputRef) {
      return false;
    }
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
      return false;
    }

    const { selectionStart, selectionEnd, value } = messageInputRef;
    if (selectionStart === null || selectionEnd === null || selectionStart !== selectionEnd) {
      return false;
    }

    if (event.key === "ArrowUp") {
      if (!sentInputHistory.length || !isCaretOnFirstLine(value, selectionStart)) {
        return false;
      }

      event.preventDefault();
      if (historyIndex === null) {
        draftBeforeHistory = inputText;
        historyIndex = sentInputHistory.length - 1;
      } else if (historyIndex > 0) {
        historyIndex -= 1;
      }

      inputText = sentInputHistory[historyIndex ?? sentInputHistory.length - 1];
      scheduleCaretToEnd();
      return true;
    }

    if (event.key === "ArrowDown") {
      if (historyIndex === null || !isCaretOnLastLine(value, selectionStart)) {
        return false;
      }

      event.preventDefault();
      const nextIndex = historyIndex + 1;
      if (nextIndex >= sentInputHistory.length) {
        historyIndex = null;
        inputText = draftBeforeHistory ?? "";
        draftBeforeHistory = null;
      } else {
        historyIndex = nextIndex;
        inputText = sentInputHistory[nextIndex];
      }
      scheduleCaretToEnd();
      return true;
    }

    return false;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (handleHistoryKeydown(e)) {
      return;
    }

    if (e.key === "Enter" && !e.shiftKey) {
      if (isImeEnterKeydown(e)) {
        return;
      }

      e.preventDefault();
      void sendMessage();
    }
  }

  onMount(() => {
    previousProjectPath = projectPath;
    previousIsActive = isActive;
    hasMounted = true;

    void initializeAssistant();
    void loadDashboard();

    let unlistenState: Promise<() => void> | undefined;
    let unlistenDashboard: Promise<() => void> | undefined;
    let unlistenLaunchFinished: Promise<() => void> | undefined;
    let unlistenTerminalClosed: Promise<() => void> | undefined;

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

      unlistenLaunchFinished = listen("launch-finished", () => {
        void loadDashboard();
      });

      unlistenTerminalClosed = listen("terminal-closed", () => {
        void loadDashboard();
      });
    } catch {
      // Event listener setup failed (e.g. test environment)
    }

    return () => {
      unlistenState?.then((fn) => fn()).catch(() => {});
      unlistenDashboard?.then((fn) => fn()).catch(() => {});
      unlistenLaunchFinished?.then((fn) => fn()).catch(() => {});
      unlistenTerminalClosed?.then((fn) => fn()).catch(() => {});
    };
  });

  $effect(() => {
    if (!hasMounted) {
      return;
    }

    const nextProjectPath = projectPath;
    const nextIsActive = isActive;
    const projectChanged = nextProjectPath !== previousProjectPath;
    const becameActive = nextIsActive && !previousIsActive;

    if (projectChanged) {
      assistantState = null;
      dashboard = null;
      inputText = "";
      isComposing = false;
      sentInputHistory = [];
      historyIndex = null;
      draftBeforeHistory = null;
      void initializeAssistant();
      void loadDashboard();
    } else if (becameActive) {
      void loadDashboard();
    }

    previousProjectPath = nextProjectPath;
    previousIsActive = nextIsActive;
  });

  $effect(() => {
    if (assistantState?.messages) {
      // Scroll to bottom when messages change
      requestAnimationFrame(() => scrollToBottom());
    }
  });
</script>

<div class="assistant-panel">
  <AssistantDashboard {dashboard} {assistantState} />

  <div class="chat-area">
    <div class="messages">
      {#if assistantState}
        {#each assistantState.messages as msg}
          <div
            class="message"
            class:user={msg.role === "user"}
            class:assistant={msg.role === "assistant"}
            class:system={msg.role === "system" || msg.role === "tool"}
            class:tool-use={msg.kind === "tool_use"}
          >
            {#if msg.role === "assistant" && msg.kind === "text"}
              <MarkdownRenderer text={msg.content} className="assistant-message-markdown" />
            {:else}
              <div class="message-content">
                {#if msg.kind === "tool_use"}
                  <span class="action-icon">&#9654;</span>
                {/if}
                {msg.content}
              </div>
            {/if}
          </div>
        {/each}

        {#if assistantState.isThinking}
          <div class="message assistant thinking">
            <div class="spinner"></div>
            <span>
              {assistantState.startupStatus === "analyzing"
                ? "Analyzing project..."
                : "Thinking..."}
            </span>
          </div>
        {/if}
      {:else}
        <div class="placeholder-msg">Loading assistant...</div>
      {/if}
      <div bind:this={messagesEndRef}></div>
    </div>

    <div class="input-area">
      {#if assistantState && !assistantState.aiReady}
        <div class="ai-not-ready">AI not configured</div>
      {/if}
      <div class="input-row">
        <textarea
          bind:this={messageInputRef}
          class="message-input"
          placeholder="Type a message..."
          bind:value={inputText}
          onkeydown={handleKeydown}
          oncompositionstart={() => (isComposing = true)}
          oncompositionend={() => (isComposing = false)}
          disabled={
            !assistantState?.aiReady ||
            !assistantState?.sessionId ||
            assistantState?.isThinking
          }
          rows={1}
        ></textarea>
        <button
          class="send-btn"
          type="button"
          onclick={sendMessage}
          disabled={
            !assistantState?.aiReady ||
            !assistantState?.sessionId ||
            assistantState?.isThinking ||
            !inputText.trim()
          }
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

  .message.tool-use {
    font-style: italic;
    opacity: 0.8;
  }

  .message.tool-use .message-content {
    font-weight: 600;
  }

  .message-content {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .message.assistant :global(.assistant-message-markdown) {
    color: var(--text-primary);
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
