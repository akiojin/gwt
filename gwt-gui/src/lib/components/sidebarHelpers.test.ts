import { describe, expect, it, vi } from "vitest";
import type { BranchInfo, PrStatusLite, WorktreeInfo } from "../types";
import {
  applyPrStatusUpdate,
  branchPriority,
  branchSortTimestamp,
  buildFilterCacheKey,
  buildWorktreeMap,
  compareBranches,
  decideRefreshFailureAction,
  divergenceClass,
  divergenceIndicator,
  getSafetyLevel,
  getSafetyTitle,
  isBranchProtected,
  normalizeBranchForPrLookup,
  normalizeTabBranch,
  resolveEventListen,
  safetyTitleForLevel,
  sortBranches,
  stripRemotePrefix,
  toErrorMessage,
  toolUsageClass,
} from "./sidebarHelpers";

function branch(overrides: Partial<BranchInfo> = {}): BranchInfo {
  return {
    name: "feature/a",
    commit: "abc1234",
    is_current: false,
    is_agent_running: false,
    agent_status: "unknown",
    ahead: 0,
    behind: 0,
    divergence_status: "UpToDate",
    ...overrides,
  };
}

function worktree(overrides: Partial<WorktreeInfo> = {}): WorktreeInfo {
  return {
    path: "/tmp/repo",
    branch: "feature/a",
    commit: "abc1234",
    status: "active",
    is_main: false,
    has_changes: false,
    has_unpushed: false,
    is_current: false,
    is_protected: false,
    is_agent_running: false,
    agent_status: "unknown",
    ahead: 0,
    behind: 0,
    is_gone: false,
    last_tool_usage: null,
    safety_level: "safe",
    ...overrides,
  };
}

function prStatus(overrides: Partial<PrStatusLite> = {}): PrStatusLite {
  return {
    number: 1,
    state: "OPEN",
    url: "https://example.invalid/p/1",
    mergeable: "MERGEABLE",
    baseBranch: "main",
    headBranch: "feature/a",
    checkSuites: [],
    ...overrides,
  };
}

describe("sidebarHelpers", () => {
  it("normalizes tab branch and remote prefix", () => {
    expect(normalizeTabBranch(" origin/feature/x ")).toBe("feature/x");
    expect(normalizeTabBranch("feature/x")).toBe("feature/x");

    expect(stripRemotePrefix(" origin/feature/x ")).toBe("feature/x");
    expect(stripRemotePrefix("upstream/feature/x")).toBe("feature/x");
    expect(stripRemotePrefix("/leading")).toBe("/leading");
    expect(stripRemotePrefix("noslash")).toBe("noslash");
  });

  it("computes branch priority and timestamp", () => {
    expect(branchPriority("main")).toBe(2);
    expect(branchPriority("origin/main")).toBe(2);
    expect(branchPriority("DEVELOP")).toBe(1);
    expect(branchPriority("feature/x")).toBe(0);

    expect(branchSortTimestamp(branch({ commit_timestamp: 123 }))).toBe(123);
    expect(branchSortTimestamp(branch({ commit_timestamp: "456" as unknown as number }))).toBe(456);
    expect(branchSortTimestamp(branch({ commit_timestamp: "abc" as unknown as number }))).toBeNull();
    expect(branchSortTimestamp(branch({ commit_timestamp: null }))).toBeNull();
  });

  it("compares and sorts branches for all sort modes", () => {
    const main = branch({ name: "main", commit_timestamp: 100 });
    const develop = branch({ name: "develop", commit_timestamp: 90 });
    const alpha = branch({ name: "feature/alpha", commit_timestamp: 200 });
    const beta = branch({ name: "feature/beta", commit_timestamp: 150 });
    const noTs = branch({ name: "feature/no-ts" });
    const remoteSet = new Set<string>(["origin/main", "origin/feature/alpha"]);

    // All filter keeps local side before remote side.
    expect(
      compareBranches(branch({ name: "origin/main" }), branch({ name: "main" }), "All", remoteSet, "name"),
    ).toBeGreaterThan(0);

    // Priority: main > develop > others.
    expect(compareBranches(main, develop, "Local", remoteSet, "updated")).toBeLessThan(0);
    expect(compareBranches(develop, alpha, "Local", remoteSet, "updated")).toBeLessThan(0);

    // Name mode.
    expect(compareBranches(alpha, beta, "Local", remoteSet, "name")).toBeLessThan(0);

    // Updated mode with missing timestamps.
    expect(compareBranches(noTs, beta, "Local", remoteSet, "updated")).toBeGreaterThan(0);
    expect(compareBranches(alpha, noTs, "Local", remoteSet, "updated")).toBeLessThan(0);
    expect(
      compareBranches(
        branch({ name: "feature/zeta" }),
        branch({ name: "feature/eta" }),
        "Local",
        remoteSet,
        "updated",
      ),
    ).toBeGreaterThan(0);

    const sorted = sortBranches([beta, main, alpha, develop], "Local", remoteSet, "updated");
    expect(sorted.map((item) => item.name)).toEqual(["main", "develop", "feature/alpha", "feature/beta"]);
  });

  it("normalizes PR lookup branch by remote membership", () => {
    const remote = new Set<string>(["origin/feature/x", "upstream/feature/y"]);
    expect(normalizeBranchForPrLookup("origin/feature/x", remote)).toBe("feature/x");
    expect(normalizeBranchForPrLookup("upstream/feature/y", remote)).toBe("feature/y");
    expect(normalizeBranchForPrLookup("feature/local", remote)).toBe("feature/local");
  });

  it("builds cache key and worktree map", () => {
    expect(buildFilterCacheKey("Remote", "/repo", 3, 9)).toBe("/repo::3");
    expect(buildFilterCacheKey("Local", "/repo", 3, 9)).toBe("/repo::3::9");
    expect(buildFilterCacheKey("All", "/repo", 3, 9)).toBe("/repo::3::9");

    const map = buildWorktreeMap([
      worktree({ branch: "feature/a" }),
      worktree({ branch: "feature/b" }),
      worktree({ branch: null }),
      worktree({ branch: "feature/a", path: "/tmp/repo-2" }),
    ]);
    expect(Array.from(map.keys())).toEqual(["feature/a", "feature/b"]);
    expect(map.get("feature/a")?.path).toBe("/tmp/repo-2");
  });

  it("formats errors safely", () => {
    expect(toErrorMessage("plain")).toBe("plain");
    expect(toErrorMessage({ message: "typed" })).toBe("typed");
    expect(toErrorMessage({ message: 42 })).toBe("42");
    expect(toErrorMessage(null)).toBe("null");
  });

  it("derives safety level/title and protected state", () => {
    const map = new Map<string, WorktreeInfo>([
      ["feature/a", worktree({ safety_level: "warning", is_protected: true })],
      ["feature/b", worktree({ branch: "feature/b", safety_level: "danger", is_current: true })],
      ["feature/c", worktree({ branch: "feature/c", safety_level: "disabled" })],
      ["feature/d", worktree({ branch: "feature/d", safety_level: "safe" })],
    ]);

    expect(getSafetyLevel(branch({ name: "feature/a" }), map)).toBe("warning");
    expect(getSafetyLevel(branch({ name: "missing" }), map)).toBe("");

    expect(safetyTitleForLevel("safe")).toBe("Safe to delete");
    expect(safetyTitleForLevel("warning")).toBe("Has uncommitted changes or unpushed commits");
    expect(safetyTitleForLevel("danger")).toBe("Has uncommitted changes and unpushed commits");
    expect(safetyTitleForLevel("disabled")).toBe("Protected or current branch");
    expect(safetyTitleForLevel("unknown")).toBe("");

    expect(getSafetyTitle(branch({ name: "feature/a" }), map)).toBe(
      "Has uncommitted changes or unpushed commits",
    );
    expect(getSafetyTitle(branch({ name: "missing" }), map)).toBe("");

    expect(isBranchProtected(branch({ name: "feature/a" }), map)).toBe(true);
    expect(isBranchProtected(branch({ name: "feature/b" }), map)).toBe(true);
    expect(isBranchProtected(branch({ name: "feature/d" }), map)).toBe(false);
    expect(isBranchProtected(branch({ name: "missing" }), map)).toBe(false);
  });

  it("derives divergence and tool usage classes", () => {
    expect(divergenceIndicator(branch({ divergence_status: "Ahead", ahead: 3 }))).toBe("+3");
    expect(divergenceIndicator(branch({ divergence_status: "Behind", behind: 2 }))).toBe("-2");
    expect(
      divergenceIndicator(branch({ divergence_status: "Diverged", ahead: 2, behind: 5 })),
    ).toBe("+2 -5");
    expect(divergenceIndicator(branch({ divergence_status: "UpToDate" }))).toBe("");

    expect(divergenceClass("Ahead")).toBe("ahead");
    expect(divergenceClass("Behind")).toBe("behind");
    expect(divergenceClass("Diverged")).toBe("diverged");
    expect(divergenceClass("Unknown")).toBe("");

    expect(toolUsageClass("claude@1")).toBe("claude");
    expect(toolUsageClass("codex@1")).toBe("codex");
    expect(toolUsageClass("gemini@1")).toBe("gemini");
    expect(toolUsageClass("opencode@1")).toBe("opencode");
    expect(toolUsageClass("open-code@1")).toBe("opencode");
    expect(toolUsageClass("other@1")).toBe("");
    expect(toolUsageClass(null)).toBe("");
  });

  it("decides refresh failure action", () => {
    expect(decideRefreshFailureAction(true, true, true, true)).toBe("clear-loading");
    expect(decideRefreshFailureAction(true, true, false, true)).toBe("ignore");
    expect(decideRefreshFailureAction(false, true, true, true)).toBe("show-error");
    expect(decideRefreshFailureAction(false, false, true, false)).toBe("ignore");
  });

  it("applies incremental PR status updates by head branch", () => {
    const next = applyPrStatusUpdate(
      {
        a: prStatus({ headBranch: "feature/a" }),
        b: null,
        c: prStatus({ headBranch: "feature/c" }),
      },
      "feature/c",
      prStatus({ number: 99, headBranch: "feature/c" }),
    );

    expect(next).not.toBeNull();
    expect(next?.a?.number).toBe(1);
    expect(next?.c?.number).toBe(99);
    expect(applyPrStatusUpdate({ only: prStatus({ headBranch: "feature/a" }) }, "feature/x", prStatus()))
      .toBeNull();
  });

  it("resolves tauri event listen function", async () => {
    const listen = vi.fn(async () => () => {});
    const fromTop = resolveEventListen({ listen });
    await fromTop("test", () => {});
    expect(listen).toHaveBeenCalledTimes(1);

    const listenDefault = vi.fn(async () => () => {});
    const fromDefault = resolveEventListen({ default: { listen: listenDefault } });
    await fromDefault("test", () => {});
    expect(listenDefault).toHaveBeenCalledTimes(1);

    expect(() => resolveEventListen({})).toThrow("Tauri event listen API is unavailable");
  });
});
