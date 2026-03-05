<script lang="ts">
  interface Props {
    agentType: 'claude' | 'codex' | 'gemini';
    status: 'starting' | 'running' | 'completed' | 'error';
    name?: string;
    onclick?: () => void;
  }

  let { agentType, status, name, onclick }: Props = $props();

  const colors: Record<string, string> = {
    claude: '#f9e2af',
    codex: '#94e2d5',
    gemini: '#cba6f7',
  };

  const statusIcons: Record<string, string> = {
    completed: '\u2713',
    error: '!',
  };

  let color = $derived(colors[agentType] ?? '#b4befe');
  let icon = $derived(statusIcons[status] ?? '');
  let label = $derived(name ?? `${agentType} agent`);
</script>

<button
  class="agent-avatar"
  class:starting={status === 'starting'}
  class:running={status === 'running'}
  class:completed={status === 'completed'}
  class:error={status === 'error'}
  style:--agent-color={color}
  onclick={onclick}
  title={`${label} (${status})`}
  aria-label={`${label}: ${status}`}
  type="button"
>
  {#if icon}
    <span class="icon">{icon}</span>
  {/if}
</button>

<style>
  .agent-avatar {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--agent-color);
    border: 2px solid transparent;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    font-weight: 700;
    color: #1e1e2e;
    padding: 0;
    transition: transform 0.15s ease;
    flex-shrink: 0;
  }

  .agent-avatar:hover {
    transform: scale(1.2);
  }

  .agent-avatar:focus-visible {
    outline: 2px solid var(--agent-color);
    outline-offset: 2px;
  }

  .icon {
    line-height: 1;
  }

  /* starting: bounce */
  .starting {
    animation: bounce 0.6s ease infinite;
  }

  @keyframes bounce {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-4px); }
  }

  /* running: slide */
  .running {
    animation: slide 1.2s ease-in-out infinite alternate;
  }

  @keyframes slide {
    0% { transform: translateX(-2px); }
    100% { transform: translateX(2px); }
  }

  /* error: shake + red ring */
  .error {
    border-color: #f38ba8;
    animation: shake 0.4s ease infinite;
  }

  @keyframes shake {
    0%, 100% { transform: translateX(0); }
    25% { transform: translateX(-2px); }
    75% { transform: translateX(2px); }
  }
</style>
