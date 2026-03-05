<script lang="ts">
  interface Props {
    status: 'idle' | 'thinking' | 'waiting_approval' | 'orchestrating' | 'error';
    onclick?: () => void;
  }

  let { status, onclick }: Props = $props();

  const statusLabels: Record<string, string> = {
    idle: 'Lead is idle',
    thinking: 'Lead is thinking...',
    waiting_approval: 'Awaiting approval',
    orchestrating: 'Orchestrating agents',
    error: 'Lead error',
  };
</script>

<button
  class="lead-orb-wrap"
  class:idle={status === 'idle'}
  class:thinking={status === 'thinking'}
  class:waiting_approval={status === 'waiting_approval'}
  class:orchestrating={status === 'orchestrating'}
  class:error={status === 'error'}
  onclick={onclick}
  aria-label={statusLabels[status] ?? 'Lead'}
  title={statusLabels[status] ?? 'Lead'}
  type="button"
>
  <div class="orb">
    {#if status === 'thinking'}
      <div class="dot-ring">
        <span class="dot"></span>
        <span class="dot"></span>
        <span class="dot"></span>
      </div>
    {/if}
    {#if status === 'waiting_approval'}
      <span class="approval-badge">!</span>
    {/if}
  </div>
  <span class="orb-label">Lead</span>
</button>

<style>
  .lead-orb-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    background: none;
    border: none;
    cursor: pointer;
    padding: 8px;
    transition: transform 0.2s ease;
  }

  .lead-orb-wrap:hover {
    transform: scale(1.05);
  }

  .lead-orb-wrap:focus-visible {
    outline: 2px solid #b4befe;
    outline-offset: 4px;
    border-radius: 50%;
  }

  .orb {
    position: relative;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: radial-gradient(circle at 35% 35%, #b4befe, #6c5ce7);
    box-shadow: 0 0 20px rgba(180, 190, 254, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .orb-label {
    font-size: var(--ui-font-xs, 10px);
    color: rgba(180, 190, 254, 0.7);
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }

  /* idle: breathing animation */
  .idle .orb {
    animation: breathe 3s ease-in-out infinite;
  }

  @keyframes breathe {
    0%, 100% { transform: scale(1); box-shadow: 0 0 20px rgba(180, 190, 254, 0.3); }
    50% { transform: scale(1.06); box-shadow: 0 0 32px rgba(180, 190, 254, 0.6); }
  }

  /* thinking: rotating dots */
  .thinking .orb {
    box-shadow: 0 0 24px rgba(116, 199, 236, 0.5);
    background: radial-gradient(circle at 35% 35%, #74c7ec, #4a6cf7);
  }

  .dot-ring {
    position: absolute;
    inset: -8px;
    animation: spin-ring 1.5s linear infinite;
  }

  .dot {
    position: absolute;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #74c7ec;
  }

  .dot:nth-child(1) { top: 0; left: 50%; transform: translateX(-50%); }
  .dot:nth-child(2) { bottom: 4px; left: 4px; }
  .dot:nth-child(3) { bottom: 4px; right: 4px; }

  @keyframes spin-ring {
    to { transform: rotate(360deg); }
  }

  /* waiting_approval: amber pulse */
  .waiting_approval .orb {
    background: radial-gradient(circle at 35% 35%, #fab387, #e67e22);
    animation: pulse-amber 1.2s ease-in-out infinite;
  }

  .approval-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #fab387;
    color: #1e1e2e;
    font-weight: 800;
    font-size: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: badge-pulse 1s ease-in-out infinite;
  }

  @keyframes pulse-amber {
    0%, 100% { box-shadow: 0 0 20px rgba(250, 179, 135, 0.4); }
    50% { box-shadow: 0 0 36px rgba(250, 179, 135, 0.7); }
  }

  @keyframes badge-pulse {
    0%, 100% { transform: scale(1); }
    50% { transform: scale(1.15); }
  }

  /* orchestrating: bright glow + downward radiance */
  .orchestrating .orb {
    background: radial-gradient(circle at 35% 35%, #a6e3a1, #40c057);
    box-shadow:
      0 0 30px rgba(166, 227, 161, 0.6),
      0 16px 40px rgba(166, 227, 161, 0.3);
    animation: glow-orchestrate 2s ease-in-out infinite;
  }

  @keyframes glow-orchestrate {
    0%, 100% { box-shadow: 0 0 30px rgba(166, 227, 161, 0.5), 0 16px 40px rgba(166, 227, 161, 0.2); }
    50% { box-shadow: 0 0 48px rgba(166, 227, 161, 0.8), 0 24px 56px rgba(166, 227, 161, 0.4); }
  }

  /* error: red ring pulse */
  .error .orb {
    background: radial-gradient(circle at 35% 35%, #f38ba8, #d63031);
    animation: pulse-error 0.8s ease-in-out infinite;
  }

  @keyframes pulse-error {
    0%, 100% { box-shadow: 0 0 20px rgba(243, 139, 168, 0.4); }
    50% { box-shadow: 0 0 40px rgba(243, 139, 168, 0.8), 0 0 0 4px rgba(243, 139, 168, 0.3); }
  }
</style>
