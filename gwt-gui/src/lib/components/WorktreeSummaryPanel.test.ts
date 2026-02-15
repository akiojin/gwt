import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn(async () => () => {});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

// Some Vite transforms may resolve the module id with an explicit extension.
vi.mock("@tauri-apps/api/core.js", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

async function renderPanel(props: any) {
  const { default: Panel } = await import("./WorktreeSummaryPanel.svelte");
  return render(Panel, { props });
}

const branchFixture = {
  name: "feature/markdown-ui",
  commit: "1234567",
  is_current: false,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
};

const sessionSummaryFixture = {
  status: "ok",
  generating: false,
  toolId: "codex",
  sessionId: "session-1",
  markdown: "## 要約\n- 変更点を整理した\n- テストを追加",
  bulletPoints: ["変更点を整理した", "テストを追加"],
  error: null,
};

function sessionSummaryCalls() {
  return invokeMock.mock.calls.filter((c) => c[0] === "get_branch_session_summary");
}

describe("WorktreeSummaryPanel", () => {
  beforeEach(() => {
    listenMock.mockClear();
    invokeMock.mockReset();
    // Some code paths may reach the real @tauri-apps/api invoke implementation.
    // Provide a minimal bridge so those calls still hit our mock.
    (globalThis as any).__TAURI_INTERNALS__ = { invoke: invokeMock };
    if ((globalThis as any).window) {
      (globalThis as any).window.__TAURI_INTERNALS__ = { invoke: invokeMock };
    }
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    delete (globalThis as any).__TAURI_INTERNALS__;
    if ((globalThis as any).window) {
      delete (globalThis as any).window.__TAURI_INTERNALS__;
    }
  });

  it("renders markdown summary with heading and list", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_branch_session_summary", {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
        cachedOnly: true,
      }, undefined);
    });

    await waitFor(() => {
      expect(rendered.container.querySelector(".session-summary-markdown h2")).toBeTruthy();
      expect(rendered.container.querySelectorAll(".session-summary-markdown li")).toHaveLength(
        2
      );
    });
  });

  it("does not poll when no agent tab exists", async () => {
    vi.useFakeTimers();
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: [],
      activeAgentTabBranch: null,
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(1);
    });

    await vi.advanceTimersByTimeAsync(120_000);
    expect(sessionSummaryCalls()).toHaveLength(1);
  });

  it("polls every 15s when agent tab is focused", async () => {
    vi.useFakeTimers();
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: ["feature/markdown-ui"],
      activeAgentTabBranch: "feature/markdown-ui",
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(1);
    });

    await vi.advanceTimersByTimeAsync(14_999);
    expect(sessionSummaryCalls()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(1);
    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(2);
    });
  });

  it("polls every 60s when agent tab exists but is not focused", async () => {
    vi.useFakeTimers();
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: ["feature/markdown-ui"],
      activeAgentTabBranch: "other-branch",
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(1);
    });

    await vi.advanceTimersByTimeAsync(15_000);
    expect(sessionSummaryCalls()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(45_000);
    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(2);
    });
  });
});
