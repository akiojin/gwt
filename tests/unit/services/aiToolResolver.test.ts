import { describe, it, expect, vi, beforeEach } from "vitest";

const mockExeca = vi.fn();

vi.mock("execa", () => ({
  execa: (...args: unknown[]) => mockExeca(...args),
  default: { execa: (...args: unknown[]) => mockExeca(...args) },
}));

const detectionCommand = process.platform === "win32" ? "where" : "which";

import {
  resolveClaudeCommand,
  resolveCodexCommand,
  buildClaudeArgs,
  buildCodexArgs,
  CODEX_DEFAULT_ARGS,
  AIToolResolutionError,
  __resetBunxCacheForTests,
} from "../../../src/services/aiToolResolver.js";

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
  __resetBunxCacheForTests();
});

describe("aiToolResolver", () => {
  it("resolves claude to local binary when available", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "claude") {
        return { stdout: "/usr/bin/claude" };
      }
      throw new Error(`Unexpected command ${cmd}`);
    });

    const result = await resolveClaudeCommand({ mode: "continue" });
    expect(result).toEqual({
      command: "claude",
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
    expect(result.command).toBe("bunx");
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
      AIToolResolutionError,
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
    expect(result.command).toBe("bunx");
    expect(result.args[0]).toBe("@openai/codex@latest");
    expect(result.args.slice(1)).toEqual(["resume", ...CODEX_DEFAULT_ARGS]);
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
});
