import { describe, it, expect, vi, beforeEach } from "vitest";
import * as config from "../../src/config/index";

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

import { readFile, readdir } from "node:fs/promises";

describe("Integration: Session Resume Workflow (T305)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should list and select from multiple sessions", async () => {
    (readdir as any).mockResolvedValue(["repo1.json", "repo2.json"]);

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
      if (path.includes("repo1"))
        return Promise.resolve(JSON.stringify(session1));
      if (path.includes("repo2"))
        return Promise.resolve(JSON.stringify(session2));
      return Promise.reject(new Error("Not found"));
    });

    const sessions = await config.getAllSessions();

    expect(sessions).toHaveLength(2);
    expect(sessions[0].timestamp).toBeGreaterThan(sessions[1].timestamp);
  });
});
