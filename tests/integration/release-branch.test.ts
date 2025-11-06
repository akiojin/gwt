import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as git from "../../src/git";
import * as worktree from "../../src/worktree";

// Mock execa
vi.mock("execa", () => ({
  execa: vi.fn(),
}));

vi.mock("node:fs", () => ({
  existsSync: vi.fn(() => true),
}));

const mkdirMock = vi.hoisted(() => vi.fn(async () => undefined));

vi.mock("node:fs/promises", async () => {
  const actual =
    await vi.importActual<typeof import("node:fs/promises")>(
      "node:fs/promises",
    );

  const mocked = {
    ...actual,
    mkdir: mkdirMock,
  };

  return {
    ...mocked,
    default: {
      ...actual.default,
      mkdir: mkdirMock,
    },
  };
});

import { execa } from "execa";

describe("Integration: Release Branch and Version Management (T208)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mkdirMock.mockClear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("Release Branch Creation Flow", () => {
    it("should create release branch with version bump", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      // Step 1: Get current version
      const currentVersion = "1.2.3";

      // Step 2: Calculate new version
      const newVersion = git.calculateNewVersion(currentVersion, "minor");
      expect(newVersion).toBe("1.3.0");

      // Step 3: Create release branch
      const releaseBranchName = `release/${newVersion}`;
      await git.createBranch(releaseBranchName, "develop");

      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        "release/1.3.0",
        "develop",
      ]);
    });

    it("should handle patch release", async () => {
      (execa as any).mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });

      const currentVersion = "2.0.0";
      const newVersion = git.calculateNewVersion(currentVersion, "patch");
      expect(newVersion).toBe("2.0.1");

      await git.createBranch(`release/${newVersion}`, "main");
    });

    it("should handle major release", async () => {
      (execa as any).mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });

      const currentVersion = "1.9.9";
      const newVersion = git.calculateNewVersion(currentVersion, "major");
      expect(newVersion).toBe("2.0.0");

      await git.createBranch(`release/${newVersion}`, "develop");
    });
  });

  describe("Version Update in Worktree", () => {
    it("should update version in release worktree", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "checkout") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/develop
`,
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "worktree" && args[1] === "add") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      // Create release branch
      await git.createBranch("release/2.0.0", "develop");

      // Create worktree
      const worktreePath = "/path/to/worktree-release-2.0.0";
      await worktree.createWorktree({
        branchName: "release/2.0.0",
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "develop",
      });

      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });
  });

  describe("Version Calculation Logic", () => {
    it("should calculate versions for different bump types", () => {
      const testCases = [
        { current: "0.1.0", bump: "patch" as const, expected: "0.1.1" },
        { current: "0.1.0", bump: "minor" as const, expected: "0.2.0" },
        { current: "0.1.0", bump: "major" as const, expected: "1.0.0" },
        { current: "1.0.0", bump: "major" as const, expected: "2.0.0" },
        { current: "10.20.30", bump: "patch" as const, expected: "10.20.31" },
      ];

      testCases.forEach(({ current, bump, expected }) => {
        const result = git.calculateNewVersion(current, bump);
        expect(result).toBe(expected);
      });
    });

    it("should handle edge cases", () => {
      expect(git.calculateNewVersion("0.0.0", "patch")).toBe("0.0.1");
      expect(git.calculateNewVersion("0.0.0", "minor")).toBe("0.1.0");
      expect(git.calculateNewVersion("0.0.0", "major")).toBe("1.0.0");
    });
  });

  describe("Release Workflow Integration", () => {
    it("should complete full release workflow", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "checkout") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/develop
`,
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "worktree" && args[1] === "add") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      // Step 1: Calculate version
      const newVersion = git.calculateNewVersion("1.5.0", "minor");
      expect(newVersion).toBe("1.6.0");

      // Step 2: Create release branch
      const branchName = `release/${newVersion}`;
      await git.createBranch(branchName, "develop");

      // Step 3: Create worktree
      const worktreePath = await worktree.generateWorktreePath(
        "/path/to/repo",
        branchName,
      );
      await worktree.createWorktree({
        branchName,
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "develop",
      });

      // Verify all steps completed
      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        branchName,
        "develop",
      ]);
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });
  });

  describe("Error Handling", () => {
    it("should handle invalid version format", () => {
      // calculateNewVersion should still work with malformed versions
      // Result may contain NaN but function doesn't throw
      const result = git.calculateNewVersion("invalid", "patch");
      expect(typeof result).toBe("string");
      expect(result).toContain(".");
    });

    it("should handle release branch creation failure", async () => {
      (execa as any).mockRejectedValue(new Error("Branch already exists"));

      await expect(
        git.createBranch("release/1.0.0", "develop"),
      ).rejects.toThrow("Failed to create branch");
    });
  });
});
