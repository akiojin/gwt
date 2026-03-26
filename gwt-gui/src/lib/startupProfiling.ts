export type StartupProfileKind = "open_project" | "restore_session";

export type StartupProfilePhase =
  | "fetch_current_branch"
  | "refresh_canvas_worktrees"
  | "restore_project_agent_tabs";

export interface StartupFrontendMetric {
  kind: "startup";
  name: string;
  durationMs: number;
  timestamp: number;
  startupToken: string;
  success: boolean;
}

type PhaseState = {
  startedAtMs: number;
  startedPerf: number;
  finished: boolean;
  success: boolean;
};

type RunState = {
  token: string;
  kind: StartupProfileKind;
  startedAtMs: number;
  startedPerf: number;
  completed: boolean;
  phases: Partial<Record<StartupProfilePhase, PhaseState>>;
};

type Clock = {
  perfNow: () => number;
  wallNow: () => number;
  nextToken: () => string;
};

const REQUIRED_PHASES: StartupProfilePhase[] = [
  "fetch_current_branch",
  "refresh_canvas_worktrees",
  "restore_project_agent_tabs",
];

function totalMetricName(kind: StartupProfileKind): string {
  return kind === "open_project"
    ? "project_start.open_project.total"
    : "project_start.restore_session.total";
}

function phaseMetricName(phase: StartupProfilePhase): string {
  return `project_start.${phase}`;
}

export function createStartupProfilingTracker(clock?: Partial<Clock>) {
  let tokenSeq = 0;
  const clocks: Clock = {
    perfNow: clock?.perfNow ?? (() => performance.now()),
    wallNow: clock?.wallNow ?? (() => Date.now()),
    nextToken:
      clock?.nextToken ??
      (() => {
        tokenSeq += 1;
        return `startup-${tokenSeq}`;
      }),
  };

  let active: RunState | null = null;

  function isActive(token: string | null | undefined): boolean {
    return !!token && active?.token === token && !active.completed;
  }

  function start(kind: StartupProfileKind): string {
    const token = clocks.nextToken();
    active = {
      token,
      kind,
      startedAtMs: clocks.wallNow(),
      startedPerf: clocks.perfNow(),
      completed: false,
      phases: {},
    };
    return token;
  }

  function discard(token: string | null | undefined): void {
    if (!isActive(token)) return;
    active = null;
  }

  function beginPhase(token: string | null | undefined, phase: StartupProfilePhase): void {
    if (!isActive(token) || !active) return;
    if (active.phases[phase]) return;
    active.phases[phase] = {
      startedAtMs: clocks.wallNow(),
      startedPerf: clocks.perfNow(),
      finished: false,
      success: true,
    };
  }

  function finishPhase(
    token: string | null | undefined,
    phase: StartupProfilePhase,
    success = true,
  ): StartupFrontendMetric[] {
    if (!isActive(token) || !active) return [];

    const phaseState =
      active.phases[phase] ??
      {
        startedAtMs: active.startedAtMs,
        startedPerf: active.startedPerf,
        finished: false,
        success: true,
      };
    if (phaseState.finished) return [];

    phaseState.finished = true;
    phaseState.success = success;
    active.phases[phase] = phaseState;

    const nowPerf = clocks.perfNow();
    const nowWall = clocks.wallNow();
    const metrics: StartupFrontendMetric[] = [
      {
        kind: "startup",
        name: phaseMetricName(phase),
        durationMs: nowPerf - phaseState.startedPerf,
        timestamp: nowWall,
        startupToken: active.token,
        success,
      },
    ];

    const allFinished = REQUIRED_PHASES.every((required) => active?.phases[required]?.finished);
    if (!allFinished) return metrics;

    active.completed = true;
    metrics.push({
      kind: "startup",
      name: totalMetricName(active.kind),
      durationMs: nowPerf - active.startedPerf,
      timestamp: nowWall,
      startupToken: active.token,
      success: REQUIRED_PHASES.every((required) => active?.phases[required]?.success),
    });
    active = null;
    return metrics;
  }

  return {
    start,
    discard,
    beginPhase,
    finishPhase,
    activeToken: () => active?.token ?? null,
  };
}

export const startupProfilingTracker = createStartupProfilingTracker();
