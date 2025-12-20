import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as sessionUtils from "../../src/utils/session.js";

// Mock execa before importing
vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

const { mockResolveClaudeCommand, MockResolutionError } = vi.hoisted(() => ({
  mockResolveClaudeCommand: vi
    .fn()
    .mockImplementation(async (options?: { args?: string[] }) => ({
      command: "bunx",
      args: ["@anthropic-ai/claude-code@latest", ...(options?.args ?? [])],
      usesFallback: true,
    })),
  MockResolutionError: class MockResolutionError extends Error {
    code = "MOCK";
  },
}));

vi.mock("../../src/services/aiToolResolver", () => ({
  resolveClaudeCommand: mockResolveClaudeCommand,
  AIToolResolutionError: MockResolutionError,
  isClaudeCodeAvailable: vi.fn(),
}));

vi.mock("fs", () => ({
  existsSync: vi.fn(() => true),
  default: { existsSync: vi.fn(() => true) },
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
  findLatestClaudeSession: vi.fn(async () => null),
}));

import { launchClaudeCode } from "../../src/claude.js";
import { execa } from "execa";

const mockExeca = execa as ReturnType<typeof vi.fn>;

describe("launchClaudeCode", () => {
  let originalGetuid: (() => number) | undefined;

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
    originalGetuid = process.getuid;
  });

  afterEach(() => {
    if (originalGetuid !== undefined) {
      process.getuid = originalGetuid;
    } else {
      delete (process as unknown as { getuid?: () => number }).getuid;
    }
  });

  it("returns sessionId from file-based detection", async () => {
    const mockFindLatestClaudeSession =
      sessionUtils.findLatestClaudeSession as unknown as ReturnType<
        typeof vi.fn
      >;
    mockFindLatestClaudeSession.mockResolvedValueOnce({
      id: "123e4567-e89b-12d3-a456-426614174000",
      cwd: "/test/path",
    });

    const result = await launchClaudeCode("/test/path", {});
    expect(result.sessionId).toBe("123e4567-e89b-12d3-a456-426614174000");
  });

  it("adds skip-permissions args and sets IS_SANDBOX when skipPermissions=true", async () => {
    process.getuid = () => 0;

    await launchClaudeCode("/test/path", { skipPermissions: true });

    const resolverArgs = mockResolveClaudeCommand.mock.calls[0]?.[0]?.args;
    expect(resolverArgs).toEqual(
      expect.arrayContaining(["--dangerously-skip-permissions"]),
    );

    const options = mockExeca.mock.calls[0]?.[2] as Record<string, unknown>;
    expect((options.env as Record<string, string>).IS_SANDBOX).toBe("1");
  });

  it("does not set IS_SANDBOX when skipPermissions=false", async () => {
    const originalIsSandbox = process.env.IS_SANDBOX;
    delete process.env.IS_SANDBOX;

    await launchClaudeCode("/test/path", { skipPermissions: false });

    const options = mockExeca.mock.calls[0]?.[2] as Record<string, unknown>;
    expect(
      (options.env as Record<string, string> | undefined)?.IS_SANDBOX,
    ).toBeUndefined();

    const resolverArgs = mockResolveClaudeCommand.mock.calls[0]?.[0]?.args;
    expect(resolverArgs).not.toContain("--dangerously-skip-permissions");

    if (originalIsSandbox !== undefined) {
      process.env.IS_SANDBOX = originalIsSandbox;
    }
  });

  it("passes resume session arguments when sessionId is provided", async () => {
    await launchClaudeCode("/test/path", {
      mode: "resume",
      sessionId: "session-123",
    });

    const resolverArgs = mockResolveClaudeCommand.mock.calls[0]?.[0]?.args;
    expect(resolverArgs).toEqual(
      expect.arrayContaining(["--resume", "session-123"]),
    );
  });
});
