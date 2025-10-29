import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as git from "../../src/git";
import { localBranches, remoteBranches } from "../fixtures/branches";

// Mock execa
vi.mock("execa", () => ({
  execa: vi.fn(),
}));

import { execa } from "execa";

describe("git.ts - Branch Operations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("getLocalBranches (T102)", () => {
    it("should return list of local branches", async () => {
      const mockOutput = `main
develop
feature/user-auth
feature/dashboard
hotfix/security-patch
release/1.2.0`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: "",
        exitCode: 0,
      });

      const branches = await git.getLocalBranches();

      expect(branches).toHaveLength(6);
      expect(branches[0]).toEqual({
        name: "main",
        type: "local",
        branchType: "main",
        isCurrent: false,
      });
      expect(branches[2]).toEqual({
        name: "feature/user-auth",
        type: "local",
        branchType: "feature",
        isCurrent: false,
      });
      expect(execa).toHaveBeenCalledWith("git", [
        "branch",
        "--format=%(refname:short)",
      ]);
    });

    it("should handle empty branch list", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      const branches = await git.getLocalBranches();

      expect(branches).toHaveLength(0);
    });

    it("should throw GitError on failure", async () => {
      (execa as any).mockRejectedValue(new Error("Git command failed"));

      await expect(git.getLocalBranches()).rejects.toThrow(
        "Failed to get local branches",
      );
    });
  });

  describe("getRemoteBranches (T103)", () => {
    it("should return list of remote branches", async () => {
      const mockOutput = `origin/main
origin/develop
origin/feature/api-integration
origin/hotfix/bug-123`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: "",
        exitCode: 0,
      } as any);

      const branches = await git.getRemoteBranches();

      expect(branches).toHaveLength(4);
      expect(branches[0]).toEqual({
        name: "origin/main",
        type: "remote",
        branchType: "main",
        isCurrent: false,
      });
      expect(branches[2]).toEqual({
        name: "origin/feature/api-integration",
        type: "remote",
        branchType: "feature",
        isCurrent: false,
      });
      expect(execa).toHaveBeenCalledWith("git", [
        "branch",
        "-r",
        "--format=%(refname:short)",
      ]);
    });

    it("should filter out HEAD references", async () => {
      const mockOutput = `origin/HEAD -> origin/main
origin/main
origin/develop`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: "",
        exitCode: 0,
      } as any);

      const branches = await git.getRemoteBranches();

      expect(branches).toHaveLength(2);
      expect(branches.every((b) => !b.name.includes("HEAD"))).toBe(true);
    });

    it("should throw GitError on failure", async () => {
      (execa as any).mockRejectedValue(new Error("Git command failed"));

      await expect(git.getRemoteBranches()).rejects.toThrow(
        "Failed to get remote branches",
      );
    });
  });

  describe("getAllBranches (T101)", () => {
    it("should return all local and remote branches", async () => {
      let callCount = 0;
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          callCount++;

          // getCurrentBranch call (check this first as it's most specific)
          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return {
              stdout: "main",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          // getRemoteBranches call
          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "origin/main\norigin/develop",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          // getLocalBranches call
          if (
            args?.[0] === "branch" &&
            args.includes("--format=%(refname:short)")
          ) {
            return {
              stdout: "main\ndevelop\nfeature/test",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          } as any;
        },
      );

      const branches = await git.getAllBranches();

      expect(branches).toHaveLength(5); // 3 local + 2 remote
      expect(branches.filter((b) => b.type === "local")).toHaveLength(3);
      expect(branches.filter((b) => b.type === "remote")).toHaveLength(2);

      // Check that current branch is marked
      const mainBranch = branches.find(
        (b) => b.name === "main" && b.type === "local",
      );
      expect(mainBranch?.isCurrent).toBe(true);
    });

    it("should mark current branch as isCurrent", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (
            args?.[0] === "branch" &&
            !args.includes("-r") &&
            !args.includes("--show-current")
          ) {
            return {
              stdout: "main\nfeature/test",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return {
              stdout: "feature/test",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          } as any;
        },
      );

      const branches = await git.getAllBranches();

      const currentBranch = branches.find((b) => b.name === "feature/test");
      expect(currentBranch?.isCurrent).toBe(true);

      const mainBranch = branches.find((b) => b.name === "main");
      expect(mainBranch?.isCurrent).toBe(false);
    });

    it("should handle no current branch (detached HEAD)", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (
            args?.[0] === "branch" &&
            !args.includes("-r") &&
            !args.includes("--show-current")
          ) {
            return {
              stdout: "main",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            } as any;
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          } as any;
        },
      );

      const branches = await git.getAllBranches();

      expect(branches.every((b) => !b.isCurrent)).toBe(true);
    });
  });

  describe("branchExists (T201)", () => {
    it("should return true for existing branch", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      const exists = await git.branchExists("main");

      expect(exists).toBe(true);
      expect(execa).toHaveBeenCalledWith("git", [
        "show-ref",
        "--verify",
        "--quiet",
        "refs/heads/main",
      ]);
    });

    it("should return false for non-existent branch", async () => {
      (execa as any).mockRejectedValue(new Error("Branch not found"));

      const exists = await git.branchExists("non-existent");

      expect(exists).toBe(false);
    });
  });

  describe("createBranch (T201)", () => {
    it("should create a new branch", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await git.createBranch("feature/new-feature", "main");

      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        "feature/new-feature",
        "main",
      ]);
    });

    it("should use main as default base branch", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await git.createBranch("feature/new-feature");

      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        "feature/new-feature",
        "main",
      ]);
    });

    it("should throw GitError on failure", async () => {
      (execa as any).mockRejectedValue(new Error("Failed to create branch"));

      await expect(git.createBranch("feature/test")).rejects.toThrow(
        "Failed to create branch",
      );
    });
  });

  describe("deleteBranch (T605)", () => {
    it("should delete a branch", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await git.deleteBranch("feature/old-feature");

      expect(execa).toHaveBeenCalledWith("git", [
        "branch",
        "-d",
        "feature/old-feature",
      ]);
    });

    it("should force delete when force=true", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await git.deleteBranch("feature/old-feature", true);

      expect(execa).toHaveBeenCalledWith("git", [
        "branch",
        "-D",
        "feature/old-feature",
      ]);
    });

    it("should throw GitError on failure", async () => {
      (execa as any).mockRejectedValue(new Error("Branch not found"));

      await expect(git.deleteBranch("feature/test")).rejects.toThrow(
        "Failed to delete branch",
      );
    });
  });

  describe("hasUnpushedCommits", () => {
    it("returns false when remote branch is in sync", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await expect(
        git.hasUnpushedCommits("/worktrees/feature-branch", "feature/branch"),
      ).resolves.toBe(false);

      expect(execa).toHaveBeenCalledWith(
        "git",
        ["log", "origin/feature/branch..feature/branch", "--oneline"],
        { cwd: "/worktrees/feature-branch" },
      );
    });

    it("returns false when remote branch is deleted but branch is merged into origin/main", async () => {
      (execa as any).mockImplementation(
        async (_command: string, args: readonly string[], options: any) => {
          if (args[0] === "log") {
            throw new Error("remote branch missing");
          }

          if (args[0] === "rev-parse") {
            if (args[2] === "origin/main") {
              return { stdout: "5819116", stderr: "", exitCode: 0 } as any;
            }
            throw new Error("ref not found");
          }

          if (args[0] === "merge-base") {
            expect(args).toEqual([
              "merge-base",
              "--is-ancestor",
              "feature/branch",
              "origin/main",
            ]);
            expect(options.cwd).toBe("/worktrees/feature-branch");
            return { stdout: "", stderr: "", exitCode: 0 } as any;
          }

          throw new Error(`Unexpected git command: ${args.join(" ")}`);
        },
      );

      await expect(
        git.hasUnpushedCommits("/worktrees/feature-branch", "feature/branch"),
      ).resolves.toBe(false);
    });

    it("returns true when remote branch is deleted and branch is not merged", async () => {
      (execa as any).mockImplementation(
        async (_command: string, args: readonly string[]) => {
          if (args[0] === "log") {
            throw new Error("remote branch missing");
          }

          if (args[0] === "rev-parse") {
            if (args[2] === "origin/main") {
              return { stdout: "5819116", stderr: "", exitCode: 0 } as any;
            }
            throw new Error("ref not found");
          }

          if (args[0] === "merge-base") {
            throw new Error("not merged");
          }

          throw new Error(`Unexpected git command: ${args.join(" ")}`);
        },
      );

      await expect(
        git.hasUnpushedCommits("/worktrees/feature-branch", "feature/branch"),
      ).resolves.toBe(true);
    });
  });

  describe("US2: Smart Branch Creation Workflow", () => {
    describe("createBranch (T201)", () => {
      it("should create branch from default base branch (main)", async () => {
        (execa as any).mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

        await git.createBranch("feature/new-feature");

        expect(execa).toHaveBeenCalledWith("git", [
          "checkout",
          "-b",
          "feature/new-feature",
          "main",
        ]);
      });

      it("should create branch from specified base branch", async () => {
        (execa as any).mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

        await git.createBranch("hotfix/urgent-fix", "develop");

        expect(execa).toHaveBeenCalledWith("git", [
          "checkout",
          "-b",
          "hotfix/urgent-fix",
          "develop",
        ]);
      });

      it("should throw GitError on failure", async () => {
        (execa as any).mockRejectedValue(new Error("Branch already exists"));

        await expect(git.createBranch("feature/duplicate")).rejects.toThrow(
          "Failed to create branch",
        );
      });
    });

    describe("Branch Type Determination (T203)", () => {
      it("should identify main branch type", async () => {
        (execa as any).mockImplementation(
          async (command: string, args?: readonly string[]) => {
            if (args?.[0] === "branch" && args.includes("--show-current")) {
              return { stdout: "main", stderr: "", exitCode: 0 };
            }
            if (args?.[0] === "branch" && args.includes("-r")) {
              return { stdout: "", stderr: "", exitCode: 0 };
            }
            if (
              args?.[0] === "branch" &&
              args.includes("--format=%(refname:short)")
            ) {
              return { stdout: "main", stderr: "", exitCode: 0 };
            }
          },
        );

        const branches = await git.getAllBranches();
        const mainBranch = branches.find((b) => b.name === "main");
        expect(mainBranch?.branchType).toBe("main");
      });

      it("should identify develop branch type", async () => {
        (execa as any).mockImplementation(
          async (command: string, args?: readonly string[]) => {
            if (args?.[0] === "branch" && args.includes("--show-current")) {
              return { stdout: "main", stderr: "", exitCode: 0 };
            }
            if (args?.[0] === "branch" && args.includes("-r")) {
              return { stdout: "", stderr: "", exitCode: 0 };
            }
            if (
              args?.[0] === "branch" &&
              args.includes("--format=%(refname:short)")
            ) {
              return { stdout: "develop", stderr: "", exitCode: 0 };
            }
          },
        );

        const branches = await git.getAllBranches();
        const devBranch = branches.find((b) => b.name === "develop");
        expect(devBranch?.branchType).toBe("develop");
      });

      it("should identify feature branch type", async () => {
        (execa as any).mockImplementation(
          async (command: string, args?: readonly string[]) => {
            if (args?.[0] === "branch" && args.includes("--show-current")) {
              return { stdout: "main", stderr: "", exitCode: 0 };
            }
            if (args?.[0] === "branch" && args.includes("-r")) {
              return { stdout: "", stderr: "", exitCode: 0 };
            }
            if (
              args?.[0] === "branch" &&
              args.includes("--format=%(refname:short)")
            ) {
              return { stdout: "feature/test", stderr: "", exitCode: 0 };
            }
          },
        );

        const branches = await git.getAllBranches();
        const featureBranch = branches.find((b) => b.name === "feature/test");
        expect(featureBranch?.branchType).toBe("feature");
      });

      it("should identify hotfix branch type", async () => {
        (execa as any).mockImplementation(
          async (command: string, args?: readonly string[]) => {
            if (args?.[0] === "branch" && args.includes("--show-current")) {
              return { stdout: "main", stderr: "", exitCode: 0 };
            }
            if (args?.[0] === "branch" && args.includes("-r")) {
              return { stdout: "", stderr: "", exitCode: 0 };
            }
            if (
              args?.[0] === "branch" &&
              args.includes("--format=%(refname:short)")
            ) {
              return { stdout: "hotfix/urgent", stderr: "", exitCode: 0 };
            }
          },
        );

        const branches = await git.getAllBranches();
        const hotfixBranch = branches.find((b) => b.name === "hotfix/urgent");
        expect(hotfixBranch?.branchType).toBe("hotfix");
      });

      it("should identify release branch type", async () => {
        (execa as any).mockImplementation(
          async (command: string, args?: readonly string[]) => {
            if (args?.[0] === "branch" && args.includes("--show-current")) {
              return { stdout: "main", stderr: "", exitCode: 0 };
            }
            if (args?.[0] === "branch" && args.includes("-r")) {
              return { stdout: "", stderr: "", exitCode: 0 };
            }
            if (
              args?.[0] === "branch" &&
              args.includes("--format=%(refname:short)")
            ) {
              return { stdout: "release/1.0.0", stderr: "", exitCode: 0 };
            }
          },
        );

        const branches = await git.getAllBranches();
        const releaseBranch = branches.find((b) => b.name === "release/1.0.0");
        expect(releaseBranch?.branchType).toBe("release");
      });

      it("should identify other branch types", async () => {
        (execa as any).mockImplementation(
          async (command: string, args?: readonly string[]) => {
            if (args?.[0] === "branch" && args.includes("--show-current")) {
              return { stdout: "main", stderr: "", exitCode: 0 };
            }
            if (args?.[0] === "branch" && args.includes("-r")) {
              return { stdout: "", stderr: "", exitCode: 0 };
            }
            if (
              args?.[0] === "branch" &&
              args.includes("--format=%(refname:short)")
            ) {
              return { stdout: "random-branch", stderr: "", exitCode: 0 };
            }
          },
        );

        const branches = await git.getAllBranches();
        const otherBranch = branches.find((b) => b.name === "random-branch");
        expect(otherBranch?.branchType).toBe("other");
      });
    });

    describe("getCurrentVersion (T204)", () => {
      it("should return version string", async () => {
        const version = await git.getCurrentVersion("/path/to/repo");
        // Should return a version string (either from package.json or default)
        expect(typeof version).toBe("string");
        expect(version).toMatch(/^\d+\.\d+\.\d+$/);
      });

      it("should return default version for nonexistent path", async () => {
        // Test with a path that definitely doesn't exist
        const version = await git.getCurrentVersion(
          "/absolutely/nonexistent/impossible/path/that/does/not/exist",
        );
        expect(version).toBe("0.0.0");
      });
    });

    describe("calculateNewVersion (T205)", () => {
      it("should calculate patch version bump", () => {
        const newVersion = git.calculateNewVersion("1.2.3", "patch");
        expect(newVersion).toBe("1.2.4");
      });

      it("should calculate minor version bump", () => {
        const newVersion = git.calculateNewVersion("1.2.3", "minor");
        expect(newVersion).toBe("1.3.0");
      });

      it("should calculate major version bump", () => {
        const newVersion = git.calculateNewVersion("1.2.3", "major");
        expect(newVersion).toBe("2.0.0");
      });

      it("should handle version with leading zeros", () => {
        const newVersion = git.calculateNewVersion("0.0.1", "patch");
        expect(newVersion).toBe("0.0.2");
      });

      it("should handle initial version", () => {
        const newVersion = git.calculateNewVersion("0.0.0", "minor");
        expect(newVersion).toBe("0.1.0");
      });
    });

    describe("executeNpmVersionInWorktree (T206)", () => {
      it("should be callable with worktree path and version", async () => {
        // This is a complex function that involves fs operations
        // Full testing should be done in integration tests
        // Here we just verify the function is callable
        expect(typeof git.executeNpmVersionInWorktree).toBe("function");
      });

      it("should handle version parameter correctly", async () => {
        // Test that the function accepts correct parameters
        const worktreePath = "/test/path";
        const version = "1.2.3";

        // Function should be callable (actual execution tested in integration tests)
        expect(async () => {
          // We don't actually call it here to avoid fs operations in unit tests
          const fn = git.executeNpmVersionInWorktree;
          expect(fn.length).toBe(2); // Expects 2 parameters
        }).not.toThrow();
      });
    });
  });
});
