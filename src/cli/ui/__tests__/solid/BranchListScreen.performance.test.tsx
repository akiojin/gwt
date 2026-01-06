/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { BranchListScreen } from "../../screens/solid/BranchListScreen.js";
import type { BranchItem, Statistics } from "../../types.js";

const isCI = Boolean(process.env.CI);
const describeFn = isCI ? describe.skip : describe;

function generateMockBranches(count: number): BranchItem[] {
  const branches: BranchItem[] = [];
  const types = ["feature", "hotfix", "release", "other"] as const;
  const branchTypes = [
    "main",
    "develop",
    "feature",
    "hotfix",
    "release",
    "other",
  ] as const;

  for (let i = 0; i < count; i += 1) {
    const type = types[i % types.length];
    const branchType = branchTypes[i % branchTypes.length];
    const hasWorktree = i % 3 === 0;

    branches.push({
      name: `${type}/test-branch-${i.toString().padStart(4, "0")}`,
      branchType,
      type: i % 10 === 0 ? "remote" : "local",
      isCurrent: i === 0,
      worktree: hasWorktree
        ? {
            path: `/mock/worktree/${type}-${i}`,
            branch: `${type}/test-branch-${i.toString().padStart(4, "0")}`,
            isAccessible: i % 5 !== 0,
          }
        : undefined,
      worktreeStatus: hasWorktree
        ? i % 5 !== 0
          ? "active"
          : "inaccessible"
        : undefined,
      hasChanges: i % 4 === 0,
      icons: [],
      label: `${type}/test-branch-${i.toString().padStart(4, "0")}`,
      value: `${type}/test-branch-${i.toString().padStart(4, "0")}`,
    });
  }

  return branches;
}

const buildStats = (branches: BranchItem[]): Statistics => ({
  localCount: branches.filter((branch) => branch.type === "local").length,
  remoteCount: branches.filter((branch) => branch.type === "remote").length,
  worktreeCount: branches.filter((branch) => branch.worktree).length,
  changesCount: branches.filter((branch) => branch.hasChanges).length,
  lastUpdated: new Date(),
});

const measureRenderLatency = async (
  renderOnce: () => Promise<void>,
  timeoutMs: number,
): Promise<number> => {
  const start = performance.now();
  await renderOnce();
  const elapsed = performance.now() - start;
  return Math.min(elapsed, timeoutMs);
};

describeFn("OpenTUI BranchListScreen Performance", () => {
  it("renders 5000 branches and measures input latency", async () => {
    const branches = generateMockBranches(5000);
    const stats = buildStats(branches);

    const testSetup = await testRender(
      () => (
        <BranchListScreen
          branches={branches}
          stats={stats}
          onSelect={() => {}}
          onQuit={() => {}}
        />
      ),
      {
        width: 120,
        height: 30,
      },
    );

    const { mockInput, renderOnce, renderer } = testSetup;

    const renderStart = performance.now();
    await renderOnce();
    const renderTime = performance.now() - renderStart;

    const steps = 5;
    const timeoutMs = 200;
    let totalLatency = 0;

    for (let i = 0; i < steps; i += 1) {
      mockInput.pressArrow("down");
      totalLatency += await measureRenderLatency(renderOnce, timeoutMs);
    }

    const avgLatency = totalLatency / steps;
    const approxFps = avgLatency > 0 ? 1000 / avgLatency : 0;

    renderer.destroy();

    expect(renderTime).toBeLessThan(10000);

    console.log("\nOpenTUI BranchListScreen Performance:");
    console.log(`  Branches: ${branches.length}`);
    console.log(`  Render time: ${renderTime.toFixed(2)}ms`);
    console.log(
      `  Avg input latency (down x${steps}): ${avgLatency.toFixed(2)}ms`,
    );
    console.log(`  Approx input FPS: ${approxFps.toFixed(1)}`);
  });
});
