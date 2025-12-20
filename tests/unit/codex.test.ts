import { describe, it, expect, vi, beforeEach } from "vitest";
import * as sessionUtils from "../../src/utils/session.js";

// Mock modules before importing
vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

const { mockResolveCodexCommand, MockResolutionError } = vi.hoisted(() => ({
  mockResolveCodexCommand: vi
    .fn()
    .mockImplementation(async (options?: { args?: string[] }) => ({
      command: "bunx",
      args: ["@openai/codex@latest", ...(options?.args ?? [])],
      usesFallback: true,
    })),
  MockResolutionError: class MockResolutionError extends Error {},
}));

vi.mock("../../src/services/aiToolResolver", () => ({
  resolveCodexCommand: mockResolveCodexCommand,
  AIToolResolutionError: MockResolutionError,
  isCodexAvailable: vi.fn(),
}));

vi.mock("fs", () => ({
  existsSync: vi.fn(() => true),
  default: { existsSync: vi.fn(() => true) },
}));

vi.mock("os", () => ({
  platform: vi.fn(() => "darwin"),
  default: { platform: vi.fn(() => "darwin") },
}));

const stdoutWrite = vi.fn();
const stderrWrite = vi.fn();

const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout", write: stdoutWrite } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr", write: stderrWrite } as unknown as NodeJS.WriteStream,
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: vi.fn(),
};

const mockChildStdio: {
  stdin: unknown;
  stdout: unknown;
  stderr: unknown;
  cleanup: ReturnType<typeof vi.fn>;
} = {
  stdin: "inherit",
  stdout: "inherit",
  stderr: "inherit",
  cleanup: vi.fn(),
};

vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
  createChildStdio: vi.fn(() => mockChildStdio),
  resetTerminalModes: vi.fn(),
}));
vi.mock("../../src/utils/session", () => ({
  findLatestCodexSession: vi.fn(async () => null),
}));

import { execa } from "execa";
import {
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_REASONING_EFFORT,
  buildDefaultCodexArgs,
  launchCodexCLI,
} from "../../src/codex";

const mockExeca = execa as ReturnType<typeof vi.fn>;

describe("codex.ts", () => {
  const worktreePath = "/tmp/worktree";

  beforeEach(() => {
    vi.clearAllMocks();
    mockTerminalStreams.exitRawMode.mockClear();
    stdoutWrite.mockClear();
    stderrWrite.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    mockExeca.mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });
  });

  it("uses gpt-5.2-codex as the default model", () => {
    expect(DEFAULT_CODEX_MODEL).toBe("gpt-5.2-codex");
  });

  it("invokes resolver and executes returned command", async () => {
    await launchCodexCLI(worktreePath);

    expect(mockResolveCodexCommand).toHaveBeenCalled();
    const [command, args, options] = mockExeca.mock.calls[0];
    expect(command).toBe("bunx");
    expect(args).toContain("@openai/codex@latest");
    expect(options).toMatchObject({
      cwd: worktreePath,
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
    });
  });

  it("captures sessionId from file-based session detection", async () => {
    const mockFindLatestCodexSession =
      sessionUtils.findLatestCodexSession as unknown as ReturnType<
        typeof vi.fn
      >;
    mockFindLatestCodexSession.mockResolvedValueOnce({
      id: "019af999-aaaa-bbbb-cccc-123456789abc",
      fullPath: "/mock/path/session.jsonl",
      mtime: new Date(),
    });

    const result = await launchCodexCLI(worktreePath);
    expect(result.sessionId).toBe("019af999-aaaa-bbbb-cccc-123456789abc");
  });

  it("places extra arguments before the default set", async () => {
    await launchCodexCLI(worktreePath, { extraArgs: ["--custom-flag"] });

    const args = mockResolveCodexCommand.mock.calls[0]?.[0]?.args as string[];
    expect(args).toEqual([
      "--custom-flag",
      ...buildDefaultCodexArgs(
        DEFAULT_CODEX_MODEL,
        DEFAULT_CODEX_REASONING_EFFORT,
      ),
    ]);
  });

  it("adds resume args when mode is continue", async () => {
    await launchCodexCLI(worktreePath, { mode: "continue" });

    const args = mockResolveCodexCommand.mock.calls[0]?.[0]?.args as string[];
    expect(args).toEqual([
      "resume",
      "--last",
      ...buildDefaultCodexArgs(
        DEFAULT_CODEX_MODEL,
        DEFAULT_CODEX_REASONING_EFFORT,
      ),
    ]);
  });

  it("adds --yolo when bypassApprovals is true", async () => {
    await launchCodexCLI(worktreePath, { bypassApprovals: true });

    const args = mockResolveCodexCommand.mock.calls[0]?.[0]?.args as string[];
    expect(args).toContain("--yolo");
  });
});
