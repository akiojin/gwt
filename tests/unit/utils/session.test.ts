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
} from "../../../src/utils/session.js";

describe("utils/session", () => {
  beforeEach(() => {
    vi.clearAllMocks();
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

  it("findLatestClaudeSessionId reads JSONL lines and extracts session_id", async () => {
    const dirent = (name: string, type: "file" | "dir") => ({
      name,
      isFile: () => type === "file",
      isDirectory: () => type === "dir",
    });

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        return Promise.resolve([dirent("log.jsonl", "file")]);
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 123 });
    (readFile as any).mockResolvedValue(
      '{"session_id":"abc-123"}\n{"message":"hello"}',
    );

    const id = await findLatestClaudeSessionId("/repos/sample");
    expect(id).toBe("abc-123");
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

    process.env.CODEX_HOME = "/custom/codex";
    process.env.CLAUDE_CONFIG_DIR = "/custom/claude";

    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir === "/custom/codex/sessions") {
          return Promise.resolve([dirent("sid.json", "file")]);
        }
        if (dir === "/custom/claude/projects/-repo/sessions") {
          return Promise.resolve([dirent("log.jsonl", "file")]);
        }
      }
      return Promise.resolve([]);
    });
    (stat as any).mockResolvedValue({ mtimeMs: 123 });
    (readFile as any).mockImplementation((filePath: string) => {
      if (filePath.endsWith("sid.json")) {
        return Promise.resolve(JSON.stringify({ id: "custom-codex" }));
      }
      return Promise.resolve('{"session_id":"custom-claude"}');
    });

    const codexId = await findLatestCodexSessionId();
    const claudeId = await findLatestClaudeSessionId("/repo");
    expect(codexId).toBe("custom-codex");
    expect(claudeId).toBe("custom-claude");

    delete process.env.CODEX_HOME;
    delete process.env.CLAUDE_CONFIG_DIR;
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
    (readdir as any).mockImplementation((dir: string, opts?: any) => {
      if (opts?.withFileTypes) {
        if (dir.endsWith("/.gemini/tmp")) {
          return Promise.resolve([dirent("projA", "dir"), dirent("projB", "dir")]);
        }
        if (dir.endsWith("projA/chats")) {
          return Promise.resolve([dirent("a.json", "file")]);
        }
        if (dir.endsWith("projB/chats")) {
          return Promise.resolve([dirent("b.json", "file")]);
        }
        return Promise.resolve([]);
      }
      if (dir.endsWith("/.gemini/tmp")) {
        return Promise.resolve(["projA", "projB"]);
      }
      if (dir.endsWith("projA/chats")) return Promise.resolve(["a.json"]);
      if (dir.endsWith("projB/chats")) return Promise.resolve(["b.json"]);
      return Promise.resolve([]);
    });
    (stat as any).mockImplementation((filePath: string) => {
      return Promise.resolve({
        mtimeMs: filePath.includes("b.json") ? 300 : 200,
      });
    });
    (readFile as any).mockImplementation((filePath: string) => {
      if (filePath.includes("b.json")) {
        return Promise.resolve(JSON.stringify({ id: "gemini-2" }));
      }
      return Promise.resolve(JSON.stringify({ id: "gemini-1" }));
    });

    const id = await findLatestGeminiSessionId("/repo");
    expect(id).toBe("gemini-2");
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
