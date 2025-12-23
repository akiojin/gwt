/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useGitData } from "../../hooks/useGitData.js";
import { Window } from "happy-dom";

vi.mock("../../../git.js", () => ({
  getAllBranches: vi.fn(),
  fetchAllRemotes: vi.fn(),
  getRepositoryRoot: vi.fn(),
  collectUpstreamMap: vi.fn(),
  getBranchDivergenceStatuses: vi.fn(),
  hasUnpushedCommitsInRepo: vi.fn(),
  hasUncommittedChanges: vi.fn(),
}));

vi.mock("../../../worktree.js", () => ({
  listAdditionalWorktrees: vi.fn(),
}));

vi.mock("../../../github.js", () => ({
  getPullRequestByBranch: vi.fn(),
}));

vi.mock("../../../config/index.js", () => ({
  getLastToolUsageMap: vi.fn(),
}));

import {
  getAllBranches,
  fetchAllRemotes,
  getRepositoryRoot,
  collectUpstreamMap,
  getBranchDivergenceStatuses,
  hasUnpushedCommitsInRepo,
  hasUncommittedChanges,
} from "../../../git.js";
import { listAdditionalWorktrees } from "../../../worktree.js";
import { getPullRequestByBranch } from "../../../github.js";
import { getLastToolUsageMap } from "../../../config/index.js";

describe("useGitData non-blocking fetch", () => {
  beforeEach(() => {
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;

    (getAllBranches as ReturnType<typeof vi.fn>).mockReset();
    (fetchAllRemotes as ReturnType<typeof vi.fn>).mockReset();
    (getRepositoryRoot as ReturnType<typeof vi.fn>).mockReset();
    (collectUpstreamMap as ReturnType<typeof vi.fn>).mockReset();
    (getBranchDivergenceStatuses as ReturnType<typeof vi.fn>).mockReset();
    (hasUnpushedCommitsInRepo as ReturnType<typeof vi.fn>).mockReset();
    (hasUncommittedChanges as ReturnType<typeof vi.fn>).mockReset();
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockReset();
    (getPullRequestByBranch as ReturnType<typeof vi.fn>).mockReset();
    (getLastToolUsageMap as ReturnType<typeof vi.fn>).mockReset();
  });

  it("does not block loading on fetchAllRemotes", async () => {
    const pending = new Promise<void>(() => {});

    (getRepositoryRoot as ReturnType<typeof vi.fn>).mockResolvedValue("/repo");
    (fetchAllRemotes as ReturnType<typeof vi.fn>).mockReturnValue(pending);
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (getLastToolUsageMap as ReturnType<typeof vi.fn>).mockResolvedValue(
      new Map(),
    );
    (collectUpstreamMap as ReturnType<typeof vi.fn>).mockResolvedValue(
      new Map(),
    );
    (getBranchDivergenceStatuses as ReturnType<typeof vi.fn>).mockResolvedValue(
      [],
    );
    (hasUnpushedCommitsInRepo as ReturnType<typeof vi.fn>).mockResolvedValue(
      false,
    );
    (hasUncommittedChanges as ReturnType<typeof vi.fn>).mockResolvedValue(
      false,
    );
    (getPullRequestByBranch as ReturnType<typeof vi.fn>).mockResolvedValue(
      null,
    );

    const { result } = renderHook(() => useGitData());

    await waitFor(
      () => {
        expect(result.current.loading).toBe(false);
      },
      { timeout: 1000 },
    );

    expect(fetchAllRemotes).toHaveBeenCalled();
  });
});
