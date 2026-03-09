<script lang="ts">
  import type { ProjectModeState, ProjectIssue } from '../types';
  import GodBar from './godgame/GodBar.svelte';
  import LeadOrb from './godgame/LeadOrb.svelte';
  import IssuePlot from './godgame/IssuePlot.svelte';
  import DecreeInput from './godgame/DecreeInput.svelte';
  import LeadTimeline from './godgame/LeadTimeline.svelte';
  import { getMockIssues } from './godgame/mockData';

  type LeadOrbStatus = 'idle' | 'thinking' | 'waiting_approval' | 'orchestrating' | 'error';

  let pmState: ProjectModeState = $state({
    messages: [],
    ai_ready: false,
    ai_error: null,
    last_error: null,
    is_waiting: false,
    session_name: 'Project Mode',
    llm_call_count: 0,
    estimated_tokens: 0,
  });

  let sending = $state(false);
  let timelineVisible = $state(false);
  let lastOpenedIssueNumber: number | null = $state(null);
  let expandedIssues = $state(new Set<string>());

  // Prototype: issues derived from state (empty until backend returns workspace data)
  let issues: ProjectIssue[] = $derived(import.meta.env.DEV ? getMockIssues() : ([] as ProjectIssue[]));

  let displaySessionName = $derived(
    pmState.session_name && pmState.session_name !== 'Project Mode'
      ? pmState.session_name
      : 'Project Mode',
  );

  let leadOrbStatus: LeadOrbStatus = $derived.by(() => {
    const s = pmState.lead_status;
    if (!s) return 'idle';
    if (s === 'thinking' || s === 'running') return 'thinking';
    if (s === 'waiting_approval') return 'waiting_approval';
    if (s === 'orchestrating') return 'orchestrating';
    if (s === 'error') return 'error';
    return 'idle';
  });

  function toErrorMessage(err: unknown): string {
    if (!err) return 'Unknown error';
    if (typeof err === 'string') return err;
    if (typeof err === 'object' && 'message' in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === 'string') return msg;
    }
    return String(err);
  }

  async function refreshState() {
    try {
      const { invoke } = await import('$lib/tauriInvoke');
      pmState = await invoke<ProjectModeState>('get_project_mode_state_cmd');
    } catch (err) {
      pmState = { ...pmState, last_error: toErrorMessage(err) };
    }
  }

  async function handleSend(text: string) {
    if (sending || pmState.is_waiting) return;
    sending = true;
    try {
      const { invoke } = await import('$lib/tauriInvoke');
      pmState = await invoke<ProjectModeState>('send_project_mode_message_cmd', {
        input: text,
      });
    } catch (err) {
      pmState = { ...pmState, last_error: toErrorMessage(err) };
    } finally {
      sending = false;
    }
  }

  function toggleIssue(id: string) {
    const next = new Set(expandedIssues);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expandedIssues = next;
  }

  // Spec issue dispatch
  $effect(() => {
    const issueNumber = pmState.active_spec_issue_number ?? null;
    if (!issueNumber || issueNumber === lastOpenedIssueNumber) return;
    lastOpenedIssueNumber = issueNumber;
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('gwt-project-mode-open-spec-issue', {
          detail: {
            issueNumber,
            specId: pmState.active_spec_id ?? null,
            issueUrl: pmState.active_spec_issue_url ?? null,
          },
        }),
      );
    }
  });

  // Initial state load
  $effect(() => {
    void refreshState();
  });

  $: displaySessionName =
    state.session_name && state.session_name !== "Project Mode"
      ? state.session_name
      : "Project Mode";

  $: {
    const issueNumber = state.active_spec_issue_number ?? null;
    if (!issueNumber || issueNumber === lastOpenedIssueNumber) {
      // no-op
    } else {
      lastOpenedIssueNumber = issueNumber;
      if (typeof window !== "undefined") {
        window.dispatchEvent(
          new CustomEvent("gwt-project-mode-open-spec-issue", {
            detail: {
              issueNumber,
              issueUrl: state.active_spec_issue_url ?? null,
            },
          })
        );
      }
    }
  }
</script>

<section class="god-world" data-testid="god-world">
  <GodBar
    sessionName={displaySessionName}
    leadStatus={pmState.lead_status ?? ''}
    llmCallCount={pmState.llm_call_count}
    estimatedTokens={pmState.estimated_tokens}
    sessionId={pmState.project_mode_session_id}
  />

  {#if pmState.last_error}
    <div class="world-alert warn" role="alert">{pmState.last_error}</div>
  {/if}
  {#if !pmState.ai_ready}
    <div class="world-alert warn" role="alert">
      {pmState.ai_error ?? 'AI settings are required.'}
    </div>
  {/if}

  <div class="world-canvas">
    <div class="orb-area">
      <LeadOrb status={leadOrbStatus} onclick={() => (timelineVisible = !timelineVisible)} />
    </div>

    {#if issues.length === 0}
      <div class="world-empty">
        <p class="empty-text">The world is quiet. Issue a decree to begin.</p>
      </div>
    {:else}
      <div class="issue-grid">
        {#each issues as issue (issue.id)}
          <IssuePlot
            {issue}
            expanded={expandedIssues.has(issue.id)}
            onToggle={() => toggleIssue(issue.id)}
          />
        {/each}
      </div>
    {/if}
  </div>

  <DecreeInput
    disabled={sending}
    isWaiting={pmState.is_waiting}
    onSend={handleSend}
  />

  <LeadTimeline
    messages={pmState.messages}
    visible={timelineVisible}
    onClose={() => (timelineVisible = false)}
  />
</section>

<style>
  .god-world {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #2d2b55;
    color: #cdd6f4;
    overflow: hidden;
  }

  .world-alert {
    padding: 6px 16px;
    font-size: var(--ui-font-sm, 11px);
    background: rgba(250, 179, 135, 0.12);
    color: #fab387;
    border-bottom: 1px solid rgba(250, 179, 135, 0.2);
    flex-shrink: 0;
  }

  .world-canvas {
    flex: 1;
    overflow-y: auto;
    padding: 24px 16px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 32px;
  }

  .orb-area {
    flex-shrink: 0;
  }

  .world-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .empty-text {
    color: rgba(180, 190, 254, 0.4);
    font-size: var(--ui-font-md, 12px);
    font-style: italic;
  }

  .issue-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    justify-content: center;
    width: 100%;
    max-width: 1080px;
  }
</style>
