<script lang="ts">
  interface Props {
    name: string;
    status: 'pending' | 'ready' | 'running' | 'completed' | 'failed' | 'cancelled';
    testStatus?: 'not_run' | 'running' | 'passed' | 'failed' | null;
    retryCount?: number;
  }

  let { name, status, testStatus = null, retryCount = 0 }: Props = $props();

  const statusColors: Record<string, string> = {
    pending: '#6c7086',
    ready: '#74c7ec',
    running: '#74c7ec',
    completed: '#a6e3a1',
    failed: '#f38ba8',
    cancelled: '#6c7086',
  };

  let barColor = $derived(statusColors[status] ?? '#6c7086');
  let barWidth = $derived(
    status === 'completed' ? '100%' :
    status === 'running' ? '60%' :
    status === 'ready' ? '10%' :
    status === 'failed' ? '100%' :
    '0%'
  );
</script>

<div class="task-bar" title={`${name} (${status})`} aria-label={`Task: ${name}, status: ${status}`}>
  <div class="task-info">
    <span class="task-name">{name}</span>
    {#if retryCount > 0}
      <span class="retry-badge" aria-label="Retry count">R{retryCount}</span>
    {/if}
    {#if testStatus && testStatus !== 'not_run'}
      <span
        class="test-badge"
        class:test-passed={testStatus === 'passed'}
        class:test-failed={testStatus === 'failed'}
        class:test-running={testStatus === 'running'}
        aria-label="Test status: {testStatus}"
      >
        {testStatus === 'passed' ? '\u2713' : testStatus === 'failed' ? '\u2717' : '\u25cf'}
      </span>
    {/if}
  </div>
  <div class="bar-track">
    <div
      class="bar-fill"
      class:running={status === 'running'}
      style:width={barWidth}
      style:background={barColor}
    ></div>
  </div>
</div>

<style>
  .task-bar {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .task-info {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .task-name {
    font-size: var(--ui-font-xs, 10px);
    color: rgba(205, 214, 244, 0.7);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }

  .retry-badge {
    font-size: 9px;
    color: #fab387;
    font-weight: 600;
    flex-shrink: 0;
  }

  .test-badge {
    font-size: 10px;
    flex-shrink: 0;
  }

  .test-passed { color: #a6e3a1; }
  .test-failed { color: #f38ba8; }
  .test-running { color: #74c7ec; }

  .bar-track {
    height: 3px;
    background: rgba(108, 112, 134, 0.3);
    border-radius: 2px;
    overflow: hidden;
  }

  .bar-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.4s ease;
  }

  .bar-fill.running {
    animation: progress-pulse 1.5s ease-in-out infinite;
  }

  @keyframes progress-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }
</style>
