import { describe, it, expect, beforeEach, vi } from "vitest";

vi.mock("execa", () => ({
  execa: vi.fn(),
}));

// fs mock must handle existsSync/statSync/readFileSync used in getRepositoryRoot
const existsSync = vi.fn();
const statSync = vi.fn();
const readFileSync = vi.fn();

vi.mock("node:fs", () => ({
  existsSync,
  statSync,
  readFileSync,
}));

import { execa } from "execa";
import { getRepositoryRoot } from "../../src/git.js";

describe("getRepositoryRoot - worktree resolution", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("should strip .worktrees segment and return real repo root", async () => {
    // Arrange: git rev-parse --show-toplevel returns worktree path
    (execa as unknown as vi.Mock).mockResolvedValueOnce({
      stdout: "/repo/.worktrees/feature-foo",
    });

    // FS layout: /repo exists and is a directory
    existsSync.mockImplementation((p: string) => p === "/repo");
    statSync.mockImplementation((p: string) => ({
      isDirectory: () => p === "/repo",
    }));

    // Act
    const root = await getRepositoryRoot();

    // Assert
    expect(root).toBe("/repo");
  });
});
