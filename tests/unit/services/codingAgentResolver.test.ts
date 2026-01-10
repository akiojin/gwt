import { describe, it, expect, mock, beforeEach } from "bun:test";

const mockExeca = mock();
const mockFindCommand = mock();

mock.module("execa", () => ({
  execa: (...args: unknown[]) => mockExeca(...args),
  default: { execa: (...args: unknown[]) => mockExeca(...args) },
}));

mock.module("../../../src/utils/command", () => ({
  findCommand: (...args: unknown[]) => mockFindCommand(...args),
}));

const detectionCommand = process.platform === "win32" ? "where" : "which";

import {
  resolveClaudeCommand,
  resolveCodexCommand,
  buildClaudeArgs,
  buildCodexArgs,
  CodingAgentResolutionError,
  __resetBunxCacheForTests,
} from "../../../src/services/codingAgentResolver.js";
import { CODEX_DEFAULT_ARGS } from "../../../src/shared/codingAgentConstants.js";

interface ErrorWithCode extends Error {
  code?: string;
}

function notFoundError(): ErrorWithCode {
  const error: ErrorWithCode = new Error("not found");
  error.code = "ENOENT";
  return error;
}

beforeEach(() => {
  mockExeca.mockReset();
  mockFindCommand.mockReset();
  mockFindCommand.mockResolvedValue({
    available: true,
    path: null,
    source: "bunx",
    version: null,
  });
  __resetBunxCacheForTests();
});

describe("codingAgentResolver", () => {
  it("resolves claude to local binary when available", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "claude") {
        return { stdout: "/usr/bin/claude" };
      }
      throw new Error(`Unexpected command ${cmd}`);
    });

    const result = await resolveClaudeCommand({ mode: "continue" });
    expect(result).toMatchObject({
      command: "/usr/bin/claude",
      args: ["-c"],
      usesFallback: false,
    });
  });

  it("falls back to bunx for claude when local binary is missing", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "claude") {
        throw notFoundError();
      }
      if (cmd === detectionCommand && args[0] === "bunx") {
        return { stdout: "/usr/bin/bunx" };
      }
      if (cmd === "bun" && args[0] === "--version") {
        return { stdout: "1.0.0" };
      }
      throw new Error(`Unexpected command ${cmd}`);
    });

    const result = await resolveClaudeCommand({ skipPermissions: true });
    expect(result.command).toBe("/usr/bin/bunx");
    expect(result.args).toEqual([
      "@anthropic-ai/claude-code@latest",
      "--dangerously-skip-permissions",
    ]);
    expect(result.usesFallback).toBe(true);
  });

  it("throws descriptive error when bunx is unavailable", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "claude") {
        throw notFoundError();
      }
      if (cmd === detectionCommand && args[0] === "bunx") {
        throw notFoundError();
      }
      throw new Error(`Unexpected command ${cmd}`);
    });

    await expect(resolveClaudeCommand()).rejects.toBeInstanceOf(
      CodingAgentResolutionError,
    );
  });

  it("builds proper Codex arguments", () => {
    expect(
      buildCodexArgs({
        mode: "continue",
        bypassApprovals: true,
        extraArgs: ["--custom"],
      }),
    ).toEqual([
      "resume",
      "--last",
      "--yolo",
      "--custom",
      ...CODEX_DEFAULT_ARGS,
    ]);
  });

  it("builds Codex arguments with explicit session id", () => {
    expect(
      buildCodexArgs({
        mode: "continue",
        sessionId: "session-123",
        extraArgs: ["--custom"],
      }),
    ).toEqual(["resume", "session-123", "--custom", ...CODEX_DEFAULT_ARGS]);
  });

  it("resolves Codex fallback command with composed args", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "codex") {
        throw notFoundError();
      }
      if (cmd === detectionCommand && args[0] === "bunx") {
        return { stdout: "/usr/bin/bunx" };
      }
      if (cmd === "bun" && args[0] === "--version") {
        return { stdout: "1.2.3" };
      }
      throw new Error(`Unexpected command ${cmd}`);
    });

    const result = await resolveCodexCommand({ mode: "resume" });
    expect(result.command).toBe("/usr/bin/bunx");
    expect(result.args[0]).toBe("@openai/codex@latest");
    expect(result.args.slice(1)).toEqual(["resume", ...CODEX_DEFAULT_ARGS]);
  });

  it("adds --enable skills for installed Codex < 0.80.0", async () => {
    mockFindCommand.mockResolvedValueOnce({
      available: true,
      path: "/usr/bin/codex",
      source: "installed",
      version: "v0.79.0",
    });

    const result = await resolveCodexCommand({ mode: "normal" });
    const enableIndex = result.args.findIndex(
      (arg, i) => arg === "--enable" && result.args[i + 1] === "skills",
    );

    expect(result.command).toBe("/usr/bin/codex");
    expect(result.usesFallback).toBe(false);
    expect(enableIndex).toBeGreaterThan(-1);
  });

  it("builds Claude args with convenience helper", () => {
    expect(
      buildClaudeArgs({
        mode: "resume",
        skipPermissions: true,
        extraArgs: ["--foo"],
      }),
    ).toEqual(["-r", "--dangerously-skip-permissions", "--foo"]);
  });

  it("builds Claude args with explicit session id", () => {
    expect(
      buildClaudeArgs({
        mode: "resume",
        sessionId: "session-456",
      }),
    ).toEqual(["--resume", "session-456"]);
  });
});
