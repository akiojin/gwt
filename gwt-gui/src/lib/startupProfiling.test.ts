import { describe, expect, it } from "vitest";
import { createStartupProfilingTracker } from "./startupProfiling";

describe("startupProfiling", () => {
  it("emits total metric after all required phases finish", () => {
    let perf = 0;
    let wall = 1_000;
    const tracker = createStartupProfilingTracker({
      perfNow: () => perf,
      wallNow: () => wall,
      nextToken: () => "startup-1",
    });

    const token = tracker.start("open_project");

    tracker.beginPhase(token, "fetch_current_branch");
    perf = 10;
    wall = 1_010;
    expect(tracker.finishPhase(token, "fetch_current_branch")).toEqual([
      expect.objectContaining({
        kind: "startup",
        name: "project_start.fetch_current_branch",
        durationMs: 10,
        startupToken: "startup-1",
      }),
    ]);

    tracker.beginPhase(token, "refresh_canvas_worktrees");
    perf = 25;
    wall = 1_025;
    tracker.finishPhase(token, "refresh_canvas_worktrees");

    tracker.beginPhase(token, "restore_project_agent_tabs");
    perf = 40;
    wall = 1_040;
    const metrics = tracker.finishPhase(token, "restore_project_agent_tabs");

    expect(metrics).toHaveLength(2);
    expect(metrics[1]).toEqual(
      expect.objectContaining({
        kind: "startup",
        name: "project_start.open_project.total",
        durationMs: 40,
        startupToken: "startup-1",
      }),
    );
    expect(tracker.activeToken()).toBeNull();
  });

  it("drops stale metrics after a new startup token starts", () => {
    let seq = 0;
    const tracker = createStartupProfilingTracker({
      perfNow: () => 0,
      wallNow: () => 1_000,
      nextToken: () => {
        seq += 1;
        return `startup-${seq}`;
      },
    });

    const first = tracker.start("open_project");
    const second = tracker.start("restore_session");

    tracker.beginPhase(first, "fetch_current_branch");
    expect(tracker.finishPhase(first, "fetch_current_branch")).toEqual([]);

    tracker.beginPhase(second, "fetch_current_branch");
    expect(tracker.finishPhase(second, "fetch_current_branch")).toHaveLength(1);
  });

  it("discards an active token explicitly", () => {
    const tracker = createStartupProfilingTracker({
      perfNow: () => 0,
      wallNow: () => 1_000,
      nextToken: () => "startup-1",
    });

    const token = tracker.start("restore_session");
    tracker.beginPhase(token, "fetch_current_branch");
    tracker.discard(token);

    expect(tracker.finishPhase(token, "fetch_current_branch")).toEqual([]);
    expect(tracker.activeToken()).toBeNull();
  });
});
