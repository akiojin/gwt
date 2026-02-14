import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { PrStatusResponse } from "./types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const mockResponse: PrStatusResponse = {
  statuses: {
    "feature/foo": {
      number: 42,
      title: "Add foo",
      state: "OPEN",
      url: "https://github.com/owner/repo/pull/42",
      mergeable: "MERGEABLE",
      author: "alice",
      baseBranch: "main",
      headBranch: "feature/foo",
      labels: ["enhancement"],
      assignees: ["alice"],
      milestone: null,
      linkedIssues: [],
      checkSuites: [
        {
          workflowName: "CI",
          runId: 1,
          status: "completed",
          conclusion: "success",
        },
      ],
      reviews: [{ reviewer: "bob", state: "APPROVED" }],
      reviewComments: [],
      changedFilesCount: 3,
      additions: 50,
      deletions: 10,
    },
  },
  ghStatus: { available: true, authenticated: true },
};

/**
 * Flush microtask queue to let async refresh() complete.
 * Uses multiple microtask ticks to ensure promise chains settle.
 */
function flushMicrotasks(): Promise<void> {
  return new Promise((resolve) => queueMicrotask(resolve));
}

async function settle() {
  // Multiple rounds to settle nested promise chains
  for (let i = 0; i < 5; i++) {
    await flushMicrotasks();
  }
}

describe("createPrPolling", () => {
  let createPrPolling: typeof import("./prPolling.svelte").createPrPolling;

  beforeEach(async () => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    const mod = await import("./prPolling.svelte");
    createPrPolling = mod.createPrPolling;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns empty statuses initially", () => {
    const poll = createPrPolling("/project", () => ["feature/foo"]);
    expect(poll.state.statuses).toEqual({});
    expect(poll.state.loading).toBe(false);
    expect(poll.state.error).toBeNull();
    poll.destroy();
  });

  it("start() triggers immediate refresh and sets interval", async () => {
    invokeMock.mockResolvedValue(mockResponse);
    const poll = createPrPolling("/project", () => ["feature/foo"]);

    poll.start();
    await settle();

    expect(invokeMock).toHaveBeenCalledWith("fetch_pr_status", {
      projectPath: "/project",
      branches: ["feature/foo"],
    });
    expect(poll.state.statuses["feature/foo"]).toBeDefined();
    expect(poll.state.statuses["feature/foo"]?.number).toBe(42);
    poll.destroy();
  });

  it("stop() clears the interval", async () => {
    invokeMock.mockResolvedValue(mockResponse);
    const poll = createPrPolling("/project", () => ["feature/foo"]);

    poll.start();
    await settle();
    invokeMock.mockClear();

    poll.stop();
    // Advance by more than the poll interval
    vi.advanceTimersByTime(60_000);
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();
    poll.destroy();
  });

  it("polls at 30-second intervals", async () => {
    invokeMock.mockResolvedValue(mockResponse);
    const poll = createPrPolling("/project", () => ["feature/foo"]);

    poll.start();
    await settle();
    invokeMock.mockClear();

    // Advance 30 seconds - should trigger one interval tick
    vi.advanceTimersByTime(30_000);
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    poll.destroy();
  });

  it("stops polling when document becomes hidden", async () => {
    invokeMock.mockResolvedValue(mockResponse);
    const poll = createPrPolling("/project", () => ["feature/foo"]);

    poll.start();
    await settle();
    invokeMock.mockClear();

    // Simulate document becoming hidden
    Object.defineProperty(document, "hidden", {
      value: true,
      writable: true,
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));

    vi.advanceTimersByTime(60_000);
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();

    // Restore
    Object.defineProperty(document, "hidden", {
      value: false,
      writable: true,
      configurable: true,
    });
    poll.destroy();
  });

  it("resumes polling when document becomes visible again", async () => {
    invokeMock.mockResolvedValue(mockResponse);
    const poll = createPrPolling("/project", () => ["feature/foo"]);

    poll.start();
    await settle();

    // Go hidden
    Object.defineProperty(document, "hidden", {
      value: true,
      writable: true,
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
    invokeMock.mockClear();

    // Come back visible
    Object.defineProperty(document, "hidden", {
      value: false,
      writable: true,
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));

    await settle();
    // Should have triggered an immediate refresh
    expect(invokeMock).toHaveBeenCalled();
    poll.destroy();
  });

  it("handles fetch failure without crashing", async () => {
    invokeMock.mockRejectedValue(new Error("network error"));
    const poll = createPrPolling("/project", () => ["feature/foo"]);

    poll.start();
    await settle();

    expect(poll.state.error).toBe("network error");
    expect(poll.state.loading).toBe(false);
    // No crash - can continue
    poll.destroy();
  });

  it("destroy() fully cleans up", async () => {
    invokeMock.mockResolvedValue(mockResponse);
    const poll = createPrPolling("/project", () => ["feature/foo"]);

    poll.start();
    await settle();
    invokeMock.mockClear();

    poll.destroy();
    // After destroy, advancing timers should not trigger fetches
    vi.advanceTimersByTime(60_000);
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();

    // Visibility change should also not trigger fetches
    Object.defineProperty(document, "hidden", {
      value: false,
      writable: true,
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("skips fetch when branch list is empty", async () => {
    invokeMock.mockResolvedValue(mockResponse);
    const poll = createPrPolling("/project", () => []);

    poll.start();
    await settle();

    expect(invokeMock).not.toHaveBeenCalled();
    expect(poll.state.statuses).toEqual({});
    poll.destroy();
  });
});
