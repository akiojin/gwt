import { describe, it, expect, mock, beforeEach } from "bun:test";

const mockExeca = mock();
const mockGetToolById = mock();

mock.module("execa", () => ({
  execa: (...args: unknown[]) => mockExeca(...args),
  default: { execa: (...args: unknown[]) => mockExeca(...args) },
}));

mock.module("../../src/config/tools.js", () => ({
  getCodingAgentById: (...args: unknown[]) => mockGetToolById(...args),
}));

const detectionCommand = process.platform === "win32" ? "where" : "which";

import {
  resolveClaudeCommand,
  resolveCodexCommand,
  resolveCodingAgentCommand,
  buildClaudeArgs,
  buildCodexArgs,
  CodingAgentResolutionError,
  __resetBunxCacheForTests,
} from "../../src/services/codingAgentResolver.js";
import { CODEX_DEFAULT_ARGS } from "../../src/shared/codingAgentConstants.js";

function notFoundError(): Error & { code: string } {
  const error = new Error("not found") as Error & { code: string };
  error.code = "ENOENT";
  return error;
}

beforeEach(() => {
  mockExeca.mockReset();
  mockGetToolById.mockReset();
  __resetBunxCacheForTests();
});

describe("codingAgentResolver", () => {
  it("resolves claude to local binary when available", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "claude") {
        return { stdout: "/usr/bin/claude" } as { stdout: string };
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
        return { stdout: "/usr/bin/bunx" } as { stdout: string };
      }
      if (cmd === "bun" && args[0] === "--version") {
        return { stdout: "1.0.0" } as { stdout: string };
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

  it("resolves Codex fallback command with composed args", async () => {
    mockExeca.mockImplementation(async (cmd, args) => {
      if (cmd === detectionCommand && args[0] === "codex") {
        throw notFoundError();
      }
      if (cmd === detectionCommand && args[0] === "bunx") {
        return { stdout: "/usr/bin/bunx" } as { stdout: string };
      }
      if (cmd === "bun" && args[0] === "--version") {
        return { stdout: "1.2.3" } as { stdout: string };
      }
      throw new Error(`Unexpected command ${cmd}`);
    });

    const result = await resolveCodexCommand({ mode: "resume" });
    expect(result.command).toBe("/usr/bin/bunx");
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

  it("resolves custom coding agents defined in tools.json", async () => {
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

    const result = await resolveCodingAgentCommand({
      agentId: "aider",
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

  it("resolves PATH command for custom coding agents", async () => {
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
        return { stdout: "/usr/local/bin/my-tool" } as { stdout: string };
      }
      return { stdout: "" } as { stdout: string };
    });

    const result = await resolveCodingAgentCommand({
      agentId: "local-tool",
    });

    expect(result.command).toBe("/usr/local/bin/my-tool");
    expect(result.usesFallback).toBe(false);
  });

  it("throws when custom coding agent id is unknown", async () => {
    mockGetToolById.mockResolvedValue(undefined);

    await expect(
      resolveCodingAgentCommand({ agentId: "missing" }),
    ).rejects.toBeInstanceOf(CodingAgentResolutionError);
  });
});
