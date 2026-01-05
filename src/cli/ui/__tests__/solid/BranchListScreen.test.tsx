/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { BranchListScreen } from "../../screens/solid/BranchListScreen.js";
import type { BranchItem, Statistics } from "../../types.js";
import type { BranchListScreenProps } from "../../screens/solid/BranchListScreen.js";
import type { ToolStatus } from "../../hooks/useToolStatus.js";

const createBranch = (
  name: string,
  type: "local" | "remote" = "local",
  overrides: Partial<BranchItem> = {},
): BranchItem => ({
  name,
  type,
  branchType: "feature",
  isCurrent: false,
  icons: [],
  hasChanges: false,
  label: name,
  value: name,
  ...overrides,
});

const buildStats = (branches: BranchItem[]): Statistics => ({
  localCount: branches.filter((branch) => branch.type === "local").length,
  remoteCount: branches.filter((branch) => branch.type === "remote").length,
  worktreeCount: branches.filter((branch) => branch.worktree).length,
  changesCount: branches.filter((branch) => branch.hasChanges).length,
  lastUpdated: new Date(),
});

const renderScreen = async (
  branches: BranchItem[],
  options: {
    width?: number;
    height?: number;
    props?: Partial<BranchListScreenProps>;
  } = {},
) => {
  const selections: BranchItem[] = [];
  const stats = buildStats(branches);

  const testSetup = await testRender(
    () => (
      <BranchListScreen
        branches={branches}
        stats={stats}
        onSelect={(branch) => selections.push(branch)}
        {...options.props}
      />
    ),
    {
      width: options.width ?? 80,
      height: options.height ?? 10,
    },
  );

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    selections,
    cleanup,
  };
};

describe("Solid BranchListScreen", () => {
  it("renders header details", async () => {
    const branches = [createBranch("main")];
    const { captureCharFrame, cleanup } = await renderScreen(branches, {
      props: {
        version: "1.2.3",
        activeProfile: "dev",
        workingDirectory: "/tmp/repo",
      },
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("gwt - Branch Selection v1.2.3 | Profile: dev");
      expect(frame).toContain("Working Directory: /tmp/repo");
    } finally {
      cleanup();
    }
  });

  it("renders stats and tool status", async () => {
    const branches = [createBranch("main"), createBranch("feature/test")];
    const toolStatuses: ToolStatus[] = [
      {
        id: "claude-code",
        name: "Claude",
        status: "installed",
        path: "/usr/bin/claude",
        version: "v1.0.0",
      },
    ];

    const { captureCharFrame, cleanup } = await renderScreen(branches, {
      props: { toolStatuses },
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Mode: All");
      expect(frame).toContain("Local: 2");
      expect(frame).toContain("Remote: 0");
      expect(frame).toContain("Worktrees: 0");
      expect(frame).toContain("Changes: 0");
      expect(frame).toContain("Tools: Claude: v1.0.0");
    } finally {
      cleanup();
    }
  });

  it("shows loading indicator", async () => {
    const branches = [createBranch("main")];
    const { renderOnce, captureCharFrame, cleanup } = await renderScreen(
      branches,
      {
        props: { loading: true, loadingIndicatorDelay: 0 },
      },
    );

    try {
      await renderOnce();
      await renderOnce();
      expect(captureCharFrame()).toContain("Loading Git information...");
    } finally {
      cleanup();
    }
  });

  it("shows error message", async () => {
    const branches: BranchItem[] = [];
    const error = new Error("Failed to load branches");
    const { captureCharFrame, cleanup } = await renderScreen(branches, {
      props: { error },
    });

    try {
      expect(captureCharFrame()).toContain("Error: Failed to load branches");
    } finally {
      cleanup();
    }
  });

  it("shows empty state", async () => {
    const branches: BranchItem[] = [];
    const { captureCharFrame, cleanup } = await renderScreen(branches);

    try {
      expect(captureCharFrame()).toContain("No branches found");
    } finally {
      cleanup();
    }
  });

  it("moves selection and selects branch on enter", async () => {
    const branches = [
      createBranch("main"),
      createBranch("feature/login"),
      createBranch("bugfix/issue-1"),
    ];

    const { mockInput, renderOnce, captureCharFrame, selections, cleanup } =
      await renderScreen(branches, { height: 12 });

    try {
      expect(captureCharFrame()).toContain("> [ ] ⚪ ❌ main");

      mockInput.pressArrow("down");
      await renderOnce();

      expect(captureCharFrame()).toContain("> [ ] ⚪ ❌ feature/login");

      mockInput.pressEnter();
      await renderOnce();

      expect(selections).toHaveLength(1);
      expect(selections[0]?.name).toBe("feature/login");
    } finally {
      cleanup();
    }
  });

  it("filters branches by query", async () => {
    const branches = [
      createBranch("feature/search"),
      createBranch("hotfix/urgent"),
      createBranch("bugfix/issue-2"),
    ];

    const { mockInput, renderOnce, captureCharFrame, cleanup } =
      await renderScreen(branches, { height: 12 });

    try {
      mockInput.pressKey("f");
      await renderOnce();

      await mockInput.typeText("feature");
      await renderOnce();

      const frame = captureCharFrame();
      expect(frame).toContain("feature/search");
      expect(frame).not.toContain("hotfix/urgent");
    } finally {
      cleanup();
    }
  });

  it("shows filtered empty state", async () => {
    const branches = [createBranch("feature/search")];
    const { mockInput, renderOnce, captureCharFrame, cleanup } =
      await renderScreen(branches, { height: 12 });

    try {
      mockInput.pressKey("f");
      await renderOnce();

      await mockInput.typeText("zzz");
      await renderOnce();

      expect(captureCharFrame()).toContain("No branches match your filter");
    } finally {
      cleanup();
    }
  });

  it("scrolls when selection moves beyond visible window", async () => {
    const branches = Array.from({ length: 6 }, (_, index) =>
      createBranch(`branch-${index}`),
    );

    const { mockInput, renderOnce, captureCharFrame, cleanup } =
      await renderScreen(branches, { height: 6 });

    try {
      for (let i = 0; i < 4; i += 1) {
        mockInput.pressArrow("down");
        await renderOnce();
      }

      const frame = captureCharFrame();
      expect(frame).toContain("branch-4");
      expect(frame).not.toContain("branch-0");
    } finally {
      cleanup();
    }
  });
});
