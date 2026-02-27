import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

vi.mock("$lib/privacyMask", () => ({
  maskSensitiveData: (s: string) => s.replace(/sk-[A-Za-z0-9_-]+/g, "[REDACTED]"),
}));

describe("diagnostics", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("collectSystemInfo", () => {
    it("returns formatted system info on success", async () => {
      invokeMock.mockResolvedValue({
        osName: "macOS",
        osVersion: "14.0",
        arch: "aarch64",
        gwtVersion: "7.10.0",
      });

      const { collectSystemInfo } = await import("./diagnostics");
      const result = await collectSystemInfo();

      expect(result).toContain("- OS: macOS 14.0");
      expect(result).toContain("- Architecture: aarch64");
      expect(result).toContain("- gwt Version: 7.10.0");
      expect(invokeMock).toHaveBeenCalledWith("get_report_system_info");
    });

    it("returns fallback message on failure", async () => {
      invokeMock.mockRejectedValue(new Error("invoke failed"));

      const { collectSystemInfo } = await import("./diagnostics");
      const result = await collectSystemInfo();

      expect(result).toBe("(Failed to collect system info)");
    });

    it("includes all three info lines joined by newline", async () => {
      invokeMock.mockResolvedValue({
        osName: "Windows",
        osVersion: "11",
        arch: "x86_64",
        gwtVersion: "7.9.0",
      });

      const { collectSystemInfo } = await import("./diagnostics");
      const result = await collectSystemInfo();
      const lines = result.split("\n");

      expect(lines).toHaveLength(3);
      expect(lines[0]).toBe("- OS: Windows 11");
      expect(lines[1]).toBe("- Architecture: x86_64");
      expect(lines[2]).toBe("- gwt Version: 7.9.0");
    });
  });

  describe("collectRecentLogs", () => {
    it("returns masked logs on success", async () => {
      invokeMock.mockResolvedValue("Log line with sk-ant-api03-abc123");

      const { collectRecentLogs } = await import("./diagnostics");
      const result = await collectRecentLogs();

      expect(result).toContain("[REDACTED]");
      expect(result).not.toContain("sk-ant-api03-abc123");
      expect(invokeMock).toHaveBeenCalledWith("read_recent_logs", { maxLines: 50 });
    });

    it("passes custom maxLines to invoke", async () => {
      invokeMock.mockResolvedValue("some logs");

      const { collectRecentLogs } = await import("./diagnostics");
      await collectRecentLogs(100);

      expect(invokeMock).toHaveBeenCalledWith("read_recent_logs", { maxLines: 100 });
    });

    it("returns fallback message on failure", async () => {
      invokeMock.mockRejectedValue(new Error("read failed"));

      const { collectRecentLogs } = await import("./diagnostics");
      const result = await collectRecentLogs();

      expect(result).toBe("(Failed to collect logs)");
    });

    it("returns clean logs when no sensitive data present", async () => {
      invokeMock.mockResolvedValue("Normal log output without secrets");

      const { collectRecentLogs } = await import("./diagnostics");
      const result = await collectRecentLogs();

      expect(result).toBe("Normal log output without secrets");
    });
  });
});
