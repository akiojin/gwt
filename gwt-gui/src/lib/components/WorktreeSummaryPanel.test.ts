import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn(async () => () => {});

vi.mock("@tauri-apps/api/core", () => ({
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

const quickStartHostEntry = {
  branch: branchFixture.name,
  tool_id: "codex",
  tool_label: "Codex",
  session_id: "session-123",
  mode: "normal",
  model: "gpt-5",
  reasoning_level: "high",
  skip_permissions: true,
  tool_version: "0.33.0",
  docker_force_host: true,
  timestamp: 1_700_000_001,
};

const quickStartDockerEntry = {
  branch: branchFixture.name,
  tool_id: "claude",
  tool_label: "Claude",
  session_id: "session-456",
  mode: "normal",
  model: "sonnet",
  reasoning_level: "high",
  skip_permissions: false,
  tool_version: "latest",
  docker_service: "workspace",
  docker_recreate: false,
  docker_build: true,
  docker_keep: false,
  timestamp: 1_700_000_002,
};
describe("WorktreeSummaryPanel", () => {
  beforeEach(() => {
    cleanup();
    listenMock.mockClear();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
  });

  it("renders branch header and tab UI when branch is selected", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.querySelector("h2")?.textContent).toBe(
        "feature/markdown-ui"
      );
    });

    // Summary tab is active by default
    const tabs = rendered.container.querySelectorAll(".summary-tab");
    expect(tabs).toHaveLength(2);
    expect(tabs[0]?.textContent?.trim()).toBe("Summary");
    expect(tabs[0]?.classList.contains("active")).toBe(true);
    expect(tabs[1]?.textContent?.trim()).toBe("PR");
  });

  it("shows placeholder when no branch is selected", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(
        rendered.container.querySelector(".placeholder h2")?.textContent
      ).toBe("Worktree Summary");
    });
  });

  it("renders Quick Start section in summary tab", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_branch_quick_start", {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
      });
    });

    await waitFor(() => {
      expect(
        rendered.container.querySelector(".quick-title")?.textContent
      ).toBe("Quick Start");
    });
  });

  it("switches to PR tab and shows PrStatusSection", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[1] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(prTab.classList.contains("active")).toBe(true);
      expect(
        rendered.container.querySelector(".pr-status-section")
      ).toBeTruthy();
    });
  });

  it("displays HostOS runtime for quick start entry", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartHostEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.getByText("runtime: HostOS")).toBeTruthy();
    });
  });

  it("displays Docker runtime and service for quick start entry", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartDockerEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.getByText("runtime: Docker")).toBeTruthy();
      expect(rendered.getByText("service: workspace")).toBeTruthy();
    });
  });
});
