import { describe, expect, it } from "vitest";
import {
  createLaunchState,
  bufferLaunchProgressEventRuntime,
  bufferLaunchFinishedEventRuntime,
  resetLaunchStateRuntime,
  LAUNCH_EVENT_BUFFER_LIMIT,
  LAUNCH_STEP_IDS,
  type LaunchState,
} from "./appLaunchStateRuntime";
import type {
  LaunchProgressPayload,
  LaunchFinishedPayload,
} from "./types";

describe("createLaunchState", () => {
  it("returns default state with all fields initialised", () => {
    const s = createLaunchState();
    expect(s.progressOpen).toBe(false);
    expect(s.jobId).toBe("");
    expect(s.step).toBe("fetch");
    expect(s.detail).toBe("");
    expect(s.status).toBe("running");
    expect(s.error).toBeNull();
    expect(s.pendingRequest).toBeNull();
    expect(s.jobStartPending).toBe(false);
    expect(s.bufferedProgressEvents).toEqual([]);
    expect(s.bufferedFinishedEvents).toEqual([]);
    expect(s.docsEditorAutoClosePaneIds).toEqual([]);
  });

  it("returns independent instances", () => {
    const a = createLaunchState();
    const b = createLaunchState();
    a.jobId = "x";
    expect(b.jobId).toBe("");
  });
});

describe("bufferLaunchProgressEventRuntime", () => {
  it("appends a progress event", () => {
    const s = createLaunchState();
    const payload: LaunchProgressPayload = {
      jobId: "j1",
      step: "fetch",
      detail: "d1",
    };
    bufferLaunchProgressEventRuntime(s, payload);
    expect(s.bufferedProgressEvents).toEqual([payload]);
  });

  it("evicts oldest event when buffer limit is reached", () => {
    const s = createLaunchState();
    for (let i = 0; i < LAUNCH_EVENT_BUFFER_LIMIT; i++) {
      bufferLaunchProgressEventRuntime(s, {
        jobId: "j1",
        step: "fetch",
        detail: `d${i}`,
      });
    }
    expect(s.bufferedProgressEvents).toHaveLength(LAUNCH_EVENT_BUFFER_LIMIT);

    const overflow: LaunchProgressPayload = {
      jobId: "j1",
      step: "validate",
      detail: "overflow",
    };
    bufferLaunchProgressEventRuntime(s, overflow);
    expect(s.bufferedProgressEvents).toHaveLength(LAUNCH_EVENT_BUFFER_LIMIT);
    expect(s.bufferedProgressEvents[0].detail).toBe("d1");
    expect(
      s.bufferedProgressEvents[LAUNCH_EVENT_BUFFER_LIMIT - 1],
    ).toEqual(overflow);
  });
});

describe("bufferLaunchFinishedEventRuntime", () => {
  it("appends a finished event", () => {
    const s = createLaunchState();
    const payload: LaunchFinishedPayload = {
      jobId: "j1",
      status: "ok",
      paneId: "p1",
    };
    bufferLaunchFinishedEventRuntime(s, payload);
    expect(s.bufferedFinishedEvents).toEqual([payload]);
  });

  it("evicts oldest event when buffer limit is reached", () => {
    const s = createLaunchState();
    for (let i = 0; i < LAUNCH_EVENT_BUFFER_LIMIT; i++) {
      bufferLaunchFinishedEventRuntime(s, {
        jobId: `j${i}`,
        status: "ok",
      });
    }
    expect(s.bufferedFinishedEvents).toHaveLength(LAUNCH_EVENT_BUFFER_LIMIT);

    const overflow: LaunchFinishedPayload = {
      jobId: "overflow",
      status: "error",
      error: "fail",
    };
    bufferLaunchFinishedEventRuntime(s, overflow);
    expect(s.bufferedFinishedEvents).toHaveLength(LAUNCH_EVENT_BUFFER_LIMIT);
    expect(s.bufferedFinishedEvents[0].jobId).toBe("j1");
    expect(
      s.bufferedFinishedEvents[LAUNCH_EVENT_BUFFER_LIMIT - 1],
    ).toEqual(overflow);
  });
});

describe("resetLaunchStateRuntime", () => {
  it("resets all fields to defaults", () => {
    const s = createLaunchState();
    s.progressOpen = true;
    s.jobId = "j1";
    s.step = "deps";
    s.detail = "installing";
    s.status = "error";
    s.error = "something broke";
    s.pendingRequest = {
      agentId: "a1",
      branch: "feature/x",
    };
    s.jobStartPending = true;
    s.bufferedProgressEvents = [{ jobId: "j1", step: "fetch" }];
    s.bufferedFinishedEvents = [{ jobId: "j1", status: "ok" }];
    s.docsEditorAutoClosePaneIds = ["p1", "p2"];

    resetLaunchStateRuntime(s);

    const fresh = createLaunchState();
    expect(s).toEqual(fresh);
  });
});

describe("LAUNCH_STEP_IDS", () => {
  it("contains the expected steps in order", () => {
    expect(LAUNCH_STEP_IDS).toEqual([
      "fetch",
      "validate",
      "paths",
      "conflicts",
      "create",
      "skills",
      "deps",
    ]);
  });
});
