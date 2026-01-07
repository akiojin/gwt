/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { BranchListScreen } from "../../screens/solid/BranchListScreen.js";
import type { BranchItem, Statistics } from "../../types.js";

const makeStats = (overrides: Partial<Statistics> = {}): Statistics => ({
  localCount: 0,
  remoteCount: 0,
  worktreeCount: 0,
  changesCount: 0,
  lastUpdated: new Date("2025-01-01T00:00:00Z"),
  ...overrides,
});

const createBranch = (overrides: Partial<BranchItem> = {}): BranchItem => {
  const hasWorktree = Boolean(overrides.worktree);
  const base: BranchItem = {
    name: "feature/test",
    type: "local",
    branchType: "feature",
    isCurrent: false,
    icons: [],
    hasChanges: false,
    label: "feature/test",
    value: "feature/test",
    worktreeStatus: hasWorktree ? "active" : undefined,
    syncStatus: "no-upstream",
  };
  return {
    ...base,
    ...overrides,
    worktreeStatus: overrides.worktreeStatus ?? base.worktreeStatus,
  };
};

const renderBranchList = async (props: {
  branches: BranchItem[];
  stats?: Statistics;
  workingDirectory?: string;
}) => {
  const testSetup = await testRender(
    () => (
      <BranchListScreen
        branches={props.branches}
        stats={props.stats ?? makeStats()}
        onSelect={() => {}}
        workingDirectory={props.workingDirectory}
      />
    ),
    { width: 80, height: 24 },
  );
  await testSetup.renderOnce();
  return testSetup;
};

describe("BranchListScreen worktree footer", () => {
  it("shows selected worktree path", async () => {
    const branch = createBranch({
      worktree: { path: "/tmp/worktree", locked: false, prunable: false },
      worktreeStatus: "active",
    });
    const testSetup = await renderBranchList({
      branches: [branch],
      stats: makeStats({ localCount: 1, worktreeCount: 1 }),
    });

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Worktree: /tmp/worktree");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("falls back to working directory for current branch without worktree", async () => {
    const branch = createBranch({
      isCurrent: true,
      worktree: undefined,
      worktreeStatus: undefined,
    });
    const testSetup = await renderBranchList({
      branches: [branch],
      stats: makeStats({ localCount: 1 }),
      workingDirectory: "/repo/root",
    });

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Worktree: /repo/root");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("shows (none) when branch list is empty", async () => {
    const testSetup = await renderBranchList({
      branches: [],
      stats: makeStats(),
    });

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Worktree: (none)");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
