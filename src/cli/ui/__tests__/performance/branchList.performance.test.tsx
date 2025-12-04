import { describe, it, expect } from "vitest";
import { render } from "ink-testing-library";
import React from "react";
import { BranchListScreen } from "../../components/screens/BranchListScreen.js";
import type { BranchItem, Statistics } from "../../types.js";

/**
 * Generate mock branch items for performance testing
 */
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

  for (let i = 0; i < count; i++) {
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
            isAccessible: i % 5 !== 0, // Some inaccessible
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

// Unused for now - keeping for potential future use
// const mockStats: Statistics = {
//   total: 0,
//   local: 0,
//   remote: 0,
//   current: 0,
//   feature: 0,
//   hotfix: 0,
//   release: 0,
//   worktree: 0,
// };

describe("BranchListScreen Performance", () => {
  it("should render 100+ branches within acceptable time", () => {
    const branches = generateMockBranches(150);
    const stats: Statistics = {
      total: branches.length,
      local: branches.filter((b) => b.type === "local").length,
      remote: branches.filter((b) => b.type === "remote").length,
      current: 1,
      feature: branches.filter((b) => b.branchType === "feature").length,
      hotfix: branches.filter((b) => b.branchType === "hotfix").length,
      release: branches.filter((b) => b.branchType === "release").length,
      worktree: branches.filter((b) => b.worktree).length,
    };

    const startTime = performance.now();

    const { unmount } = render(
      <BranchListScreen
        branches={branches}
        stats={stats}
        onSelect={() => {}}
        onQuit={() => {}}
      />,
    );

    const renderTime = performance.now() - startTime;

    unmount();

    // Rendering should complete within 500ms (generous threshold)
    expect(renderTime).toBeLessThan(500);

    // Log performance metrics
    console.log(`\n📊 Performance Test Results:`);
    console.log(`   Branches: ${branches.length}`);
    console.log(`   Render time: ${renderTime.toFixed(2)}ms`);
    console.log(
      `   Average per branch: ${(renderTime / branches.length).toFixed(3)}ms`,
    );
  });

  it("should handle re-render efficiently when stats update", () => {
    const branches = generateMockBranches(100);
    const stats: Statistics = {
      total: branches.length,
      local: branches.filter((b) => b.type === "local").length,
      remote: branches.filter((b) => b.type === "remote").length,
      current: 1,
      feature: branches.filter((b) => b.branchType === "feature").length,
      hotfix: branches.filter((b) => b.branchType === "hotfix").length,
      release: branches.filter((b) => b.branchType === "release").length,
      worktree: branches.filter((b) => b.worktree).length,
    };

    const { rerender, unmount } = render(
      <BranchListScreen
        branches={branches}
        stats={stats}
        onSelect={() => {}}
        onQuit={() => {}}
        lastUpdated={new Date()}
      />,
    );

    // Simulate stats update (real-time refresh)
    const startTime = performance.now();

    rerender(
      <BranchListScreen
        branches={branches}
        stats={{ ...stats, total: stats.total + 1 }}
        onSelect={() => {}}
        onQuit={() => {}}
        lastUpdated={new Date()}
      />,
    );

    const rerenderTime = performance.now() - startTime;

    unmount();

    // Re-render should be very fast (< 100ms)
    expect(rerenderTime).toBeLessThan(100);

    console.log(`\n🔄 Re-render Performance:`);
    console.log(`   Re-render time: ${rerenderTime.toFixed(2)}ms`);
  });

  it("should handle large branch list (200+ branches)", () => {
    const branches = generateMockBranches(250);
    const stats: Statistics = {
      total: branches.length,
      local: branches.filter((b) => b.type === "local").length,
      remote: branches.filter((b) => b.type === "remote").length,
      current: 1,
      feature: branches.filter((b) => b.branchType === "feature").length,
      hotfix: branches.filter((b) => b.branchType === "hotfix").length,
      release: branches.filter((b) => b.branchType === "release").length,
      worktree: branches.filter((b) => b.worktree).length,
    };

    const startTime = performance.now();

    const { unmount } = render(
      <BranchListScreen
        branches={branches}
        stats={stats}
        onSelect={() => {}}
        onQuit={() => {}}
      />,
    );

    const renderTime = performance.now() - startTime;

    unmount();

    // Even with 250+ branches, should render within 1 second
    expect(renderTime).toBeLessThan(1000);

    console.log(`\n🚀 Large Branch List Performance:`);
    console.log(`   Branches: ${branches.length}`);
    console.log(`   Render time: ${renderTime.toFixed(2)}ms`);
  });
});
