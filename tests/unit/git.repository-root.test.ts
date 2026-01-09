import { describe, it, expect, beforeEach, mock } from "bun:test";

mock.module("execa", () => ({
  execa: mock(),
}));

// fs mock must handle existsSync/statSync/readFileSync used in getRepositoryRoot
const existsSync = mock();
const statSync = mock();
const readFileSync = mock();
const readdirSync = mock(() => []);
const unlinkSync = mock();
const mkdirSync = mock();

mock.module("node:fs", () => ({
  existsSync,
  statSync,
  readFileSync,
  readdirSync,
  unlinkSync,
  mkdirSync,
  default: {
    existsSync,
    statSync,
    readFileSync,
    readdirSync,
    unlinkSync,
    mkdirSync,
  },
}));

import { execa } from "execa";
import { getRepositoryRoot } from "../../src/git.js";

describe("getRepositoryRoot - worktree resolution", () => {
  beforeEach(() => {
    mock.restore();
    mock.clearAllMocks();
  });

  it("should strip .worktrees segment and return real repo root", async () => {
    // Arrange: git rev-parse --show-toplevel returns worktree path
    (execa as unknown as Mock).mockResolvedValueOnce({
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
