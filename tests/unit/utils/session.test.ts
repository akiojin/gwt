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
    (readdir as any).mockResolvedValue(["old.json", "new.json"]);
    (stat as any).mockImplementation((filePath: string) => {
      return Promise.resolve({
        mtimeMs: filePath.includes("new.json") ? 200 : 100,
      });
    });
    (readFile as any).mockImplementation((filePath: string) => {
      if (filePath.includes("new.json")) {
        return Promise.resolve(JSON.stringify({ id: "new-session-id" }));
      }
      return Promise.resolve(JSON.stringify({ id: "old-session-id" }));
    });

    const id = await findLatestCodexSessionId();
    expect(id).toBe("new-session-id");
  });

  it("findLatestClaudeSessionId reads JSONL lines and extracts session_id", async () => {
    (readdir as any).mockResolvedValue(["log.jsonl"]);
    (stat as any).mockResolvedValue({ mtimeMs: 123 });
    (readFile as any).mockResolvedValue(
      '{"session_id":"abc-123"}\n{"message":"hello"}',
    );

    const id = await findLatestClaudeSessionId("/repos/sample");
    expect(id).toBe("abc-123");
    expect(readdir).toHaveBeenCalledWith(
      "/home/test/.claude/projects/-repos-sample/sessions",
    );
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
    (readdir as any).mockImplementation((dir: string) => {
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
    (readdir as any).mockImplementation((dir: string) => {
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
