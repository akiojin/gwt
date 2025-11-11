import { describe, it, expect, vi, beforeEach } from "vitest";

const mockExeca = vi.fn();
const mockGetToolById = vi.fn();

vi.mock("execa", () => ({
  execa: (...args: unknown[]) => mockExeca(...args),
  default: { execa: (...args: unknown[]) => mockExeca(...args) },
}));

vi.mock("../../src/config/tools.js", () => ({
  getToolById: (...args: unknown[]) => mockGetToolById(...args),
}));

const detectionCommand = process.platform === "win32" ? "where" : "which";

import {
  resolveClaudeCommand,
  resolveCodexCommand,
  resolveCustomToolCommand,
  buildClaudeArgs,
  buildCodexArgs,
  AIToolResolutionError,
  __resetBunxCacheForTests,
} from "../../src/services/aiToolResolver.js";
import { CODEX_DEFAULT_ARGS } from "../../src/shared/aiToolConstants.js";

function notFoundError(): Error {
  const error: any = new Error("not found");
  error.code = "ENOENT";
  return error;
}

beforeEach(() => {
  mockExeca.mockReset();
  mockGetToolById.mockReset();
  __resetBunxCacheForTests();
});

describe("aiToolResolver", () => {
  it("resolves claude to local binary when available", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "claude") {
        return { stdout: "/usr/bin/claude" } as any;
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
        return { stdout: "/usr/bin/bunx" } as any;
      }
      if (cmd === "bun" && args[0] === "--version") {
        return { stdout: "1.0.0" } as any;
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
        return { stdout: "/usr/bin/bunx" } as any;
      }
      if (cmd === "bun" && args[0] === "--version") {
        return { stdout: "1.2.3" } as any;
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

  it("resolves custom tools defined in tools.json", async () => {
    mockGetToolById.mockResolvedValue({
      id: "aider",
      displayName: "Aider",
      type: "bunx",
      command: "@paul-gauthier/aider@latest",
      modeArgs: {
        normal: ["--auto-commit"],
        continue: ["--resume"],
      },
      permissionSkipArgs: ["--yes"],
      env: { AIDER_DEBUG: "1" },
    });

    const result = await resolveCustomToolCommand({
      toolId: "aider",
      mode: "continue",
      skipPermissions: true,
      extraArgs: ["--branch", "feature/x"],
    });

    expect(result.command).toBe("bunx");
    expect(result.args).toEqual([
      "@paul-gauthier/aider@latest",
      "--resume",
      "--yes",
      "--branch",
      "feature/x",
    ]);
    expect(result.env?.AIDER_DEBUG).toBe("1");
  });

  it("resolved PATH command for custom tools", async () => {
    mockGetToolById.mockResolvedValue({
      id: "local-tool",
      displayName: "Local Tool",
      type: "command",
      command: "my-tool",
      modeArgs: {
        normal: [],
      },
    });

    mockExeca.mockImplementation(async (cmd: string, args: unknown[]) => {
      if (cmd === detectionCommand && (args as string[])[0] === "my-tool") {
        return { stdout: "/usr/local/bin/my-tool" } as any;
      }
      return { stdout: "" } as any;
    });

    const result = await resolveCustomToolCommand({
      toolId: "local-tool",
    });

    expect(result.command).toBe("/usr/local/bin/my-tool");
    expect(result.usesFallback).toBe(false);
  });

  it("throws when custom tool id is unknown", async () => {
    mockGetToolById.mockResolvedValue(undefined);

    await expect(
      resolveCustomToolCommand({ toolId: "missing" }),
    ).rejects.toBeInstanceOf(AIToolResolutionError);
  });
});
