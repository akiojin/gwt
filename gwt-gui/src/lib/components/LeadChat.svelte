<script lang="ts">
  import type { LeadMessage } from "../types";

  interface Props {
    messages: LeadMessage[];
    isWaiting: boolean;
    onSend: (text: string) => void;
  }

  let { messages, isWaiting, onSend }: Props = $props();

  let input = $state("");
  let isComposing = $state(false);
  let ignoreEnterAfterComposition = $state(false);
  let chatEl: HTMLDivElement | undefined = $state();
  let lastMessageCount = $state(0);

  $effect(() => {
    const count = messages.length;
    if (chatEl && count !== lastMessageCount) {
      chatEl.scrollTop = chatEl.scrollHeight;
      lastMessageCount = count;
    }
  });

  function formatTime(ts: number): string {
    const d = new Date(ts);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function send() {
    if (isWaiting) return;
    const text = input.trim();
    if (!text) return;
    onSend(text);
    input = "";
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      if (isComposing || ignoreEnterAfterComposition || event.isComposing) {
        event.preventDefault();
        return;
      }
      if (!event.shiftKey) {
        event.preventDefault();
        send();
      }
      return;
    }
    if (event.key === "Process" && (isComposing || event.isComposing)) {
      event.preventDefault();
    }
  }

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

  const kindBadgeKinds = ["thought", "action", "observation", "error", "progress"] as const;

  function showKindBadge(msg: LeadMessage): boolean {
    return kindBadgeKinds.includes(msg.kind as (typeof kindBadgeKinds)[number]);
  }
</script>

<section class="lead-chat">
  <div class="lead-chat-messages" bind:this={chatEl}>
    {#if messages.length === 0}
      <div class="lead-empty">Start a conversation...</div>
    {:else}
      {#each messages as msg}
        <div class="lead-message {msg.role} {msg.kind}">
          <div class="lead-bubble">
            {#if showKindBadge(msg)}
              <span class="lead-kind-badge">{msg.kind.toUpperCase()}</span>
            {/if}
            <div class="lead-content">{msg.content}</div>
            <span class="lead-timestamp">{formatTime(msg.timestamp)}</span>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <footer class="lead-input">
    <textarea
      placeholder="Type a message and press Enter..."
      bind:value={input}
      onkeydown={onKeydown}
      oncompositionstart={onCompositionStart}
      oncompositionend={onCompositionEnd}
      disabled={isWaiting}
      rows="3"
    ></textarea>
    <button
      class="send-btn"
      onclick={send}
      disabled={isWaiting || !input.trim()}
    >
      {#if isWaiting}
        <span class="spinner" aria-hidden="true"></span>
      {/if}
      <span>{isWaiting ? "Working..." : "Send"}</span>
    </button>
  </footer>
</section>

<style>
  .lead-chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    gap: 12px;
  }

  .lead-chat-messages {
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

  .lead-empty {
    color: var(--text-muted);
    font-size: 14px;
  }

  .lead-message {
    display: flex;
    width: 100%;
  }

  .lead-message.user {
    justify-content: flex-end;
  }

  .lead-message.assistant {
    justify-content: flex-start;
  }

  .lead-message.system,
  .lead-message.thought,
  .lead-message.action,
  .lead-message.observation,
  .lead-message.error,
  .lead-message.progress {
    justify-content: flex-start;
  }

  .lead-bubble {
    max-width: 72%;
    padding: 8px 10px;
    border-radius: 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    position: relative;
  }

  .lead-message.user .lead-bubble {
    background: rgba(64, 160, 255, 0.12);
    border-color: rgba(64, 160, 255, 0.4);
  }

  .lead-message.assistant .lead-bubble {
    background: rgba(46, 196, 182, 0.12);
    border-color: rgba(46, 196, 182, 0.4);
  }

  .lead-message.thought .lead-bubble {
    background: rgba(137, 180, 250, 0.12);
    border-color: rgba(137, 180, 250, 0.4);
  }

  .lead-message.action .lead-bubble {
    background: rgba(250, 227, 175, 0.12);
    border-color: rgba(250, 227, 175, 0.4);
  }

  .lead-message.observation .lead-bubble {
    background: rgba(166, 227, 161, 0.12);
    border-color: rgba(166, 227, 161, 0.4);
  }

  .lead-message.error .lead-bubble {
    background: rgba(243, 139, 168, 0.12);
    border-color: rgba(243, 139, 168, 0.4);
  }

  .lead-message.progress .lead-bubble {
    background: rgba(180, 190, 254, 0.12);
    border-color: rgba(180, 190, 254, 0.4);
  }

  .lead-kind-badge {
    display: inline-block;
    text-transform: uppercase;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    color: var(--text-muted);
    margin-bottom: 4px;
    padding: 1px 6px;
    border-radius: 4px;
    background: rgba(128, 128, 128, 0.15);
  }

  .lead-content {
    white-space: pre-wrap;
    color: var(--text-primary);
    font-size: 13px;
  }

  .lead-timestamp {
    display: block;
    text-align: right;
    font-size: 10px;
    color: var(--text-muted);
    margin-top: 4px;
  }

  .lead-input {
    display: flex;
    gap: 12px;
  }

  .lead-input textarea {
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
    cursor: pointer;
  }

  .send-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.4);
    border-top-color: var(--text-primary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
