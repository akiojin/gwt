import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as config from "../../../src/config/index";

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

// Mock node:os
vi.mock("node:os", () => {
  const homedir = vi.fn(() => "/home/testuser");
  return {
    homedir,
    default: { homedir },
  };
});

import { readFile, writeFile, mkdir, readdir } from "node:fs/promises";

describe("config/index.ts - Session Management", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("saveSession (T301)", () => {
    it("should save session data to file", async () => {
      (mkdir as any).mockResolvedValue(undefined);
      (writeFile as any).mockResolvedValue(undefined);

      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/test",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      await config.saveSession(sessionData);

      expect(mkdir).toHaveBeenCalled();
      expect(writeFile).toHaveBeenCalled();

      const writeFileCall = (writeFile as any).mock.calls[0];
      expect(writeFileCall[0]).toContain(".config/gwt/sessions");
      expect(writeFileCall[1]).toContain("feature/test");
    });

    it("should handle save errors gracefully", async () => {
      (mkdir as any).mockResolvedValue(undefined);
      (writeFile as any).mockRejectedValue(new Error("Write failed"));

      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/test",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      // Should not throw - errors are handled internally
      await config.saveSession(sessionData);

      // If we got here, no error was thrown
      expect(true).toBe(true);
    });

    it("should create session directory if not exists", async () => {
      (mkdir as any).mockResolvedValue(undefined);
      (writeFile as any).mockResolvedValue(undefined);

      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "main",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      await config.saveSession(sessionData);

      expect(mkdir).toHaveBeenCalledWith(
        expect.stringContaining(".config/gwt/sessions"),
        { recursive: true },
      );
    });
  });

  describe("loadSession (T302)", () => {
    it("should load valid session data", async () => {
      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/test",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      (readFile as any).mockResolvedValue(JSON.stringify(sessionData));

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).toEqual(sessionData);
    });

    it("should return null for expired sessions", async () => {
      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/test",
        timestamp: Date.now() - 25 * 60 * 60 * 1000, // 25 hours ago
        repositoryRoot: "/path/to/repo",
      };

      (readFile as any).mockResolvedValue(JSON.stringify(sessionData));

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).toBeNull();
    });

    it("should return null when session file does not exist", async () => {
      (readFile as any).mockRejectedValue(new Error("ENOENT: file not found"));

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).toBeNull();
    });

    it("should return null for invalid JSON", async () => {
      (readFile as any).mockResolvedValue("invalid json");

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).toBeNull();
    });

    it("should accept sessions within 24 hours", async () => {
      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/test",
        timestamp: Date.now() - 23 * 60 * 60 * 1000, // 23 hours ago
        repositoryRoot: "/path/to/repo",
      };

      (readFile as any).mockResolvedValue(JSON.stringify(sessionData));

      const loaded = await config.loadSession("/path/to/repo");

      expect(loaded).not.toBeNull();
      expect(loaded?.lastBranch).toBe("feature/test");
    });
  });

  describe("getAllSessions (T303)", () => {
    it("should return all valid sessions", async () => {
      (readdir as any).mockResolvedValue([
        "repo1_hash1.json",
        "repo2_hash2.json",
      ]);

      const session1: config.SessionData = {
        lastWorktreePath: "/path/to/worktree1",
        lastBranch: "feature/test1",
        timestamp: Date.now() - 1000,
        repositoryRoot: "/path/to/repo1",
      };

      const session2: config.SessionData = {
        lastWorktreePath: "/path/to/worktree2",
        lastBranch: "feature/test2",
        timestamp: Date.now() - 2000,
        repositoryRoot: "/path/to/repo2",
      };

      (readFile as any).mockImplementation((path: string) => {
        if (path.includes("repo1")) {
          return Promise.resolve(JSON.stringify(session1));
        } else if (path.includes("repo2")) {
          return Promise.resolve(JSON.stringify(session2));
        }
        return Promise.reject(new Error("File not found"));
      });

      const sessions = await config.getAllSessions();

      expect(sessions).toHaveLength(2);
      expect(sessions[0].lastBranch).toBe("feature/test1"); // Most recent first
      expect(sessions[1].lastBranch).toBe("feature/test2");
    });

    it("should filter out expired sessions", async () => {
      (readdir as any).mockResolvedValue([
        "repo1_hash1.json",
        "repo2_hash2.json",
      ]);

      const validSession: config.SessionData = {
        lastWorktreePath: "/path/to/worktree1",
        lastBranch: "feature/valid",
        timestamp: Date.now() - 1000,
        repositoryRoot: "/path/to/repo1",
      };

      const expiredSession: config.SessionData = {
        lastWorktreePath: "/path/to/worktree2",
        lastBranch: "feature/expired",
        timestamp: Date.now() - 25 * 60 * 60 * 1000, // 25 hours ago
        repositoryRoot: "/path/to/repo2",
      };

      (readFile as any).mockImplementation((path: string) => {
        if (path.includes("repo1")) {
          return Promise.resolve(JSON.stringify(validSession));
        } else if (path.includes("repo2")) {
          return Promise.resolve(JSON.stringify(expiredSession));
        }
        return Promise.reject(new Error("File not found"));
      });

      const sessions = await config.getAllSessions();

      expect(sessions).toHaveLength(1);
      expect(sessions[0].lastBranch).toBe("feature/valid");
    });

    it("should return empty array when session directory does not exist", async () => {
      (readdir as any).mockRejectedValue(
        new Error("ENOENT: directory not found"),
      );

      const sessions = await config.getAllSessions();

      expect(sessions).toEqual([]);
    });

    it("should skip non-JSON files", async () => {
      (readdir as any).mockResolvedValue([
        "session.json",
        "readme.txt",
        "data.xml",
      ]);

      const sessionData: config.SessionData = {
        lastWorktreePath: "/path/to/worktree",
        lastBranch: "feature/test",
        timestamp: Date.now(),
        repositoryRoot: "/path/to/repo",
      };

      (readFile as any).mockResolvedValue(JSON.stringify(sessionData));

      const sessions = await config.getAllSessions();

      // Only .json file should be processed
      expect(sessions.length).toBeGreaterThanOrEqual(0);
      expect(readFile).toHaveBeenCalledTimes(1);
    });

    it("should sort sessions by timestamp (newest first)", async () => {
      (readdir as any).mockResolvedValue([
        "repo1.json",
        "repo2.json",
        "repo3.json",
      ]);

      const sessions = [
        {
          lastWorktreePath: "/path/1",
          lastBranch: "branch1",
          timestamp: Date.now() - 3000,
          repositoryRoot: "/repo1",
        },
        {
          lastWorktreePath: "/path/2",
          lastBranch: "branch2",
          timestamp: Date.now() - 1000,
          repositoryRoot: "/repo2",
        },
        {
          lastWorktreePath: "/path/3",
          lastBranch: "branch3",
          timestamp: Date.now() - 2000,
          repositoryRoot: "/repo3",
        },
      ];

      (readFile as any).mockImplementation((path: string) => {
        if (path.includes("repo1"))
          return Promise.resolve(JSON.stringify(sessions[0]));
        if (path.includes("repo2"))
          return Promise.resolve(JSON.stringify(sessions[1]));
        if (path.includes("repo3"))
          return Promise.resolve(JSON.stringify(sessions[2]));
        return Promise.reject(new Error("File not found"));
      });

      const result = await config.getAllSessions();

      expect(result[0].lastBranch).toBe("branch2"); // Newest
      expect(result[1].lastBranch).toBe("branch3");
      expect(result[2].lastBranch).toBe("branch1"); // Oldest
    });
  });
});
