import { describe, it, expect, mock, beforeEach } from "bun:test";
import * as config from "../../src/config/index";

mock.module("node:fs/promises", () => {
  const readFile = mock();
  const writeFile = mock();
  const mkdir = mock();
  const readdir = mock();
  return {
    readFile,
    writeFile,
    mkdir,
    readdir,
    default: { readFile, writeFile, mkdir, readdir },
  };
});

mock.module("node:os", () => {
  const homedir = mock(() => "/home/testuser");
  return {
    homedir,
    default: { homedir },
  };
});

import { readFile, readdir } from "node:fs/promises";

describe("Integration: Session Resume Workflow (T305)", () => {
  beforeEach(() => {
    mock.restore();
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
    const [first, second] = sessions;
    if (!first || !second) {
      throw new Error("Expected two sessions");
    }
    expect(first.timestamp).toBeGreaterThan(second.timestamp);
  });
});
