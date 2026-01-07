/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import type { BranchItem, Statistics } from "../../types.js";
import { BranchListScreen } from "../../screens/solid/BranchListScreen.js";

const buildBranch = (overrides: Partial<BranchItem>): BranchItem => ({
  name: "feature/active-clean",
  type: "local",
  branchType: "feature",
  isCurrent: false,
  icons: [],
  worktreeStatus: "active",
  hasChanges: false,
  label: "feature/active-clean",
  value: "feature/active-clean",
  syncStatus: "up-to-date",
  lastToolUsageLabel: null,
  safeToCleanup: true,
  ...overrides,
});

const buildStats = (branches: BranchItem[]): Statistics => ({
  localCount: branches.filter((branch) => branch.type === "local").length,
  remoteCount: branches.filter((branch) => branch.type === "remote").length,
  worktreeCount: branches.filter((branch) => Boolean(branch.worktreeStatus))
    .length,
  changesCount: branches.filter((branch) => branch.hasChanges).length,
  lastUpdated: new Date(),
});

const renderScreen = async (branches: BranchItem[], selected: string[]) => {
  const stats = buildStats(branches);
  const testSetup = await testRender(
    () => (
      <BranchListScreen
        branches={branches}
        stats={stats}
        onSelect={() => {}}
        selectedBranches={selected}
      />
    ),
    { width: 80, height: 20 },
  );
  await testSetup.renderOnce();

  return {
    captureCharFrame: testSetup.captureCharFrame,
    cleanup: () => testSetup.renderer.destroy(),
  };
};

describe("BranchListScreen icons", () => {
  it("renders emoji icons without cursor prefix", async () => {
    const branches = [
      buildBranch({
        name: "feature/active-clean",
        label: "feature/active-clean",
        value: "feature/active-clean",
        worktreeStatus: "active",
        safeToCleanup: true,
      }),
      buildBranch({
        name: "feature/no-worktree",
        label: "feature/no-worktree",
        value: "feature/no-worktree",
        worktreeStatus: undefined,
        safeToCleanup: false,
        hasUnpushedCommits: true,
        mergedPR: undefined,
      }),
    ];

    const { captureCharFrame, cleanup } = await renderScreen(branches, [
      "feature/active-clean",
    ]);

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("✅🟢⭕️ feature/active-clean");
      expect(frame).toContain("☑️⚪❌ feature/no-worktree");
      expect(frame).not.toContain(">✅");
      expect(frame).not.toContain(">☑️");
    } finally {
      cleanup();
    }
  });
});
