import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventEmitter } from "node:events";
import * as sessionUtils from "../../src/utils/session.js";

// Mock execa before importing
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
  waitForClaudeSessionId: vi.fn(async () => null),
  findLatestClaudeSessionId: vi.fn(async () => null),
  findLatestClaudeSession: vi.fn(async () => null),
}));

import { launchClaudeCode } from "../../src/claude.js";
import { execa } from "execa";

// Get typed mock
const mockExeca = execa as ReturnType<typeof vi.fn>;

// Mock console.log to avoid test output clutter
let consoleLogSpy: ReturnType<typeof vi.spyOn>;
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

describe("launchClaudeCode - Root User Detection", () => {
  let originalGetuid: (() => number) | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    mockTerminalStreams.exitRawMode.mockClear();
    (mockTerminalStreams.stdout.write as any).mockClear?.();
    (mockTerminalStreams.stderr.write as any).mockClear?.();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    // Default execa mock
    (mockExeca as any).mockImplementation(() => createChildProcess() as any);
    // Store original getuid
    originalGetuid = process.getuid;
  });

  afterEach(() => {
    // Restore original getuid
    if (originalGetuid !== undefined) {
      process.getuid = originalGetuid;
    } else {
      delete (process as any).getuid;
    }
  });

  it("captures sessionId from stdout and returns it", async () => {
    process.getuid = () => 1000;

    (sessionUtils.waitForClaudeSessionId as any).mockResolvedValueOnce(
      "123e4567-e89b-12d3-a456-426614174000",
    );
    (mockExeca as any)
      .mockRejectedValueOnce(new Error("Command not found"))
      .mockReturnValueOnce(
        createChildProcess((stdout) => {
          stdout.emit(
            "data",
            "Session ID: 123e4567-e89b-12d3-a456-426614174000",
          );
        }) as any,
      );

    const result = await launchClaudeCode("/test/path", {});
    expect(result.sessionId).toBe("123e4567-e89b-12d3-a456-426614174000");
  });

  describe("T104: Root user detection logic", () => {
    it("should detect root user when process.getuid() returns 0", async () => {
      // Mock process.getuid to return 0 (root user)
      process.getuid = () => 0;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify execa was called with IS_SANDBOX=1 in env
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          shell: true,
          stdout: "pipe",
          stderr: "pipe",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    it("should not detect root user when process.getuid() returns non-zero", async () => {
      // Mock process.getuid to return 1000 (non-root user)
      process.getuid = () => 1000;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even for non-root users
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          shell: true,
          stdout: "pipe",
          stderr: "pipe",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    it("should handle environments where process.getuid() is not available", async () => {
      // Mock process without getuid (e.g., Windows)
      delete (process as any).getuid;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even when getuid is unavailable
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          shell: true,
          stdout: "pipe",
          stderr: "pipe",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });
  });

  describe("T105: IS_SANDBOX=1 set when skipPermissions=true and root", () => {
    it("should set IS_SANDBOX=1 when both root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockReturnValueOnce(createChildProcess() as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      const bunxCall = mockExeca.mock.calls[1];
      expect(bunxCall[0]).toBe("bunx");
      expect(bunxCall[1] as string[]).toEqual(
        expect.arrayContaining([
          "@anthropic-ai/claude-code@latest",
          "--dangerously-skip-permissions",
        ]),
      );
      const options = bunxCall[2] as Record<string, any>;
      expect(options.stdout).toBe("pipe");
      expect(options.stderr).toBe("pipe");
      expect(options.env?.IS_SANDBOX).toBe("1");
    });
  });

  describe("Continue mode without saved session", () => {
    it("falls back to new session when no sessionId is provided", async () => {
      // which/where fails so bunx path is used
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 } as any); // bunx

      await launchClaudeCode("/test/path", { mode: "continue" });

      // Second call is the actual launch (bunx)
      const bunxCall = mockExeca.mock.calls[1];
      expect(bunxCall[0]).toBe("bunx");
      const args = bunxCall[1] as string[];
      expect(args).not.toContain("-c");
      expect(args).not.toContain("--resume");
    });
  });

  describe("T106: IS_SANDBOX=1 not set when skipPermissions=false", () => {
    it("should not set IS_SANDBOX=1 when skipPermissions=false even if root", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: false });

      const bunxCall = mockExeca.mock.calls[1];
      const options = bunxCall[2] as Record<string, unknown>;
      expect(options.stdout).toBe("pipe");
      expect(options.stderr).toBe("pipe");
      expect(options.env && (options.env as Record<string, string>).IS_SANDBOX).toBeUndefined();

      // Verify --dangerously-skip-permissions is NOT in args
      expect(bunxCall[1] as string[]).not.toContain(
        "--dangerously-skip-permissions",
      );
    });

    it("should not set IS_SANDBOX=1 when skipPermissions is undefined", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", {});

      const bunxCall = mockExeca.mock.calls[1];
      const options = bunxCall[2] as Record<string, any>;
      expect(options.stdout).toBe("pipe");
      expect(options.stderr).toBe("pipe");
      expect(options.env?.IS_SANDBOX).toBeUndefined();
    });
  });

  describe("T203-T205: Warning message display", () => {
    it("T204: should display warning message when root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify warning messages are displayed
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("⚠️  Skipping permissions check"),
      );
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining(
          "⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
        ),
      );
    });

    it("T205: should not display sandbox warning when non-root user", async () => {
      // Mock non-root user
      process.getuid = () => 1000;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox warning is NOT displayed
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("⚠️  Skipping permissions check"),
      );
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining(
          "⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
        ),
      );
    });

    it("should not display any warning when skipPermissions=false", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: false });

      // Verify no skip permissions warnings are displayed
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining("⚠️  Skipping permissions check"),
      );
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining(
          "⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
        ),
      );
    });
  });

  describe("TTY handoff", () => {
    it("should pass fallback file descriptors when usingFallback is true", async () => {
      mockTerminalStreams.usingFallback = true;
      mockChildStdio.stdin = 101 as unknown as any;
      mockChildStdio.stdout = 102 as unknown as any;
      mockChildStdio.stderr = 103 as unknown as any;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

    await launchClaudeCode("/test/path");

    // 2nd call should be bunx (1st call is which/where check)
    expect(mockExeca).toHaveBeenNthCalledWith(
      2,
      "bunx",
      expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
      expect.objectContaining({
        stdin: 101,
        stdout: "pipe",
        stderr: "pipe",
      }),
    );

      expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);

      mockTerminalStreams.usingFallback = false;
      mockChildStdio.stdin = "inherit";
      mockChildStdio.stdout = "inherit";
      mockChildStdio.stderr = "inherit";
    });
  });

  describe("T504: Claude command auto-detection", () => {
    // Clear the default mock from parent beforeEach for these tests
    beforeEach(() => {
      vi.clearAllMocks();
      consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
      mockTerminalStreams.exitRawMode.mockClear();
      (mockTerminalStreams.stdout.write as any).mockClear?.();
      (mockTerminalStreams.stderr.write as any).mockClear?.();
      mockChildStdio.cleanup.mockClear();
      mockChildStdio.stdin = "inherit";
      mockChildStdio.stdout = "inherit";
      mockChildStdio.stderr = "inherit";
      (mockExeca as any).mockImplementation(
        () => createChildProcess() as any,
      );
    });

    it("should use locally installed claude command when available", async () => {
      // Mock which/where command to indicate claude is available
      mockExeca
        .mockResolvedValueOnce({
          // First call: which/where claude (success)
          stdout: "/usr/local/bin/claude",
          stderr: "",
          exitCode: 0,
        } as any)
        .mockResolvedValueOnce({
          // Second call: claude execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path");

      // First call should be which/where to check claude availability
      expect(mockExeca).toHaveBeenNthCalledWith(
        1,
        expect.stringMatching(/which|where/),
        ["claude"],
        expect.objectContaining({ shell: true }),
      );

      // Second call should be the actual claude command (not bunx)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "claude",
        expect.any(Array),
        expect.objectContaining({
          cwd: "/test/path",
          shell: true,
          stdout: "inherit",
          stderr: "inherit",
        }),
      );

      // Verify log message for using local claude
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Using locally installed claude command"),
      );
    });

    it("should fallback to bunx when claude command is not available", async () => {
      // Mock which/where command to indicate claude is NOT available
      mockExeca
        .mockRejectedValueOnce(
          // First call: which/where claude (failure)
          new Error("Command not found"),
        )
        .mockResolvedValueOnce({
          // Second call: bunx execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path");

      // First call should be which/where to check claude availability
      expect(mockExeca).toHaveBeenNthCalledWith(
        1,
        expect.stringMatching(/which|where/),
        ["claude"],
        expect.objectContaining({ shell: true }),
      );

      // Second call should be bunx (fallback)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          cwd: "/test/path",
          shell: true,
          stdout: "pipe",
          stderr: "pipe",
        }),
      );

      // Verify log message for bunx fallback
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Falling back to bunx"),
      );
    });

    it("should pass arguments correctly when using local claude command", async () => {
      // Mock which/where command to indicate claude is available
      mockExeca
        .mockResolvedValueOnce({
          // First call: which/where claude
          stdout: "/usr/local/bin/claude",
          stderr: "",
          exitCode: 0,
        } as any)
        .mockResolvedValueOnce({
          // Second call: claude execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", {
        mode: "continue",
        skipPermissions: true,
        extraArgs: ["--verbose"],
      });

      // Verify arguments are passed correctly to claude command
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "claude",
        expect.arrayContaining([
          "--dangerously-skip-permissions",
          "--verbose", // extra args
        ]),
        expect.objectContaining({
          cwd: "/test/path",
          shell: true,
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );

      const args = mockExeca.mock.calls[1][1] as string[];
      expect(args).not.toContain("-c");
      expect(args).not.toContain("--resume");
    });

    it("should pass arguments correctly when using bunx fallback", async () => {
      // Mock which/where command to indicate claude is NOT available
      mockExeca
        .mockRejectedValueOnce(
          // First call: which/where claude (failure)
          new Error("Command not found"),
        )
        .mockResolvedValueOnce({
          // Second call: bunx execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", {
        mode: "resume",
        extraArgs: ["--debug"],
      });

      // Verify arguments are passed correctly to bunx command
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining([
          "@anthropic-ai/claude-code@latest",
          "-r", // resume mode
          "--debug", // extra args
        ]),
        expect.objectContaining({
          shell: true,
          stdout: "pipe",
          stderr: "pipe",
        }),
      );
    });
  });

  describe("FR-008: Launch arguments display", () => {
    it("should display launch arguments in console log", async () => {
      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify that args are logged with 📋 prefix
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("📋 Args:"),
      );

      // Verify that the actual arguments are included in the log
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("--dangerously-skip-permissions"),
      );
    });
  });
});
