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
  readFileSync: vi.fn(() => "Linux version 6.1.0"),
  default: {
    existsSync: vi.fn(() => true),
    readFileSync: vi.fn(() => "Linux version 6.1.0"),
  },
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
  let originalPlatformDescriptor: PropertyDescriptor | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    mockTerminalStreams.exitRawMode.mockClear();
    stdoutWrite.mockClear();
    stderrWrite.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    // Default execa mock
    mockExeca.mockImplementation(() => createChildProcess());
    // Store original getuid
    originalGetuid = process.getuid;
    originalPlatformDescriptor = Object.getOwnPropertyDescriptor(
      process,
      "platform",
    );
  });

  afterEach(() => {
    // Restore original getuid
    if (originalGetuid !== undefined) {
      process.getuid = originalGetuid;
    } else {
      delete (process as unknown as { getuid?: () => number }).getuid;
    }

    if (originalPlatformDescriptor) {
      Object.defineProperty(process, "platform", originalPlatformDescriptor);
    }
  });

  it("captures sessionId from file-based detection and returns it", async () => {
    process.getuid = () => 1000;

    // Mock findLatestClaudeSession to return session info
    const mockFindLatestClaudeSession =
      sessionUtils.findLatestClaudeSession as unknown as ReturnType<
        typeof vi.fn
      >;
    mockFindLatestClaudeSession.mockResolvedValueOnce({
      id: "123e4567-e89b-12d3-a456-426614174000",
      cwd: "/test/path",
    });
    mockExeca
      .mockRejectedValueOnce(new Error("Command not found"))
      .mockReturnValueOnce(createChildProcess());

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
        });

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify execa was called with IS_SANDBOX=1 in env
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdout: "inherit",
          stderr: "inherit",
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
        });

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even for non-root users
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
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
      delete (process as unknown as { getuid?: () => number }).getuid;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even when getuid is unavailable
      // 2nd call should be bunx (1st call is which/where check)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
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
        .mockReturnValueOnce(createChildProcess());

      await launchClaudeCode("/test/path", { skipPermissions: true });

      const bunxCall = mockExeca.mock.calls[1];
      expect(bunxCall[0]).toBe("bunx");
      expect(bunxCall[1] as string[]).toEqual(
        expect.arrayContaining([
          "@anthropic-ai/claude-code@latest",
          "--dangerously-skip-permissions",
        ]),
      );
      const options = bunxCall[2] as Record<string, unknown>;
      expect(options.stdout).toBe("inherit");
      expect(options.stderr).toBe("inherit");
      expect(
        (options.env as Record<string, string> | undefined)?.IS_SANDBOX,
      ).toBe("1");
    });
  });

  describe("Continue mode without saved session", () => {
    it("falls back to new session when no sessionId is provided", async () => {
      // which/where fails so bunx path is used
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 }); // bunx

      await launchClaudeCode("/test/path", { mode: "continue" });

      // Second call is the actual launch (bunx)
      const bunxCall = mockExeca.mock.calls[1];
      expect(bunxCall[0]).toBe("bunx");
      const args = bunxCall[1] as string[];
      expect(args).not.toContain("-c");
      expect(args).not.toContain("--resume");
    });
  });

  describe("Chrome integration flag", () => {
    const originalWslDistro = process.env.WSL_DISTRO_NAME;
    const originalWslInterop = process.env.WSL_INTEROP;

    afterEach(() => {
      if (originalWslDistro === undefined) {
        delete process.env.WSL_DISTRO_NAME;
      } else {
        process.env.WSL_DISTRO_NAME = originalWslDistro;
      }
      if (originalWslInterop === undefined) {
        delete process.env.WSL_INTEROP;
      } else {
        process.env.WSL_INTEROP = originalWslInterop;
      }
    });

    it("adds --chrome on supported platforms", async () => {
      delete process.env.WSL_DISTRO_NAME;
      delete process.env.WSL_INTEROP;

      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

      await launchClaudeCode("/test/path", { chrome: true });

      const bunxCall = mockExeca.mock.calls[1];
      const args = bunxCall[1] as string[];
      expect(args).toContain("--chrome");
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Chrome integration enabled"),
      );
    });

    it("adds --chrome on Windows platform", async () => {
      Object.defineProperty(process, "platform", {
        ...(originalPlatformDescriptor ?? {
          configurable: true,
          enumerable: true,
          writable: false,
        }),
        value: "win32",
      });

      delete process.env.WSL_DISTRO_NAME;
      delete process.env.WSL_INTEROP;

      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

      await launchClaudeCode("/test/path", { chrome: true });

      const npxCall = mockExeca.mock.calls.find((call) => call[0] === "npx");
      expect(npxCall).toBeTruthy();
      const args = (npxCall?.[1] ?? []) as string[];
      expect(args).toContain("--chrome");
    });

    it("adds --chrome on macOS platform", async () => {
      Object.defineProperty(process, "platform", {
        ...(originalPlatformDescriptor ?? {
          configurable: true,
          enumerable: true,
          writable: false,
        }),
        value: "darwin",
      });

      delete process.env.WSL_DISTRO_NAME;
      delete process.env.WSL_INTEROP;

      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

      await launchClaudeCode("/test/path", { chrome: true });

      const bunxCall = mockExeca.mock.calls[1];
      const args = bunxCall[1] as string[];
      expect(args).toContain("--chrome");
    });

    it("skips --chrome on WSL environments", async () => {
      process.env.WSL_DISTRO_NAME = "Ubuntu";
      delete process.env.WSL_INTEROP;

      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

      await launchClaudeCode("/test/path", { chrome: true });

      const bunxCall = mockExeca.mock.calls[1];
      const args = bunxCall[1] as string[];
      expect(args).not.toContain("--chrome");
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Chrome integration is not supported"),
      );
    });
  });

  describe("T106: IS_SANDBOX=1 not set when skipPermissions=false", () => {
    it("should not set IS_SANDBOX=1 when skipPermissions=false even if root", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Temporarily remove IS_SANDBOX from process.env if present
      const originalIsSandbox = process.env.IS_SANDBOX;
      delete process.env.IS_SANDBOX;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

      await launchClaudeCode("/test/path", { skipPermissions: false });

      const bunxCall = mockExeca.mock.calls[1];
      const options = bunxCall[2] as Record<string, unknown>;
      expect(options.stdout).toBe("inherit");
      expect(options.stderr).toBe("inherit");
      expect(
        options.env && (options.env as Record<string, string>).IS_SANDBOX,
      ).toBeUndefined();

      // Verify --dangerously-skip-permissions is NOT in args
      expect(bunxCall[1] as string[]).not.toContain(
        "--dangerously-skip-permissions",
      );

      // Restore IS_SANDBOX
      if (originalIsSandbox !== undefined) {
        process.env.IS_SANDBOX = originalIsSandbox;
      }
    });

    it("should not set IS_SANDBOX=1 when skipPermissions is undefined", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Temporarily remove IS_SANDBOX from process.env if present
      const originalIsSandbox = process.env.IS_SANDBOX;
      delete process.env.IS_SANDBOX;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

      await launchClaudeCode("/test/path", {});

      const bunxCall = mockExeca.mock.calls[1];
      const options = bunxCall[2] as Record<string, unknown>;
      expect(options.stdout).toBe("inherit");
      expect(options.stderr).toBe("inherit");
      expect(
        (options.env as Record<string, string> | undefined)?.IS_SANDBOX,
      ).toBeUndefined();

      // Restore IS_SANDBOX
      if (originalIsSandbox !== undefined) {
        process.env.IS_SANDBOX = originalIsSandbox;
      }
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
        });

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
        });

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
        });

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
      mockChildStdio.stdin = 101;
      mockChildStdio.stdout = 102;
      mockChildStdio.stderr = 103;

      // Mock which/where to fail (claude not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

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
      consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
      mockTerminalStreams.exitRawMode.mockClear();
      stdoutWrite.mockClear();
      stderrWrite.mockClear();
      mockChildStdio.cleanup.mockClear();
      mockChildStdio.stdin = "inherit";
      mockChildStdio.stdout = "inherit";
      mockChildStdio.stderr = "inherit";
      mockExeca.mockImplementation(() => createChildProcess());
    });

    it("should use locally installed claude command when available", async () => {
      // Mock which/where command to indicate claude is available
      mockExeca
        .mockResolvedValueOnce({
          // First call: which/where claude (success)
          stdout: "/usr/local/bin/claude",
          stderr: "",
          exitCode: 0,
        })
        .mockResolvedValueOnce({
          // Second call: claude execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

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
        });

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
          stdout: "inherit",
          stderr: "inherit",
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
        })
        .mockResolvedValueOnce({
          // Second call: claude execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        });

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
        });

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
          stdout: "inherit",
          stderr: "inherit",
        }),
      );
    });
  });

  describe("Windows fallback", () => {
    it("uses npx when claude is missing and npx is available", async () => {
      Object.defineProperty(process, "platform", {
        ...(originalPlatformDescriptor ?? {
          configurable: true,
          enumerable: true,
          writable: false,
        }),
        value: "win32",
      });

      // where claude fails, where npx succeeds, then npx runs
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // where claude
        .mockResolvedValueOnce({ stdout: String.raw`C:\bin\npx.cmd` }) // where npx
        .mockResolvedValueOnce(createChildProcess()); // npx execution

      await launchClaudeCode("/test/path");

      expect(mockExeca).toHaveBeenNthCalledWith(
        1,
        "where",
        ["claude"],
        expect.objectContaining({ shell: true }),
      );
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "where",
        ["npx"],
        expect.objectContaining({ shell: true }),
      );
      expect(mockExeca).toHaveBeenNthCalledWith(
        3,
        "npx",
        expect.arrayContaining(["-y", "@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          cwd: "/test/path",
          stdout: "inherit",
          stderr: "inherit",
        }),
      );

      const calledCommands = mockExeca.mock.calls.map((call) => call[0]);
      expect(calledCommands).not.toContain("bunx");
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
        });

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
