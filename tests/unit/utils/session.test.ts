/**
 * @vitest-environment node
 */
import { describe, it, expect, beforeEach, vi } from "vitest";

vi.mock("node:fs/promises", () => {
  const readdir = vi.fn();
  const readFile = vi.fn();
  const stat = vi.fn();
  return {
    readdir,
    readFile,
    stat,
    default: { readdir, readFile, stat },
  };
});

vi.mock("node:os", () => {
  const homedir = vi.fn(() => "/home/test");
  return {
    homedir,
    default: { homedir },
  };
});

import { readdir, readFile, stat } from "node:fs/promises";
import {
  encodeClaudeProjectPath,
  findLatestClaudeSessionId,
  findLatestCodexSessionId,
  findLatestGeminiSessionId,
  findLatestQwenSessionId,
  waitForClaudeSessionId,
  waitForCodexSessionId,
  isValidUuidSessionId,
  claudeSessionFileExists,
} from "../../../src/utils/session.js";

describe("utils/session", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("isValidUuidSessionId", () => {
    it("returns true for valid UUIDs", () => {
      expect(isValidUuidSessionId("550e8400-e29b-41d4-a716-446655440000")).toBe(true);
      expect(isValidUuidSessionId("AABBCCDD-EEFF-1122-3344-556677889900")).toBe(true);
    });

    it("returns false for invalid UUIDs", () => {
      expect(isValidUuidSessionId("not-a-uuid")).toBe(false);
      expect(isValidUuidSessionId("550e8400-e29b-41d4-a716")).toBe(false);
      expect(isValidUuidSessionId("")).toBe(false);
      expect(isValidUuidSessionId("550e8400e29b41d4a716446655440000")).toBe(false); // no dashes
    });
  });

  describe("claudeSessionFileExists", () => {
    it("returns true when session file exists", async () => {
      const validUuid = "12345678-1234-1234-1234-123456789012";
      (stat as any).mockResolvedValue({ mtimeMs: 123 });

      const exists = await claudeSessionFileExists(validUuid, "/repo");
      expect(exists).toBe(true);
    });

    it("returns false for invalid UUID format", async () => {
      const exists = await claudeSessionFileExists("not-a-uuid", "/repo");
      expect(exists).toBe(false);
      expect(stat).not.toHaveBeenCalled();
    });

    it("returns false when file does not exist", async () => {
      const validUuid = "12345678-1234-1234-1234-123456789012";
      (stat as any).mockRejectedValue(new Error("ENOENT"));

      const exists = await claudeSessionFileExists(validUuid, "/repo");
      expect(exists).toBe(false);
    });
  });

  it("encodes Claude project path by replacing separators and underscores", () => {
    expect(encodeClaudeProjectPath("/Users/name/my_project")).toBe(
      "-Users-name-my-project",
    );
    expect(encodeClaudeProjectPath("C:\\Users\\name\\repo_test")).toBe(
      "C-Users-name-repo-test",
    );
  });

  it("findLatestCodexSessionId returns newest session id from JSON", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir.endsWith("/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (dir.endsWith("/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (dir.endsWith("/.codex/sessions/2025/12")) {
          return Promise.resolve([
            dirent(
              "rollout-2025-12-06T15-12-04-019af438-56b3-7b32-bf8e-a5faeba5c9db.jsonl",
              "file",
            ),
          ]);
        }
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 300 });
    (readFile as any).mockResolvedValue("[]");

    const id = await findLatestCodexSessionId();
    expect(id).toBe("019af438-56b3-7b32-bf8e-a5faeba5c9db");
  });

  it("findLatestCodexSessionId can pick session closest to reference time", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    // Use valid UUIDs in filenames to match the filename-first extraction
    const earlyUuid = "11111111-1111-1111-1111-111111111111";
    const lateUuid = "22222222-2222-2222-2222-222222222222";

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir.endsWith("/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (dir.endsWith("/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (dir.endsWith("/.codex/sessions/2025/12")) {
          return Promise.resolve([
            dirent(`rollout-${earlyUuid}.jsonl`, "file"),
            dirent(`rollout-${lateUuid}.jsonl`, "file"),
          ]);
        }
      }
      return Promise.resolve([]);
    });
    (stat as any).mockImplementation((filePath: string) => {
      if (filePath.includes(earlyUuid)) return Promise.resolve({ mtimeMs: 1_000 });
      return Promise.resolve({ mtimeMs: 5_000 });
    });
    (readFile as any).mockImplementation((filePath: string) => {
      if (filePath.includes(earlyUuid)) {
        return Promise.resolve(`{"sessionId":"${earlyUuid}"}`);
      }
      return Promise.resolve(`{"sessionId":"${lateUuid}"}`);
    });

    const id = await findLatestCodexSessionId({
      preferClosestTo: 1_200,
      windowMs: 2_000,
    });
    expect(id).toBe(earlyUuid);
  });

  it("waitForCodexSessionId polls until a nearby session appears", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const waitUuid = "33333333-3333-3333-3333-333333333333";

    // first poll returns nothing, second poll sees the file
    const calls: string[] = [];
    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      calls.push(dir);
      if (opts?.withFileTypes) {
        if (dir.endsWith("/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (dir.endsWith("/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (dir.endsWith("/.codex/sessions/2025/12")) {
          // Only on second pass we expose the file
          if (calls.filter((c) => c.endsWith("/2025/12")).length >= 2) {
            return Promise.resolve([dirent(`rollout-${waitUuid}.jsonl`, "file")]);
          }
          return Promise.resolve([]);
        }
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 5_000 });
    (readFile as any).mockResolvedValue(`{"sessionId":"${waitUuid}"}`);

    const idPromise = waitForCodexSessionId({
      startedAt: 4_000,
      timeoutMs: 10_000,
      pollIntervalMs: 10,
    });

    const id = await idPromise;
    expect(id).toBe(waitUuid);
  });

  it("waitForClaudeSessionId polls until a session appears", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const claudeUuid = "44444444-4444-4444-4444-444444444444";

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir.endsWith("/projects/-repo/sessions")) {
          // First call: no files, second call: file appears
          const count = (readdir as any).mock.calls.filter((c: any[]) =>
            (c[0] as string).endsWith("/projects/-repo/sessions"),
          ).length;
          if (count >= 1) {
            return Promise.resolve([dirent(`${claudeUuid}.jsonl`, "file")]);
          }
          return Promise.resolve([]);
        }
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 10 });
    (readFile as any).mockResolvedValue(`{"session_id":"${claudeUuid}"}`);

    const id = await waitForClaudeSessionId("/repo", {
      timeoutMs: 5_000,
      pollIntervalMs: 10,
    });
    expect(id).toBe(claudeUuid);
  });

  it("findLatestClaudeSessionId tries dot-to-dash encoding used by Claude", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const dotdashUuid = "55555555-5555-5555-5555-555555555555";

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir.endsWith("/projects/-repo--worktrees-branch/sessions")) {
          return Promise.resolve([dirent(`${dotdashUuid}.jsonl`, "file")]);
        }
        return Promise.resolve([]);
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 20 });
    (readFile as any).mockResolvedValue(`{"sessionId":"${dotdashUuid}"}`);

    const id = await findLatestClaudeSessionId("/repo/.worktrees/branch");
    expect(id).toBe(dotdashUuid);
  });

  it("findLatestClaudeSessionId reads JSONL lines and extracts session_id", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const jsonlUuid = "66666666-6666-6666-6666-666666666666";

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        return Promise.resolve([dirent("log.jsonl", "file")]);
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 123 });
    (readFile as any).mockResolvedValue(
      `{"session_id":"${jsonlUuid}"}\n{"message":"hello"}`,
    );

    const id = await findLatestClaudeSessionId("/repos/sample");
    expect(id).toBe(jsonlUuid);
    expect(readdir).toHaveBeenCalledWith(
      "/home/test/.claude/projects/-repos-sample/sessions",
      { withFileTypes: true },
    );
  });

  it("uses CODEX_HOME and CLAUDE_CONFIG_DIR when provided", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const customCodexUuid = "77777777-7777-7777-7777-777777777777";
    const customClaudeUuid = "88888888-8888-8888-8888-888888888888";

    process.env.CODEX_HOME = "/custom/codex";
    process.env.CLAUDE_CONFIG_DIR = "/custom/claude";

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir === "/custom/codex/sessions") {
          return Promise.resolve([dirent(`rollout-${customCodexUuid}.jsonl`, "file")]);
        }
        if (dir === "/custom/claude/projects/-repo/sessions") {
          return Promise.resolve([dirent(`${customClaudeUuid}.jsonl`, "file")]);
        }
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 123 });
    (readFile as any).mockImplementation((filePath: string) => {
      if (filePath.includes(customCodexUuid)) {
        return Promise.resolve(JSON.stringify({ id: customCodexUuid }));
      }
      return Promise.resolve(`{"session_id":"${customClaudeUuid}"}`);
    });

    const codexId = await findLatestCodexSessionId();
    const claudeId = await findLatestClaudeSessionId("/repo");
    expect(codexId).toBe(customCodexUuid);
    expect(claudeId).toBe(customClaudeUuid);

    delete process.env.CODEX_HOME;
    delete process.env.CLAUDE_CONFIG_DIR;
  });

  it("falls back to ~/.config/claude when ~/.claude is missing", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const cfgClaudeUuid = "99999999-9999-9999-9999-999999999999";

    // First call (.claude) will throw, second (.config/claude) returns file
    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir === "/home/test/.claude/projects/-repo/sessions") {
          return Promise.reject(new Error("missing"));
        }
        if (dir === "/home/test/.config/claude/projects/-repo/sessions") {
          return Promise.resolve([dirent(`${cfgClaudeUuid}.jsonl`, "file")]);
        }
      }
      return Promise.resolve([]);
    });

    (stat as any).mockResolvedValue({ mtimeMs: 10 });
    (readFile as any).mockResolvedValue(`{"session_id":"${cfgClaudeUuid}"}`);

    const id = await findLatestClaudeSessionId("/repo");
    expect(id).toBe(cfgClaudeUuid);
  });

  it("reads Claude sessionId from ~/.claude/history.jsonl when per-project sessions are absent", async () => {
    // No per-project sessions
    (readdir as any).mockRejectedValue(new Error("missing sessions"));

    const histUuid = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const otherUuid = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";

    // history.jsonl contains entries for other projects and the target one
    const historyContent = [
      `{"project":"/other","sessionId":"${otherUuid}"}`,
      `{"project":"/repo/sub","sessionId":"${histUuid}"}`,
    ].join("\n");

    (readFile as any).mockImplementation((filePath: string) => {
      if (filePath.endsWith("history.jsonl")) {
        return Promise.resolve(historyContent);
      }
      return Promise.reject(new Error("no sessions"));
    });

    const id = await findLatestClaudeSessionId("/repo/subdir");
    expect(id).toBe(histUuid);
  });

  it("returns null when session files are missing", async () => {
    (readdir as any).mockRejectedValue(new Error("missing"));
    const codexId = await findLatestCodexSessionId();
    const claudeId = await findLatestClaudeSessionId("/repos/none");
    const geminiId = await findLatestGeminiSessionId("/repos/none");
    const qwenId = await findLatestQwenSessionId("/repos/none");
    expect(codexId).toBeNull();
    expect(claudeId).toBeNull();
    expect(geminiId).toBeNull();
    expect(qwenId).toBeNull();
  });

  it("findLatestGeminiSessionId picks latest chats json", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const gemini1Uuid = "cccccccc-cccc-cccc-cccc-cccccccccccc";
    const gemini2Uuid = "dddddddd-dddd-dddd-dddd-dddddddddddd";

    // collectFilesRecursive walks through directories recursively
    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir.endsWith("/.gemini/tmp")) {
          return Promise.resolve([dirent("projA", "dir"), dirent("projB", "dir")]);
        }
        if (dir.endsWith("/projA")) {
          return Promise.resolve([dirent("a.json", "file")]);
        }
        if (dir.endsWith("/projB")) {
          return Promise.resolve([dirent("b.json", "file")]);
        }
        return Promise.resolve([]);
      }
      return Promise.resolve([]);
    });
    (stat as any).mockImplementation((filePath: string) => {
      return Promise.resolve({
        mtimeMs: filePath.includes("b.json") ? 300 : 200,
      });
    });
    // Include cwd field so that the cwd filter matches the requested path
    (readFile as any).mockImplementation((filePath: string) => {
      if (filePath.includes("b.json")) {
        return Promise.resolve(JSON.stringify({ id: gemini2Uuid, cwd: "/repo" }));
      }
      return Promise.resolve(JSON.stringify({ id: gemini1Uuid, cwd: "/repo" }));
    });

    const id = await findLatestGeminiSessionId("/repo");
    expect(id).toBe(gemini2Uuid);
  });

  it("findLatestQwenSessionId falls back to filename when no id", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });
    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir.endsWith("/.qwen/tmp")) {
          return Promise.resolve([dirent("p1", "dir")]);
        }
        return Promise.resolve([dirent("save-123.json", "file")]);
      }
      if (dir.endsWith("/.qwen/tmp")) {
        return Promise.resolve(["p1"]);
      }
      return Promise.resolve(["save-123.json"]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 123 });
    (readFile as any).mockResolvedValue("{}");

    const id = await findLatestQwenSessionId("/repo");
    expect(id).toBe("save-123");
  });
});
