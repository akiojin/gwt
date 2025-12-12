import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock all dependencies
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
import * as git from "../../src/git";
import * as worktree from "../../src/worktree";

const stripAnsi = (value: string) => value.replace(/\u001B\[[0-9;]*m/g, "");

describe("E2E: Complete Branch to Worktree Flow", () => {
  let repoRootSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.clearAllMocks();
    mkdirMock.mockClear();
    repoRootSpy = vi
      .spyOn(git, "getRepositoryRoot")
      .mockResolvedValue("/path/to/repo");
  });

  afterEach(() => {
    vi.restoreAllMocks();
    repoRootSpy?.mockRestore();
  });

  describe("Full User Workflow (T110)", () => {
    it("should complete end-to-end workflow: list → select → create → verify", async () => {
      // Mock complete Git repository state
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // Step 1: Get local branches
          if (
            args?.[0] === "branch" &&
            args.includes("--format=%(refname:short)")
          ) {
            return {
              stdout: "main\ndevelop\nfeature/user-auth\nfeature/dashboard",
              stderr: "",
              exitCode: 0,
            };
          }

          // Get remote branches
          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout:
                "origin/main\norigin/develop\norigin/feature/api-integration",
              stderr: "",
              exitCode: 0,
            };
          }

          // Get current branch
          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return {
              stdout: "main",
              stderr: "",
              exitCode: 0,
            };
          }

          // Step 2: List existing worktrees
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/worktree-feature-user-auth
HEAD def5678
branch refs/heads/feature/user-auth
`,
              stderr: "",
              exitCode: 0,
            };
          }

          // Step 3: Create new worktree
          if (args?.[0] === "worktree" && args[1] === "add") {
            return {
              stdout:
                "Preparing worktree (new branch 'feature/dashboard')\nBranch 'feature/dashboard' set up to track local branch 'main'.",
              stderr: "",
              exitCode: 0,
            };
          }

          // Get repository root
          if (args?.[0] === "rev-parse" && args.includes("--git-common-dir")) {
            return {
              stdout: ".git",
              stderr: "",
              exitCode: 0,
            };
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          };
        },
      );

      // === STEP 1: User views branch list ===
      const allBranches = await git.getAllBranches();
      expect(allBranches.length).toBeGreaterThanOrEqual(7); // 4 local + 3 remote

      // Verify current branch is marked
      const currentBranch = allBranches.find((b) => b.isCurrent);
      expect(currentBranch?.name).toBe("main");

      // === STEP 2: Get existing worktrees ===
      const existingWorktrees = await worktree.listAdditionalWorktrees();
      expect(existingWorktrees.length).toBeGreaterThanOrEqual(0);

      // === STEP 3: Create branch table for display ===
      // NOTE: Skipped - legacy UI table display removed in Ink.js migration
      // The Ink UI handles display differently via React components

      // === STEP 4: User selects a branch ===
      const selectedBranchName = "feature/dashboard";
      const selectedBranch = allBranches.find(
        (b) => b.name === selectedBranchName,
      );
      expect(selectedBranch).toBeDefined();

      // === STEP 5: Check if worktree exists for selected branch ===
      const existingWorktreePath =
        await worktree.worktreeExists(selectedBranchName);
      expect(existingWorktreePath).toBeNull();

      // === STEP 6: Generate worktree path ===
      const repoRoot = "/path/to/repo";
      const worktreePath = await worktree.generateWorktreePath(
        repoRoot,
        selectedBranchName,
      );
      expect(worktreePath).toContain("feature-dashboard");

      // === STEP 7: Create worktree ===
      const config = {
        branchName: selectedBranchName,
        worktreePath,
        repoRoot,
        isNewBranch: false,
        baseBranch: "main",
      };

      await worktree.createWorktree(config);

      // === STEP 8: Verify worktree was created ===
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining([
          "worktree",
          "add",
          worktreePath,
          selectedBranchName,
        ]),
      );

      // === STEP 9: Verify new worktree would appear in list ===
      // (In real scenario, we would fetch the list again)
      // This simulates the user seeing the new worktree in the UI
    });

    it("should handle complete new branch creation workflow", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // Get branches
          if (
            args?.[0] === "branch" &&
            args.includes("--format=%(refname:short)")
          ) {
            return {
              stdout: "main",
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "origin/main",
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return {
              stdout: "main",
              stderr: "",
              exitCode: 0,
            };
          }

          // No existing worktrees
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main
`,
              stderr: "",
              exitCode: 0,
            };
          }

          // Branch doesn't exist
          if (args?.[0] === "show-ref") {
            throw new Error("Branch not found");
          }

          // Create worktree with new branch
          if (
            args?.[0] === "worktree" &&
            args[1] === "add" &&
            args.includes("-b")
          ) {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "rev-parse") {
            return {
              stdout: ".git",
              stderr: "",
              exitCode: 0,
            };
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          };
        },
      );

      // User wants to create new feature branch
      const newBranchName = "feature/new-feature";

      // Verify branch doesn't exist
      const branchExists = await git.branchExists(newBranchName);
      expect(branchExists).toBe(false);

      // Generate path
      const worktreePath = await worktree.generateWorktreePath(
        "/path/to/repo",
        newBranchName,
      );

      // Create worktree with new branch
      await worktree.createWorktree({
        branchName: newBranchName,
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: true,
        baseBranch: "main",
      });

      // Verify creation with -b flag
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["-b", newBranchName]),
      );
    });

    it("should handle workflow with remote branch selection", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "origin/main\norigin/feature/remote-work",
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "show-ref") {
            throw new Error("Local branch not found");
          }

          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main
`,
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "worktree" && args[1] === "add") {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "rev-parse") {
            return {
              stdout: ".git",
              stderr: "",
              exitCode: 0,
            };
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          };
        },
      );

      // Get remote branches
      const remoteBranches = await git.getRemoteBranches();
      const remoteFeature = remoteBranches.find(
        (b) => b.name === "origin/feature/remote-work",
      );
      expect(remoteFeature).toBeDefined();

      // Extract local name
      const localName = "feature/remote-work";

      // Verify local doesn't exist
      const localExists = await git.branchExists(localName);
      expect(localExists).toBe(false);

      // Create worktree from remote
      await worktree.createWorktree({
        branchName: localName,
        worktreePath: "/path/to/worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: true,
        baseBranch: "origin/feature/remote-work",
      });

      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });
  });

  describe("Error Recovery Scenarios", () => {
    it("should handle and recover from worktree creation failure", async () => {
      let callCount = 0;
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "worktree" && args[1] === "add") {
            callCount++;
            if (callCount === 1) {
              throw new Error("Disk full");
            }
            return { stdout: "", stderr: "", exitCode: 0 };
          }
          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      // First attempt fails
      await expect(worktree.createWorktree(config)).rejects.toThrow(
        "Failed to create worktree",
      );

      // Second attempt succeeds
      await expect(worktree.createWorktree(config)).resolves.toBeUndefined();
    });
  });
});
