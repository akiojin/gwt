import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

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

import { launchClaudeCode } from "../../src/claude.js";
import { execa } from "execa";

// Get typed mock
const mockExeca = execa as ReturnType<typeof vi.fn>;

// Mock console.log to avoid test output clutter
const consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});

describe("launchClaudeCode - Root User Detection", () => {
  let originalGetuid: (() => number) | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy.mockClear();
    mockTerminalStreams.exitRawMode.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
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
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
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
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
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
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify IS_SANDBOX=1 is set
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining([
          "@anthropic-ai/claude-code@latest",
          "--dangerously-skip-permissions",
        ]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
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

      // Verify IS_SANDBOX=1 is NOT set
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: process.env,
        }),
      );

      // Verify --dangerously-skip-permissions is NOT in args
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.not.arrayContaining(["--dangerously-skip-permissions"]),
        expect.anything(),
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

      // Verify IS_SANDBOX=1 is NOT set
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: process.env,
        }),
      );
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

      consoleLogSpy.mockClear();

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

      consoleLogSpy.mockClear();

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
          stdout: 102,
          stderr: 103,
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
      consoleLogSpy.mockClear();
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
          "-c", // continue mode
          "--dangerously-skip-permissions",
          "--verbose", // extra args
        ]),
        expect.objectContaining({
          cwd: "/test/path",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
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
        expect.anything(),
      );
    });
  });
});
