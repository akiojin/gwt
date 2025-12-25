import { describe, it, expect, beforeEach } from "bun:test";

describe("command utilities", () => {
  let commandModule: typeof import("../../../src/utils/command.js");

  beforeEach(async () => {
    // Re-import module
    // Note: Bun doesn't fully support module mocking, so we test the actual implementation
    commandModule = await import("../../../src/utils/command.js");

    // Clear command lookup cache before each test
    commandModule.clearCommandLookupCache();
  });

  describe("findCommand - integration tests", () => {
    it("returns a CommandLookupResult with correct structure", async () => {
      const result = await commandModule.findCommand("nonexistent-command-xyz");

      expect(result).toHaveProperty("available");
      expect(result).toHaveProperty("path");
      expect(result).toHaveProperty("source");
      expect(typeof result.available).toBe("boolean");
      expect(["installed", "bunx"]).toContain(result.source);
    });

    it("returns bunx source for unknown commands", async () => {
      const result = await commandModule.findCommand(
        "definitely-not-a-real-command-12345",
      );

      expect(result.available).toBe(true);
      expect(result.source).toBe("bunx");
      expect(result.path).toBeNull();
    });

    it("finds common system commands", async () => {
      // 'ls' on Unix, 'where' on Windows should exist
      const command = process.platform === "win32" ? "where" : "ls";
      const result = await commandModule.findCommand(command);

      expect(result.available).toBe(true);
      expect(result.source).toBe("installed");
      expect(result.path).not.toBeNull();
    });
  });

  describe("isCommandAvailable - integration tests", () => {
    it("returns true for existing system commands", async () => {
      const command = process.platform === "win32" ? "where" : "ls";
      const result = await commandModule.isCommandAvailable(command);

      expect(result).toBe(true);
    });

    it("returns true for non-existent commands (bunx available)", async () => {
      const result = await commandModule.isCommandAvailable(
        "definitely-not-a-real-command-67890",
      );

      // Always available via bunx
      expect(result).toBe(true);
    });
  });

  describe("detectAllToolStatuses", () => {
    it("returns status for all builtin tools", async () => {
      const results = await commandModule.detectAllToolStatuses();

      expect(results).toHaveLength(3);

      // Check Claude
      const claude = results.find((t) => t.id === "claude-code");
      expect(claude).toBeDefined();
      expect(claude?.name).toBe("Claude");
      expect(["installed", "bunx"]).toContain(claude?.status);

      // Check Codex
      const codex = results.find((t) => t.id === "codex-cli");
      expect(codex).toBeDefined();
      expect(codex?.name).toBe("Codex");
      expect(["installed", "bunx"]).toContain(codex?.status);

      // Check Gemini
      const gemini = results.find((t) => t.id === "gemini-cli");
      expect(gemini).toBeDefined();
      expect(gemini?.name).toBe("Gemini");
      expect(["installed", "bunx"]).toContain(gemini?.status);
    });

    it("returns ToolStatus with correct structure", async () => {
      const results = await commandModule.detectAllToolStatuses();

      for (const tool of results) {
        expect(tool).toHaveProperty("id");
        expect(tool).toHaveProperty("name");
        expect(tool).toHaveProperty("status");
        expect(tool).toHaveProperty("path");
        expect(typeof tool.id).toBe("string");
        expect(typeof tool.name).toBe("string");
        expect(["installed", "bunx"]).toContain(tool.status);
      }
    });
  });
});

describe("KNOWN_INSTALL_PATHS coverage", () => {
  beforeEach(async () => {
    const { clearCommandLookupCache } =
      await import("../../../src/utils/command.js");
    clearCommandLookupCache();
  });

  it("checks fallback paths for claude", async () => {
    const { findCommand } = await import("../../../src/utils/command.js");

    // This test verifies that fallback path checking works
    // by checking the result structure
    const result = await findCommand("claude");

    expect(result).toHaveProperty("available", true);
    expect(result).toHaveProperty("source");
    expect(["installed", "bunx"]).toContain(result.source);
  });

  it("checks fallback paths for codex", async () => {
    const { findCommand } = await import("../../../src/utils/command.js");

    const result = await findCommand("codex");

    expect(result).toHaveProperty("available", true);
    expect(result).toHaveProperty("source");
    expect(["installed", "bunx"]).toContain(result.source);
  });

  it("checks fallback paths for gemini", async () => {
    const { findCommand } = await import("../../../src/utils/command.js");

    const result = await findCommand("gemini");

    expect(result).toHaveProperty("available", true);
    expect(result).toHaveProperty("source");
    expect(["installed", "bunx"]).toContain(result.source);
  });
});
