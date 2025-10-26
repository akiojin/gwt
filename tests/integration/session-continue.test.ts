import { describe, it, expect, vi, beforeEach } from "vitest";
import * as config from "../../src/config/index";

// Mock node:fs/promises
vi.mock("node:fs/promises", () => {
  const readFile = vi.fn();
  const writeFile = vi.fn();
  const mkdir = vi.fn();
  const readdir = vi.fn();
  return {
    readFile,
    writeFile,
    mkdir,
    readdir,
    default: { readFile, writeFile, mkdir, readdir },
  };
});

vi.mock("node:os", () => {
  const homedir = vi.fn(() => "/home/testuser");
  return {
    homedir,
    default: { homedir },
  };
});

import { readFile, writeFile, mkdir } from "node:fs/promises";

describe("Integration: Session Continue Workflow (T304)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("Session Continuation Flow", () => {
    it("should save and load session successfully", async () => {
      (mkdir as any).mockResolvedValue(undefined);
      (writeFile as any).mockResolvedValue(undefined);

      // Step 1: Save session
      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree-feature-test",
        lastBranch: "feature/test",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      await config.saveSession(sessionData);

      expect(writeFile).toHaveBeenCalled();

      // Step 2: Load session (simulate -c option)
      const savedData = (writeFile as any).mock.calls[0][1];
      (readFile as any).mockResolvedValue(savedData);

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).not.toBeNull();
      expect(loaded?.lastBranch).toBe("feature/test");
      expect(loaded?.lastWorktreePath).toBe("/path/to/worktree-feature-test");
    });

    it("should handle multiple save/load cycles", async () => {
      (mkdir as any).mockResolvedValue(undefined);
      (writeFile as any).mockResolvedValue(undefined);

      // Save first session
      const session1: config.SessionData = {
        lastWorktreePath: "/path/to/worktree1",
        lastBranch: "feature/first",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      await config.saveSession(session1);

      // Update session
      const session2: config.SessionData = {
        lastWorktreePath: "/path/to/worktree2",
        lastBranch: "feature/second",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      await config.saveSession(session2);

      // Load should return latest session
      const latestData = (writeFile as any).mock.calls[1][1];
      (readFile as any).mockResolvedValue(latestData);

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded?.lastBranch).toBe("feature/second");
    });
  });

  describe("Session Expiration", () => {
    it("should reject expired sessions", async () => {
      const expiredSession: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/old",
        timestamp: Date.now() - 25 * 60 * 60 * 1000, // 25 hours ago
        repositoryRoot: "/path/to/repo",
      };

      (readFile as any).mockResolvedValue(JSON.stringify(expiredSession));

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).toBeNull();
    });

    it("should accept recent sessions", async () => {
      const recentSession: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/recent",
        timestamp: Date.now() - 1 * 60 * 60 * 1000, // 1 hour ago
        repositoryRoot: "/path/to/repo",
      };

      (readFile as any).mockResolvedValue(JSON.stringify(recentSession));

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).not.toBeNull();
      expect(loaded?.lastBranch).toBe("feature/recent");
    });
  });
});
