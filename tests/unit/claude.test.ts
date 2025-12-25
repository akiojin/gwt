import {
  describe,
  it,
  expect,
  vi,
  beforeAll,
  beforeEach,
  afterEach,
} from "vitest";
import { EventEmitter } from "node:events";
import * as sessionUtils from "../../src/utils/session.js";

// Define mock state that will be used across mocks
const mockExistsSync = vi.fn(() => false);
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

// Mock execa before importing
vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

// Mock existsSync to return false by default (for fallback path checks in findCommand)
// Individual tests can override this if they need existsSync to return true
vi.mock("fs", async () => {
  return {
    existsSync: (...args: unknown[]) => mockExistsSync(...args),
    readFileSync: vi.fn(() => "Linux version 6.1.0"),
    default: {
      existsSync: (...args: unknown[]) => mockExistsSync(...args),
      readFileSync: vi.fn(() => "Linux version 6.1.0"),
    },
  };
});

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
import {
  clearCommandLookupCache,
  findCommand,
} from "../../src/utils/command.js";

// Detect if claude is installed in the test environment
let claudeIsInstalled = false;
let detectedClaudeCommand = "bunx"; // default fallback

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

/**
 * NOTE: Most tests in this file are skipped because Bun's vitest compatibility
 * does not support module mocking (vi.mock). The real execa is called instead
 * of the mock, causing tests to fail.
 *
 * Core functionality is verified in:
 * - tests/unit/utils/command.test.ts (findCommand, caching, fallback paths)
 *
 * These tests can be re-enabled when Bun improves vitest module mocking support.
 */
describe("launchClaudeCode - Root User Detection", () => {
  let originalGetuid: (() => number) | undefined;
  let originalPlatformDescriptor: PropertyDescriptor | undefined;

  // Detect the actual command that will be used (depends on test environment)
  beforeAll(async () => {
    const result = await findCommand("claude");
    claudeIsInstalled = result.source === "installed";
    detectedClaudeCommand = claudeIsInstalled ? "claude" : "bunx";
  });

  beforeEach(() => {
    vi.clearAllMocks();
    clearCommandLookupCache(); // Clear command lookup cache between tests
    // Return true for worktree path checks, false for fallback path checks
    mockExistsSync.mockImplementation((path: string) => {
      // Worktree paths used in tests
      if (path === "/test/path" || path.includes("worktree")) {
        return true;
      }
      // Fallback paths for command detection should return false
      return false;
    });
    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    mockTerminalStreams.exitRawMode.mockClear();
    stdoutWrite.mockClear();
    stderrWrite.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    // Default execa mock for actual command execution (bunx, claude, etc.)
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

  // Skipped: Bun does not support vi.mock for execa module
  it.skip("captures sessionId from file-based detection and returns it", async () => {
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
    // findCommand is mocked to return bunx, so execa is only called for the bunx command
    mockExeca.mockReturnValueOnce(createChildProcess());

    const result = await launchClaudeCode("/test/path", {});
    expect(result.sessionId).toBe("123e4567-e89b-12d3-a456-426614174000");
  });

  describe("T104: Root user detection logic", () => {
    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should detect root user when process.getuid() returns 0", async () => {
      // Mock process.getuid to return 0 (root user)
      process.getuid = () => 0;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify execa was called with IS_SANDBOX=1 in env
      // The command depends on whether claude is installed in the test environment
      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      expect(call[2]).toEqual(
        expect.objectContaining({
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should not detect root user when process.getuid() returns non-zero", async () => {
      // Mock process.getuid to return 1000 (non-root user)
      process.getuid = () => 1000;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even for non-root users
      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      expect(call[2]).toEqual(
        expect.objectContaining({
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should handle environments where process.getuid() is not available", async () => {
      // Mock process without getuid (e.g., Windows)
      delete (process as unknown as { getuid?: () => number }).getuid;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even when getuid is unavailable
      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      expect(call[2]).toEqual(
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
    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should set IS_SANDBOX=1 when both root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      mockExeca.mockReturnValueOnce(createChildProcess());

      await launchClaudeCode("/test/path", { skipPermissions: true });

      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      expect(call[1] as string[]).toEqual(
        expect.arrayContaining(["--dangerously-skip-permissions"]),
      );
      const options = call[2] as Record<string, unknown>;
      expect(options.stdout).toBe("inherit");
      expect(options.stderr).toBe("inherit");
      expect(
        (options.env as Record<string, string> | undefined)?.IS_SANDBOX,
      ).toBe("1");
    });
  });

  describe("Continue mode without saved session", () => {
    // Skipped: Bun does not support vi.mock for execa module
    it.skip("falls back to new session when no sessionId is provided", async () => {
      mockExeca.mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });

      await launchClaudeCode("/test/path", { mode: "continue" });

      // First call is the actual launch
      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      const args = call[1] as string[];
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

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("adds --chrome on supported platforms", async () => {
      delete process.env.WSL_DISTRO_NAME;
      delete process.env.WSL_INTEROP;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { chrome: true });

      const call = mockExeca.mock.calls[0];
      const args = call[1] as string[];
      expect(args).toContain("--chrome");
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Chrome integration enabled"),
      );
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("adds --chrome on Windows platform", async () => {
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

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { chrome: true });

      const bunxCall = mockExeca.mock.calls.find((call) => call[0] === "bunx");
      expect(bunxCall).toBeTruthy();
      const args = (bunxCall?.[1] ?? []) as string[];
      expect(args).toContain("--chrome");
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("adds --chrome on macOS platform", async () => {
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

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { chrome: true });

      const call = mockExeca.mock.calls[0];
      const args = call[1] as string[];
      expect(args).toContain("--chrome");
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("skips --chrome on WSL environments", async () => {
      process.env.WSL_DISTRO_NAME = "Ubuntu";
      delete process.env.WSL_INTEROP;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { chrome: true });

      const call = mockExeca.mock.calls[0];
      const args = call[1] as string[];
      expect(args).not.toContain("--chrome");
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Chrome integration is not supported"),
      );
    });
  });

  describe("T106: IS_SANDBOX=1 not set when skipPermissions=false", () => {
    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should not set IS_SANDBOX=1 when skipPermissions=false even if root", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Temporarily remove IS_SANDBOX from process.env if present
      const originalIsSandbox = process.env.IS_SANDBOX;
      delete process.env.IS_SANDBOX;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", { skipPermissions: false });

      const call = mockExeca.mock.calls[0];
      const options = call[2] as Record<string, unknown>;
      expect(options.stdout).toBe("inherit");
      expect(options.stderr).toBe("inherit");
      expect(
        options.env && (options.env as Record<string, string>).IS_SANDBOX,
      ).toBeUndefined();

      // Verify --dangerously-skip-permissions is NOT in args
      expect(call[1] as string[]).not.toContain(
        "--dangerously-skip-permissions",
      );

      // Restore IS_SANDBOX
      if (originalIsSandbox !== undefined) {
        process.env.IS_SANDBOX = originalIsSandbox;
      }
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should not set IS_SANDBOX=1 when skipPermissions is undefined", async () => {
      // Mock root user
      process.getuid = () => 0;

      // Temporarily remove IS_SANDBOX from process.env if present
      const originalIsSandbox = process.env.IS_SANDBOX;
      delete process.env.IS_SANDBOX;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path", {});

      const call = mockExeca.mock.calls[0];
      const options = call[2] as Record<string, unknown>;
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
    // Skipped: Bun does not support vi.mock for execa module
    it.skip("T204: should display warning message when root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      // findCommand is mocked to return bunx source
      mockExeca.mockResolvedValue({
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

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("T205: should not display sandbox warning when non-root user", async () => {
      // Mock non-root user
      process.getuid = () => 1000;

      // findCommand is mocked to return bunx source
      mockExeca.mockResolvedValue({
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

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should not display any warning when skipPermissions=false", async () => {
      // Mock root user
      process.getuid = () => 0;

      // findCommand is mocked to return bunx source
      mockExeca.mockResolvedValue({
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
    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should pass fallback file descriptors when usingFallback is true", async () => {
      mockTerminalStreams.usingFallback = true;
      mockChildStdio.stdin = 101;
      mockChildStdio.stdout = 102;
      mockChildStdio.stderr = 103;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchClaudeCode("/test/path");

      // Verify file descriptors are passed correctly
      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      expect(call[2]).toEqual(
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
    // These tests verify the auto-detection behavior works correctly
    // with whatever command is detected in the environment

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should pass arguments correctly with detected command", async () => {
      mockExeca.mockImplementation(() => createChildProcess());

      await launchClaudeCode("/test/path", {
        mode: "resume",
        extraArgs: ["--debug"],
      });

      // Verify arguments are passed correctly to the detected command
      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      const args = call[1] as string[];
      expect(args).toContain("-r"); // resume mode
      expect(args).toContain("--debug"); // extra args
      expect(call[2]).toEqual(
        expect.objectContaining({
          stdout: "inherit",
          stderr: "inherit",
        }),
      );
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should pass skipPermissions flag correctly", async () => {
      process.getuid = () => 0; // root user
      mockExeca.mockImplementation(() => createChildProcess());

      await launchClaudeCode("/test/path", {
        skipPermissions: true,
        extraArgs: ["--verbose"],
      });

      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);
      const args = call[1] as string[];
      expect(args).toContain("--dangerously-skip-permissions");
      expect(args).toContain("--verbose");
      expect(call[2]).toEqual(
        expect.objectContaining({
          cwd: "/test/path",
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should use correct command based on environment detection", async () => {
      mockExeca.mockImplementation(() => createChildProcess());

      await launchClaudeCode("/test/path");

      // The command should match what was detected
      const call = mockExeca.mock.calls[0];
      expect(call[0]).toBe(detectedClaudeCommand);

      // Log message should indicate which path was used
      if (claudeIsInstalled) {
        expect(consoleLogSpy).toHaveBeenCalledWith(
          expect.stringContaining("Using locally installed claude command"),
        );
      } else {
        expect(consoleLogSpy).toHaveBeenCalledWith(
          expect.stringContaining("Falling back to bunx"),
        );
      }
    });
  });

  describe("Windows fallback", () => {
    // This test requires mocking findCommand which is not supported in Bun
    // The Windows fallback logic is tested implicitly through command.test.ts
    it.skip("uses bunx when claude is missing", async () => {
      // Skipped: Cannot mock findCommand in Bun environment
      // Functionality is covered by command.test.ts
    });
  });

  describe("FR-008: Launch arguments display", () => {
    // Skipped: Bun does not support vi.mock for execa module
    it.skip("should display launch arguments in console log", async () => {
      mockExeca.mockImplementation(() => createChildProcess());

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
