/* eslint-disable @typescript-eslint/no-explicit-any */
import { describe, it, expect, mock, beforeEach } from "bun:test";
import * as config from "../../../src/config/index";

// Mock fs/promises
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

import { readFile, writeFile, mkdir, readdir } from "node:fs/promises";

describe("config/index.ts - session history", () => {
  beforeEach(() => {
    // Clear mock call counts and reset implementations
    (readFile as any).mockReset();
    (writeFile as any).mockReset();
    (mkdir as any).mockReset();
    (readdir as any).mockReset();
  });

  it("appends to history and caps at 100 entries", async () => {
    // Existing history: 100 entries (timestamps 1..100)
    const existingHistory = Array.from({ length: 100 }, (_, i) => ({
      branch: `b${i + 1}`,
      worktreePath: `/wt/${i + 1}`,
      toolId: "codex-cli",
      toolLabel: "Codex",
      mode: "normal",
      model: null,
      timestamp: i + 1,
    }));

    (readFile as any).mockResolvedValue(
      JSON.stringify({
        lastWorktreePath: "/old",
        lastBranch: "old",
        timestamp: 0,
        repositoryRoot: "/repo",
        history: existingHistory,
      }),
    );
    (mkdir as any).mockResolvedValue(undefined);
    (writeFile as any).mockResolvedValue(undefined);

    const newSession: config.SessionData = {
      lastWorktreePath: "/new/wt",
      lastBranch: "feature/new",
      lastUsedTool: "codex-cli",
      toolLabel: "Codex",
      mode: "normal",
      model: null,
      timestamp: 200,
      repositoryRoot: "/repo",
    };

    await config.saveSession(newSession);

    const call = (writeFile as any).mock.calls[0];
    const payload = JSON.parse(call[1]);
    expect(payload.history).toHaveLength(100);
    const timestamps = payload.history.map((h: any) => h.timestamp);
    expect(Math.max(...timestamps)).toBe(200);
    expect(Math.min(...timestamps)).toBe(2); // 1 should be dropped
  });

  it("creates history when none exists", async () => {
    (readFile as any).mockRejectedValue(new Error("not found"));
    (mkdir as any).mockResolvedValue(undefined);
    (writeFile as any).mockResolvedValue(undefined);

    const newSession: config.SessionData = {
      lastWorktreePath: "/wt",
      lastBranch: "feature/one",
      lastUsedTool: "codex-cli",
      toolLabel: "Codex",
      mode: "normal",
      model: null,
      timestamp: 123,
      repositoryRoot: "/repo",
    };

    await config.saveSession(newSession);

    const call = (writeFile as any).mock.calls[0];
    const payload = JSON.parse(call[1]);
    expect(payload.history).toHaveLength(1);
    expect(payload.history[0].branch).toBe("feature/one");
    expect(payload.history[0].timestamp).toBe(123);
  });
});
