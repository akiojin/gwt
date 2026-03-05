<script lang="ts">
  import PixelSprite from './PixelSprite.svelte';
  import { getAgentSprite } from './sprites';

  interface Props {
    agentType: 'claude' | 'codex' | 'gemini';
    status: 'starting' | 'running' | 'completed' | 'error';
    name?: string;
    onclick?: () => void;
  }

  let { agentType, status, name, onclick }: Props = $props();

  let sprite = $derived(getAgentSprite(agentType));
  let isAnimated = $derived(status === 'running' || status === 'starting');
  let label = $derived(name ?? `${agentType} agent`);
</script>

<button
  class="agent-avatar"
  class:starting={status === 'starting'}
  class:running={status === 'running'}
  class:completed={status === 'completed'}
  class:error={status === 'error'}
  onclick={onclick}
  title={`${label} (${status})`}
  aria-label={`${label}: ${status}`}
  type="button"
>
  <PixelSprite {sprite} scale={2} animate={isAnimated} frameIndex={0} />
  {#if status === 'completed'}
    <span class="check-overlay">{'\u2713'}</span>
  {/if}
</button>

<style>
  .agent-avatar {
    position: relative;
    width: 32px;
    height: 32px;
    border: none;
    background: none;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    transition: transform 0.15s ease;
    flex-shrink: 0;
  }

  .agent-avatar:hover {
    transform: scale(1.2);
  }

  .agent-avatar:focus-visible {
    outline: 2px solid #b4befe;
    outline-offset: 2px;
  }

  .check-overlay {
    position: absolute;
    bottom: -2px;
    right: -2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #a6e3a1;
    color: #1e1e2e;
    font-size: 9px;
    font-weight: 800;
    display: flex;
    align-items: center;
    justify-content: center;
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

  /* error: shake + red filter */
  .error {
    animation: shake 0.4s ease infinite;
    filter: saturate(0.5) brightness(0.8) sepia(0.5) hue-rotate(-30deg);
  }

  @keyframes shake {
    0%, 100% { transform: translateX(0); }
    25% { transform: translateX(-2px); }
    75% { transform: translateX(2px); }
  }
</style>
