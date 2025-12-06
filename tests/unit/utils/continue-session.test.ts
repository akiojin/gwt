import { describe, it, expect } from "vitest";
import {
  resolveContinueSessionId,
  findLatestBranchSession,
} from "../../../src/cli/ui/utils/continueSession.js";
import type {
  SessionData,
  ToolSessionEntry,
} from "../../../src/config/index.js";

describe("resolveContinueSessionId", () => {
  const branch = "feature/session";
  const toolId = "codex-cli";
  const repoRoot = "/repo";

  it("prefers latest matching history entry", async () => {
    const history: ToolSessionEntry[] = [
      { branch, toolId, worktreePath: "/wt1", toolLabel: "Codex", timestamp: 1 },
      {
        branch,
        toolId,
        worktreePath: "/wt2",
        toolLabel: "Codex",
        timestamp: 2,
        sessionId: "hist-2",
      },
    ];
    const sessionData = {
      lastBranch: branch,
      lastUsedTool: toolId,
      lastSessionId: "last-1",
    } as SessionData;

    const result = await resolveContinueSessionId({
      history,
      sessionData,
      branch,
      toolId,
      repoRoot,
    });

    expect(result).toBe("hist-2");
  });

  it("falls back to lastSessionId when history lacks sessionId", async () => {
    const history: ToolSessionEntry[] = [
      { branch, toolId, worktreePath: "/wt1", toolLabel: "Codex", timestamp: 1 },
    ];
    const sessionData = {
      lastBranch: branch,
      lastUsedTool: toolId,
      lastSessionId: "last-1",
    } as SessionData;

    const result = await resolveContinueSessionId({
      history,
      sessionData,
      branch,
      toolId,
      repoRoot,
      lookupLatestSessionId: vi.fn(),
    });

    expect(result).toBe("last-1");
  });

  it("returns null when no history or matching lastSessionId", async () => {
    const sessionData = {
      lastBranch: branch,
      lastUsedTool: toolId,
      lastSessionId: null,
    } as SessionData;

    const result = await resolveContinueSessionId({
      history: [],
      sessionData,
      branch,
      toolId,
      repoRoot,
    });

    expect(result).toBeNull();
  });

  it("returns null when branch/tool do not match", async () => {
    const sessionData = {
      lastBranch: "other",
      lastUsedTool: toolId,
      lastSessionId: "other-id",
    } as SessionData;

    const result = await resolveContinueSessionId({
      history: [],
      sessionData,
      branch,
      toolId,
      repoRoot,
    });

    expect(result).toBeNull();
  });

  it("findLatestBranchSession returns most recent entry for branch", () => {
    const history: ToolSessionEntry[] = [
      { branch: "feature/other", toolId, toolLabel: "Codex", worktreePath: "/a", timestamp: 1 },
      {
        branch,
        toolId,
        toolLabel: "Codex",
        worktreePath: "/b",
        timestamp: 2,
        sessionId: "s-2",
      },
      {
        branch,
        toolId,
        toolLabel: "Codex",
        worktreePath: "/c",
        timestamp: 3,
        sessionId: "s-3",
      },
    ];

    const entry = findLatestBranchSession(history, branch);
    expect(entry?.sessionId).toBe("s-3");
  });
});
