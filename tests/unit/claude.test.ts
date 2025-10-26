import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { launchClaudeCode } from "../../src/claude.js";
import { existsSync } from "fs";

const { mockExeca } = vi.hoisted(() => ({
  mockExeca: vi.fn(),
}));

vi.mock("execa", () => ({
  execa: mockExeca,
  default: { execa: mockExeca },
}));

// Mock fs
vi.mock("fs", () => {
  const existsSync = vi.fn(() => true);
  return {
    existsSync,
    default: { existsSync },
  };
});

// Mock console.log to avoid test output clutter
const consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});

describe("launchClaudeCode - Root User Detection", () => {
  let originalGetuid: (() => number) | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy.mockClear();
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

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify execa was called with IS_SANDBOX=1 in env
      expect(mockExeca).toHaveBeenCalledWith(
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

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify execa was called without IS_SANDBOX=1
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          env: process.env,
        }),
      );
    });

    it("should handle environments where process.getuid() is not available", async () => {
      // Mock process without getuid (e.g., Windows)
      delete (process as any).getuid;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify execa was called without IS_SANDBOX=1 (fallback to non-root)
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          env: process.env,
        }),
      );
    });
  });

  describe("T105: IS_SANDBOX=1 set when skipPermissions=true and root", () => {
    it("should set IS_SANDBOX=1 when both root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify IS_SANDBOX=1 is set
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining([
          "@anthropic-ai/claude-code@latest",
          "--dangerously-skip-permissions",
        ]),
        expect.objectContaining({
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

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await launchClaudeCode("/test/path", { skipPermissions: false });

      // Verify IS_SANDBOX=1 is NOT set
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          env: process.env,
        }),
      );

      // Verify --dangerously-skip-permissions is NOT in args
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.not.arrayContaining(["--dangerously-skip-permissions"]),
        expect.anything(),
      );
    });

    it("should not set IS_SANDBOX=1 when skipPermissions is undefined", async () => {
      // Mock root user
      process.getuid = () => 0;

      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      } as any);

      await launchClaudeCode("/test/path", {});

      // Verify IS_SANDBOX=1 is NOT set
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          env: process.env,
        }),
      );
    });
  });

  describe("T203-T205: Warning message display", () => {
    it("T204: should display warning message when root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      mockExeca.mockResolvedValue({
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
          "⚠️  Docker/サンドボックス環境として実行中（IS_SANDBOX=1）",
        ),
      );
    });

    it("T205: should not display sandbox warning when non-root user", async () => {
      // Mock non-root user
      process.getuid = () => 1000;

      mockExeca.mockResolvedValue({
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
          "⚠️  Docker/サンドボックス環境として実行中（IS_SANDBOX=1）",
        ),
      );
    });

    it("should not display any warning when skipPermissions=false", async () => {
      // Mock root user
      process.getuid = () => 0;

      mockExeca.mockResolvedValue({
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
          "⚠️  Docker/サンドボックス環境として実行中（IS_SANDBOX=1）",
        ),
      );
    });
  });
});
