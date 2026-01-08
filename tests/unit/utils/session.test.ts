import { describe, it, expect, beforeEach, mock } from "bun:test";
import path from "node:path";

mock.module("node:fs/promises", () => {
  const readdir = mock();
  const readFile = mock();
  const stat = mock();
  return {
    readdir,
    readFile,
    stat,
    default: { readdir, readFile, stat },
  };
});

mock.module("node:os", () => {
  const homedir = mock(() => "/home/test");
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
  waitForClaudeSessionId,
  waitForCodexSessionId,
  isValidUuidSessionId,
  claudeSessionFileExists,
} from "../../../src/utils/session.js";

type ReaddirOptions = { withFileTypes?: boolean };
type MockFn = Mock;

const readdirMock = readdir as unknown as MockFn;
const readFileMock = readFile as unknown as MockFn;
const statMock = stat as unknown as MockFn;
const normalizePath = (value: string) => value.replace(/\\/g, "/");
const endsWithPath = (value: string, suffix: string) =>
  normalizePath(value).endsWith(suffix);
const equalsPath = (value: string, expected: string) =>
  normalizePath(value) === expected;

describe("utils/session", () => {
  beforeEach(() => {
    // Clear mock call counts and reset implementations
    readdirMock.mockReset();
    readFileMock.mockReset();
    statMock.mockReset();
  });

  describe("isValidUuidSessionId", () => {
    it("returns true for valid UUIDs", () => {
      expect(isValidUuidSessionId("550e8400-e29b-41d4-a716-446655440000")).toBe(
        true,
      );
      expect(isValidUuidSessionId("AABBCCDD-EEFF-1122-3344-556677889900")).toBe(
        true,
      );
    });

    it("returns false for invalid UUIDs", () => {
      expect(isValidUuidSessionId("not-a-uuid")).toBe(false);
      expect(isValidUuidSessionId("550e8400-e29b-41d4-a716")).toBe(false);
      expect(isValidUuidSessionId("")).toBe(false);
      expect(isValidUuidSessionId("550e8400e29b41d4a716446655440000")).toBe(
        false,
      ); // no dashes
    });
  });

  describe("claudeSessionFileExists", () => {
    it("returns true when session file exists", async () => {
      const validUuid = "12345678-1234-1234-1234-123456789012";
      statMock.mockResolvedValue({ mtimeMs: 123 });

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
      statMock.mockRejectedValue(new Error("ENOENT"));

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

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025/12")) {
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
    statMock.mockResolvedValue({ mtimeMs: 300 });
    readFileMock.mockResolvedValue("[]");

    const id = await findLatestCodexSessionId();
    expect(id).toBe("019af438-56b3-7b32-bf8e-a5faeba5c9db");
  });

  it("findLatestCodexSessionId extracts cwd from nested payload object", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const sessionUuid = "019af9b0-1d45-7840-a20a-c579b2710459";

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025/12")) {
          return Promise.resolve([
            dirent(`rollout-2025-12-07T16-40-59-${sessionUuid}.jsonl`, "file"),
          ]);
        }
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 300 });
    // Codex session format: cwd is inside payload object
    readFileMock.mockResolvedValue(
      JSON.stringify({
        timestamp: "2025-12-07T16:40:59.000Z",
        type: "session_meta",
        payload: { id: sessionUuid, cwd: "/gwt" },
      }),
    );

    // cwd option matches worktree path, session cwd is repo root
    const id = await findLatestCodexSessionId({
      cwd: "/gwt/.worktrees/feature",
    });
    expect(id).toBe(sessionUuid);
  });

  it("findLatestCodexSessionId matches when worktree path starts with session cwd", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const sessionUuid = "abcdabcd-abcd-abcd-abcd-abcdabcdabcd";

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025/12")) {
          return Promise.resolve([
            dirent(`rollout-${sessionUuid}.jsonl`, "file"),
          ]);
        }
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 500 });
    // Session cwd is /repo, but we search with /repo/.worktrees/branch
    readFileMock.mockResolvedValue(
      JSON.stringify({ payload: { id: sessionUuid, cwd: "/repo" } }),
    );

    // Should match: /repo/.worktrees/branch starts with /repo
    const id = await findLatestCodexSessionId({
      cwd: "/repo/.worktrees/branch",
    });
    expect(id).toBe(sessionUuid);
  });

  it("findLatestCodexSessionId falls back when cwd is missing in session file", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    const sessionUuid = "99999999-9999-9999-9999-999999999999";

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025/12")) {
          return Promise.resolve([
            dirent(`rollout-${sessionUuid}.jsonl`, "file"),
          ]);
        }
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 700 });
    readFileMock.mockResolvedValue("{}");

    const id = await findLatestCodexSessionId({
      cwd: "/repo/.worktrees/branch",
    });
    expect(id).toBe(sessionUuid);
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

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025/12")) {
          return Promise.resolve([
            dirent(`rollout-${earlyUuid}.jsonl`, "file"),
            dirent(`rollout-${lateUuid}.jsonl`, "file"),
          ]);
        }
      }
      return Promise.resolve([]);
    });
    statMock.mockImplementation((filePath: string) => {
      if (filePath.includes(earlyUuid))
        return Promise.resolve({ mtimeMs: 1_000 });
      return Promise.resolve({ mtimeMs: 5_000 });
    });
    readFileMock.mockImplementation((filePath: string) => {
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
    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      calls.push(dir);
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/.codex/sessions")) {
          return Promise.resolve([dirent("2025", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025")) {
          return Promise.resolve([dirent("12", "dir")]);
        }
        if (endsWithPath(dir, "/.codex/sessions/2025/12")) {
          // Only on second pass we expose the file
          if (calls.filter((c) => endsWithPath(c, "/2025/12")).length >= 2) {
            return Promise.resolve([
              dirent(`rollout-${waitUuid}.jsonl`, "file"),
            ]);
          }
          return Promise.resolve([]);
        }
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 5_000 });
    readFileMock.mockResolvedValue(`{"sessionId":"${waitUuid}"}`);

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

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/projects/-repo/sessions")) {
          // First call: no files, second call: file appears
          const count = readdirMock.mock.calls.filter(
            (call) =>
              typeof call[0] === "string" &&
              endsWithPath(call[0], "/projects/-repo/sessions"),
          ).length;
          if (count >= 1) {
            return Promise.resolve([dirent(`${claudeUuid}.jsonl`, "file")]);
          }
          return Promise.resolve([]);
        }
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 10 });
    readFileMock.mockResolvedValue(`{"session_id":"${claudeUuid}"}`);

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

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/projects/-repo--worktrees-branch/sessions")) {
          return Promise.resolve([dirent(`${dotdashUuid}.jsonl`, "file")]);
        }
        return Promise.resolve([]);
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 20 });
    readFileMock.mockResolvedValue(`{"sessionId":"${dotdashUuid}"}`);

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

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        return Promise.resolve([dirent("log.jsonl", "file")]);
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 123 });
    readFileMock.mockResolvedValue(
      `{"session_id":"${jsonlUuid}"}\n{"message":"hello"}`,
    );

    const id = await findLatestClaudeSessionId("/repos/sample");
    expect(id).toBe(jsonlUuid);
    expect(readdir).toHaveBeenCalledWith(
      path.join(
        "/home/test",
        ".claude",
        "projects",
        "-repos-sample",
        "sessions",
      ),
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

    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (equalsPath(dir, "/custom/codex/sessions")) {
          return Promise.resolve([
            dirent(`rollout-${customCodexUuid}.jsonl`, "file"),
          ]);
        }
        if (equalsPath(dir, "/custom/claude/projects/-repo/sessions")) {
          return Promise.resolve([dirent(`${customClaudeUuid}.jsonl`, "file")]);
        }
      }
      return Promise.resolve([]);
    });
    statMock.mockResolvedValue({ mtimeMs: 123 });
    readFileMock.mockImplementation((filePath: string) => {
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
    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (equalsPath(dir, "/home/test/.claude/projects/-repo/sessions")) {
          return Promise.reject(new Error("missing"));
        }
        if (
          equalsPath(dir, "/home/test/.config/claude/projects/-repo/sessions")
        ) {
          return Promise.resolve([dirent(`${cfgClaudeUuid}.jsonl`, "file")]);
        }
      }
      return Promise.resolve([]);
    });

    statMock.mockResolvedValue({ mtimeMs: 10 });
    readFileMock.mockResolvedValue(`{"session_id":"${cfgClaudeUuid}"}`);

    const id = await findLatestClaudeSessionId("/repo");
    expect(id).toBe(cfgClaudeUuid);
  });

  it("reads Claude sessionId from ~/.claude/history.jsonl when per-project sessions are absent", async () => {
    // No per-project sessions
    readdirMock.mockRejectedValue(new Error("missing sessions"));

    const histUuid = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const otherUuid = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";

    // history.jsonl contains entries for other projects and the target one
    // Using /repo/subdir to match the cwd exactly, and /repo to test prefix matching
    const historyContent = [
      `{"project":"/other","sessionId":"${otherUuid}"}`,
      `{"project":"/repo","sessionId":"${histUuid}"}`,
    ].join("\n");

    // Mock stat to return file info for history.jsonl
    statMock.mockImplementation((filePath: string) => {
      if (filePath.endsWith("history.jsonl")) {
        return Promise.resolve({ mtimeMs: 100 });
      }
      return Promise.reject(new Error("not found"));
    });

    readFileMock.mockImplementation((filePath: string) => {
      if (filePath.endsWith("history.jsonl")) {
        return Promise.resolve(historyContent);
      }
      return Promise.reject(new Error("no sessions"));
    });

    const id = await findLatestClaudeSessionId("/repo/subdir");
    expect(id).toBe(histUuid);
  });

  it("returns null when session files are missing", async () => {
    readdirMock.mockRejectedValue(new Error("missing"));
    const codexId = await findLatestCodexSessionId();
    const claudeId = await findLatestClaudeSessionId("/repos/none");
    const geminiId = await findLatestGeminiSessionId("/repos/none");
    expect(codexId).toBeNull();
    expect(claudeId).toBeNull();
    expect(geminiId).toBeNull();
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
    readdirMock.mockImplementation((dir: string, opts?: ReaddirOptions) => {
      if (opts?.withFileTypes) {
        if (endsWithPath(dir, "/.gemini/tmp")) {
          return Promise.resolve([
            dirent("projA", "dir"),
            dirent("projB", "dir"),
          ]);
        }
        if (endsWithPath(dir, "/projA")) {
          return Promise.resolve([dirent("a.json", "file")]);
        }
        if (endsWithPath(dir, "/projB")) {
          return Promise.resolve([dirent("b.json", "file")]);
        }
        return Promise.resolve([]);
      }
      return Promise.resolve([]);
    });
    statMock.mockImplementation((filePath: string) => {
      return Promise.resolve({
        mtimeMs: filePath.includes("b.json") ? 300 : 200,
      });
    });
    // Include cwd field so that the cwd filter matches the requested path
    readFileMock.mockImplementation((filePath: string) => {
      if (filePath.includes("b.json")) {
        return Promise.resolve(
          JSON.stringify({ id: gemini2Uuid, cwd: "/repo" }),
        );
      }
      return Promise.resolve(JSON.stringify({ id: gemini1Uuid, cwd: "/repo" }));
    });

    const id = await findLatestGeminiSessionId("/repo");
    expect(id).toBe(gemini2Uuid);
  });
});
