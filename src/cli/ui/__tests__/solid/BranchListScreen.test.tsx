/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { BranchListScreen } from "../../screens/solid/BranchListScreen.js";
import type { BranchListScreenProps } from "../../screens/solid/BranchListScreen.js";
import type { BranchItem, Statistics } from "../../types.js";

const makeStats = (overrides: Partial<Statistics> = {}): Statistics => ({
  localCount: 0,
  remoteCount: 0,
  worktreeCount: 0,
  changesCount: 0,
  lastUpdated: new Date("2025-01-01T00:00:00Z"),
  ...overrides,
});

const statsForBranches = (branches: BranchItem[]): Statistics => ({
  localCount: branches.filter((branch) => branch.type === "local").length,
  remoteCount: branches.filter((branch) => branch.type === "remote").length,
  worktreeCount: branches.filter((branch) => Boolean(branch.worktreeStatus))
    .length,
  changesCount: branches.filter((branch) => branch.hasChanges).length,
  lastUpdated: new Date("2025-01-01T00:00:00Z"),
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
  selectedBranches?: string[];
  activeProfile?: string | null;
  cleanupUI?: BranchListScreenProps["cleanupUI"];
}) => {
  const testSetup = await testRender(
    () => (
      <BranchListScreen
        branches={props.branches}
        stats={props.stats ?? makeStats()}
        onSelect={() => {}}
        workingDirectory={props.workingDirectory}
        {...(props.activeProfile !== undefined
          ? { activeProfile: props.activeProfile }
          : {})}
        {...(props.selectedBranches
          ? { selectedBranches: props.selectedBranches }
          : {})}
        {...(props.cleanupUI ? { cleanupUI: props.cleanupUI } : {})}
      />
    ),
    { width: 80, height: 24 },
  );
  await testSetup.renderOnce();
  return testSetup;
};

describe("BranchListScreen icons", () => {
  it("renders ASCII icons with spacing and no cursor prefix", async () => {
    const branches = [
      createBranch({
        name: "feature/active-clean",
        label: "feature/active-clean",
        value: "feature/active-clean",
        worktreeStatus: "active",
        safeToCleanup: true,
      }),
      createBranch({
        name: "feature/no-worktree",
        label: "feature/no-worktree",
        value: "feature/no-worktree",
        worktreeStatus: undefined,
        safeToCleanup: false,
        hasUnpushedCommits: true,
        mergedPR: undefined,
      }),
    ];

    const testSetup = await renderBranchList({
      branches,
      stats: statsForBranches(branches),
      selectedBranches: ["feature/active-clean"],
    });

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toMatch(/\[\*\] w {2,}feature\/active-clean/);
      expect(frame).toContain("[ ] . ! feature/no-worktree");
      expect(frame).not.toContain(">[*]");
      expect(frame).not.toContain(">[ ]");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("shows spinner during safety checks and blank icon for remote branches", async () => {
    const branches = [
      createBranch({
        name: "feature/loading",
        label: "feature/loading",
        value: "feature/loading",
        worktree: { path: "/tmp/worktree", locked: false, prunable: false },
      }),
      createBranch({
        name: "feature/unmerged",
        label: "feature/unmerged",
        value: "feature/unmerged",
        worktree: { path: "/tmp/worktree2", locked: false, prunable: false },
        isUnmerged: true,
        safeToCleanup: false,
      }),
      createBranch({
        name: "origin/remote-only",
        label: "origin/remote-only",
        value: "origin/remote-only",
        type: "remote",
        hasRemoteCounterpart: false,
        worktree: undefined,
        worktreeStatus: undefined,
      }),
    ];

    const testSetup = await renderBranchList({
      branches,
      stats: statsForBranches(branches),
      cleanupUI: {
        indicators: {},
        footerMessage: null,
        inputLocked: false,
        safetyLoading: true,
      },
    });

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toMatch(/\[ \] w [-\\|/] feature\/loading/);
      expect(frame).toMatch(/\[ \] w \* feature\/unmerged/);
      expect(frame).toMatch(/\[ \] \. {2,}origin\/remote-only/);
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

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

describe("BranchListScreen shortcut hints", () => {
  it("shows shortcuts in labels and omits them from footer", async () => {
    const branch = createBranch({
      name: "feature/shortcuts",
      label: "feature/shortcuts",
      value: "feature/shortcuts",
    });
    const testSetup = await renderBranchList({
      branches: [branch],
      stats: makeStats({ localCount: 1 }),
      activeProfile: "dev",
    });

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Filter(f): (press f to filter)");
      expect(frame).toContain("Mode(tab): All");
      expect(frame).toContain("Profile(p): dev");
      expect(frame).not.toContain("[f] Filter");
      expect(frame).not.toContain("[tab] Mode");
      expect(frame).not.toContain("[p] Profiles");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
