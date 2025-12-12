import { describe, it, expect, vi, beforeEach } from "vitest";
import { EventEmitter } from "node:events";

// Mock modules before importing
vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

vi.mock("fs", () => ({
  existsSync: vi.fn(() => true),
  default: { existsSync: vi.fn(() => true) },
}));

vi.mock("os", () => ({
  platform: vi.fn(() => "darwin"),
  default: { platform: vi.fn(() => "darwin") },
}));

const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout", write: vi.fn() } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr", write: vi.fn() } as unknown as NodeJS.WriteStream,
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
vi.mock("../../src/utils/session", () => ({
  waitForCodexSessionId: vi.fn(async () => null),
  findLatestCodexSession: vi.fn(async () => null),
}));

import { execa } from "execa";
import {
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_REASONING_EFFORT,
  buildDefaultCodexArgs,
  launchCodexCLI,
} from "../../src/codex";

// Get typed mock
const mockExeca = execa as ReturnType<typeof vi.fn>;

const DEFAULT_CODEX_ARGS = buildDefaultCodexArgs(
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_REASONING_EFFORT,
);

const createChildProcess = (
  onEmit?: (stdout: EventEmitter, stderr: EventEmitter) => void,
) => {
  const stdout = new EventEmitter();
  const stderr = new EventEmitter();
  const promise = new Promise((resolve) => {
    setImmediate(() => {
      onEmit?.(stdout, stderr);
      resolve({ stdout, stderr, exitCode: 0 });
    });
  });
  return Object.assign(promise, { stdout, stderr });
};

describe("codex.ts", () => {
  const worktreePath = "/tmp/worktree";

  beforeEach(() => {
    vi.clearAllMocks();
    mockTerminalStreams.exitRawMode.mockClear();
    (mockTerminalStreams.stdout.write as any).mockClear?.();
    (mockTerminalStreams.stderr.write as any).mockClear?.();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    (execa as any).mockImplementation(() => createChildProcess() as any);
  });

  it("should append default Codex CLI arguments on launch", async () => {
    await launchCodexCLI(worktreePath);

    expect(execa).toHaveBeenCalledTimes(1);
    const [, args, options] = (execa as any).mock.calls[0];

    expect(args).toEqual(["@openai/codex@latest", ...DEFAULT_CODEX_ARGS]);
    expect(options).toMatchObject({
      cwd: worktreePath,
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
    });
    // exitRawMode is called once before child process and once in finally block
    expect(mockTerminalStreams.exitRawMode).toHaveBeenCalledTimes(2);
    expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);
  });

  it("captures sessionId from file-based session detection", async () => {
    const { findLatestCodexSession } = await import(
      "../../src/utils/session.js"
    );
    (findLatestCodexSession as any).mockResolvedValueOnce({
      id: "019af999-aaaa-bbbb-cccc-123456789abc",
      fullPath: "/mock/path/session.jsonl",
      mtime: new Date(),
    });

    const result = await launchCodexCLI(worktreePath);
    expect(result.sessionId).toBe("019af999-aaaa-bbbb-cccc-123456789abc");
  });

  it("should place extra arguments before the default set", async () => {
    await launchCodexCLI(worktreePath, { extraArgs: ["--custom-flag"] });

    const [, args] = (execa as any).mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      "--custom-flag",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });

  it("should include resume command arguments before defaults when continuing", async () => {
    await launchCodexCLI(worktreePath, { mode: "continue" });

    const [, args] = (execa as any).mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      "resume",
      "--last",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });

  it("applies provided model and reasoning effort overrides", async () => {
    await launchCodexCLI(worktreePath, {
      model: "gpt-5.1-codex-max",
      reasoningEffort: "xhigh",
    });

    const [, args] = (execa as any).mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      ...buildDefaultCodexArgs("gpt-5.1-codex-max", "xhigh"),
    ]);
  });

  it("passes gpt-5.2 and xhigh reasoning when requested", async () => {
    await launchCodexCLI(worktreePath, {
      model: "gpt-5.2",
      reasoningEffort: "xhigh",
    });

    const [, args] = (execa as any).mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      ...buildDefaultCodexArgs("gpt-5.2", "xhigh"),
    ]);
  });

  it("should hand off fallback file descriptors when stdin is not a TTY", async () => {
    mockTerminalStreams.usingFallback = true;
    mockChildStdio.stdin = 11 as unknown as any;
    mockChildStdio.stdout = 12 as unknown as any;
    mockChildStdio.stderr = 13 as unknown as any;

    await launchCodexCLI(worktreePath);

    const [, , options] = (execa as any).mock.calls[0];
    expect(options).toMatchObject({
      stdin: 11,
      stdout: 12,
      stderr: 13,
    });
    expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);

    mockTerminalStreams.usingFallback = false;
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
  });

  it("should include --enable skills in default arguments (FR-202)", async () => {
    await launchCodexCLI(worktreePath);

    const [, args] = (execa as any).mock.calls[0];

    // Find the index of "--enable" followed by "skills"
    const enableIndex = args.findIndex(
      (arg: string, i: number) =>
        arg === "--enable" && args[i + 1] === "skills",
    );

    expect(enableIndex).toBeGreaterThan(-1);
    expect(args[enableIndex]).toBe("--enable");
    expect(args[enableIndex + 1]).toBe("skills");
  });

  it("should display launch arguments in console log (FR-008)", async () => {
    const consoleSpy = vi.spyOn(console, "log").mockImplementation(() => {});

    await launchCodexCLI(worktreePath);

    // Verify that args are logged with 📋 prefix
    expect(consoleSpy).toHaveBeenCalledWith(
      expect.stringContaining("📋 Args:"),
    );

    // Verify that the actual arguments are included in the log
    expect(consoleSpy).toHaveBeenCalledWith(
      expect.stringContaining("--enable"),
    );
    expect(consoleSpy).toHaveBeenCalledWith(
      expect.stringContaining("skills"),
    );

    consoleSpy.mockRestore();
  });
});
