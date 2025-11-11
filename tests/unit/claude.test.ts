import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

vi.mock("fs", () => ({
  existsSync: vi.fn(() => true),
  default: { existsSync: vi.fn(() => true) },
}));

const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout" } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr" } as unknown as NodeJS.WriteStream,
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: vi.fn(),
};

const mockChildStdio = {
  stdin: "inherit" as const,
  stdout: "inherit" as const,
  stderr: "inherit" as const,
  cleanup: vi.fn(),
};

vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
  createChildStdio: vi.fn(() => mockChildStdio),
}));

vi.mock("../../src/services/aiToolResolver", () => {
  class MockResolutionError extends Error {
    constructor(
      public code: string,
      message: string,
      public hints?: string[],
    ) {
      super(message);
      this.name = "AIToolResolutionError";
    }
  }

  const mockResolveClaudeCommand = vi.fn().mockResolvedValue({
    command: "claude",
    args: [],
    usesFallback: false,
  });

  (globalThis as any).__resolverMocks = {
    mockResolveClaudeCommand,
    MockResolutionError,
  };

  return {
    resolveClaudeCommand: mockResolveClaudeCommand,
    AIToolResolutionError: MockResolutionError,
    isClaudeCodeAvailable: vi.fn(),
  };
});

const { mockResolveClaudeCommand, MockResolutionError } = (globalThis as any)
  .__resolverMocks as {
  mockResolveClaudeCommand: ReturnType<typeof vi.fn>;
  MockResolutionError: new (
    code: string,
    message: string,
    hints?: string[],
  ) => Error & { hints?: string[] };
};

import { execa } from "execa";
import { launchClaudeCode } from "../../src/claude.js";

const mockExeca = execa as ReturnType<typeof vi.fn>;
const consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});

describe("launchClaudeCode", () => {
  let originalGetuid: typeof process.getuid | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy.mockClear();
    mockTerminalStreams.exitRawMode.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    mockTerminalStreams.usingFallback = false;
    originalGetuid = process.getuid;

    mockResolveClaudeCommand.mockReset();
    mockResolveClaudeCommand.mockResolvedValue({
      command: "claude",
      args: [],
      usesFallback: false,
    });

    mockExeca.mockReset();
    mockExeca.mockResolvedValue({
      stdout: "",
      stderr: "",
      exitCode: 0,
    });
  });

  afterEach(() => {
    if (originalGetuid) {
      process.getuid = originalGetuid;
    } else {
      delete (process as any).getuid;
    }
  });

  it("passes mode, skipPermissions, and extraArgs to resolver", async () => {
    await launchClaudeCode("/tmp/worktree", {
      mode: "continue",
      skipPermissions: true,
      extraArgs: ["--foo"],
    });

    expect(mockResolveClaudeCommand).toHaveBeenCalledWith(
      expect.objectContaining({
        mode: "continue",
        skipPermissions: true,
        extraArgs: ["--foo"],
      }),
    );
  });

  it("sets IS_SANDBOX when skipPermissions is true and running as root", async () => {
    process.getuid = () => 0;

    await launchClaudeCode("/tmp/worktree", { skipPermissions: true });

    expect(mockExeca).toHaveBeenCalledTimes(1);
    expect(mockExeca).toHaveBeenCalledWith(
      "claude",
      [],
      expect.objectContaining({
        env: expect.objectContaining({ IS_SANDBOX: "1" }),
      }),
    );
  });

  it("does not set IS_SANDBOX when skipPermissions is false", async () => {
    process.getuid = () => 0;

    await launchClaudeCode("/tmp/worktree", { skipPermissions: false });

    expect(mockExeca).toHaveBeenCalledWith(
      "claude",
      [],
      expect.objectContaining({ env: process.env }),
    );
  });

  it("hands off fallback descriptors when Ink terminal falls back", async () => {
    mockTerminalStreams.usingFallback = true;
    mockChildStdio.stdin = 11 as unknown as any;
    mockChildStdio.stdout = 12 as unknown as any;
    mockChildStdio.stderr = 13 as unknown as any;

    await launchClaudeCode("/tmp/worktree");

    expect(mockExeca).toHaveBeenCalledWith(
      "claude",
      [],
      expect.objectContaining({ stdin: 11, stdout: 12, stderr: 13 }),
    );
    expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);
  });

  it("uses resolver-returned fallback command when available", async () => {
    mockResolveClaudeCommand.mockResolvedValue({
      command: "bunx",
      args: ["@anthropic-ai/claude-code@latest", "-c"],
      usesFallback: true,
    });

    await launchClaudeCode("/tmp/worktree");

    expect(mockExeca).toHaveBeenCalledWith(
      "bunx",
      ["@anthropic-ai/claude-code@latest", "-c"],
      expect.objectContaining({ cwd: "/tmp/worktree" }),
    );
  });

  it("wraps resolver errors and surfaces hints", async () => {
    mockResolveClaudeCommand.mockRejectedValue(
      new MockResolutionError("BUNX_NOT_FOUND", "bunx missing", [
        "Install Bun",
      ]),
    );

    await expect(launchClaudeCode("/tmp/worktree")).rejects.toThrow(
      /bunx missing/,
    );
    await expect(launchClaudeCode("/tmp/worktree")).rejects.toThrow(
      /Install Bun/,
    );
  });

  it("maps ENOENT errors to claude-specific guidance", async () => {
    const enoentError = Object.assign(new Error("missing"), { code: "ENOENT" });
    mockExeca.mockRejectedValueOnce(enoentError);

    await expect(launchClaudeCode("/tmp/worktree")).rejects.toThrow(
      /claude command not found/,
    );
    expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);
  });

  it("maps ENOENT errors to bunx guidance when fallback command fails", async () => {
    mockResolveClaudeCommand.mockResolvedValue({
      command: "bunx",
      args: ["@anthropic-ai/claude-code@latest"],
      usesFallback: true,
    });
    const enoentError = Object.assign(new Error("missing"), { code: "ENOENT" });
    mockExeca.mockRejectedValueOnce(enoentError);

    await expect(launchClaudeCode("/tmp/worktree")).rejects.toThrow(
      /bunx command not found/,
    );
  });
});
