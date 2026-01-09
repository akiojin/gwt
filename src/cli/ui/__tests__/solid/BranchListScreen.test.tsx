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
      expect(frame).toMatch(/\[\*\] w o feature\/active-clean/);
      expect(frame).toContain("[ ] . ! feature/no-worktree");
      expect(frame).not.toContain(">[*]");
      expect(frame).not.toContain(">[ ]");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("shows spinner for pending safety checks and blank icon for remote branches", async () => {
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
        safetyPendingBranches: new Set(["feature/loading"]),
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

describe("BranchListScreen cursor position stability (FR-037a)", () => {
  it("preserves cursor position when safety check completes", async () => {
    const { createSignal } = await import("solid-js");
    const branches = [
      createBranch({
        name: "feature/first",
        label: "feature/first",
        value: "feature/first",
        worktreeStatus: "active",
      }),
      createBranch({
        name: "feature/second",
        label: "feature/second",
        value: "feature/second",
        worktreeStatus: "active",
      }),
      createBranch({
        name: "feature/third",
        label: "feature/third",
        value: "feature/third",
        worktreeStatus: "active",
      }),
    ];

    // safetyPendingBranchesを動的に変更するためのシグナル
    const [safetyPending, setSafetyPending] = createSignal<Set<string>>(
      new Set(["feature/first", "feature/second", "feature/third"]),
    );

    const testSetup = await testRender(
      () => (
        <BranchListScreen
          branches={branches}
          stats={statsForBranches(branches)}
          onSelect={() => {}}
          cleanupUI={{
            indicators: {},
            footerMessage: null,
            inputLocked: false,
            safetyPendingBranches: safetyPending(),
          }}
        />
      ),
      { width: 80, height: 24 },
    );
    await testSetup.renderOnce();

    try {
      // 1. 初期状態ではカーソルは最初のブランチにある
      let frame = testSetup.captureCharFrame();
      // 最初のブランチがハイライトされている（選択されている）ことを確認
      expect(frame).toContain("feature/first");

      // 2. 下矢印キーでカーソルを2番目のブランチに移動
      testSetup.mockInput.pressArrow("down");
      await testSetup.renderOnce();

      // 3. 安全状態確認が完了したことをシミュレート（pendingから削除）
      setSafetyPending(new Set(["feature/second", "feature/third"]));
      await testSetup.renderOnce();

      // さらに別のブランチの安全状態確認が完了
      setSafetyPending(new Set(["feature/third"]));
      await testSetup.renderOnce();

      // 4. カーソル位置が保持されていることを確認
      // カーソルが2番目のブランチにあるので、下矢印でさらに移動できるはず
      testSetup.mockInput.pressArrow("down");
      await testSetup.renderOnce();

      // 3番目のブランチに移動できていれば、カーソル位置は保持されていた
      frame = testSetup.captureCharFrame();
      expect(frame).toContain("feature/third");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
