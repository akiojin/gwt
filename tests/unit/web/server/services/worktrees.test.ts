import { describe, it, expect, beforeEach,  mock } from "bun:test";

const mocks = (({
  mockGenerateWorktreePath: mock(),
  mockCreateWorktree: mock(),
  mockListAdditionalWorktrees: mock(),
}));

mock.module("../../../../../src/worktree.js", () => ({
  listAdditionalWorktrees: mocks.mockListAdditionalWorktrees,
  createWorktree: mocks.mockCreateWorktree,
  removeWorktree: mock(),
  generateWorktreePath: mocks.mockGenerateWorktreePath,
  isProtectedBranchName: () => false,
}));

mock.module("../../../../../src/git.js", () => ({
  getRepositoryRoot: mock().mockResolvedValue("/repo"),
  getCurrentBranch: mock().mockResolvedValue("develop"),
}));

import { createNewWorktree } from "../../../../../src/web/server/services/worktrees.js";

describe("createNewWorktree", () => {
  beforeEach(() => {
    mock.restore();
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
