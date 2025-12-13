import { describe, it, expect, beforeEach, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  mockGenerateWorktreePath: vi.fn<[string, string], Promise<string>>(),
  mockCreateWorktree: vi.fn(),
  mockListAdditionalWorktrees: vi.fn(),
}));

vi.mock("../../../../../src/worktree.js", () => ({
  listAdditionalWorktrees: mocks.mockListAdditionalWorktrees,
  createWorktree: mocks.mockCreateWorktree,
  removeWorktree: vi.fn(),
  generateWorktreePath: mocks.mockGenerateWorktreePath,
  isProtectedBranchName: () => false,
}));

vi.mock("../../../../../src/git.js", () => ({
  getRepositoryRoot: vi.fn().mockResolvedValue("/repo"),
  getCurrentBranch: vi.fn().mockResolvedValue("develop"),
}));

import { createNewWorktree } from "../../../../../src/web/server/services/worktrees.js";

describe("createNewWorktree", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.mockGenerateWorktreePath.mockResolvedValue(
      "/repo/.worktrees/feature-test",
    );
    mocks.mockCreateWorktree.mockResolvedValue(undefined);
    mocks.mockListAdditionalWorktrees.mockResolvedValue([
      {
        path: "/repo/.worktrees/feature-test",
        branch: "feature/test",
        head: "abc1234",
      },
    ]);
  });

  it("passes repo root before branch name to path generator", async () => {
    await createNewWorktree("feature/test", false);

    expect(mocks.mockGenerateWorktreePath).toHaveBeenCalledWith(
      "/repo",
      "feature/test",
    );

    expect(mocks.mockCreateWorktree).toHaveBeenCalledWith(
      expect.objectContaining({
        worktreePath: "/repo/.worktrees/feature-test",
      }),
    );
  });
});
