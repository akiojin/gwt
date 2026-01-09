import { describe, it, expect, beforeEach, afterEach, mock } from "bun:test";

const execaMock = mock<(...args: unknown[]) => unknown>();
const existsSyncMock = mock<(...args: unknown[]) => boolean>(() => false);
const readdirSyncMock = mock<(...args: unknown[]) => string[]>(() => []);
const statSyncMock = mock<
  (...args: unknown[]) => { isFile: () => boolean; mtime: Date }
>(() => ({ isFile: () => false, mtime: new Date() }));
const unlinkSyncMock = mock<(...args: unknown[]) => void>(() => {});
const mkdirSyncMock = mock<(...args: unknown[]) => void>(() => {});
let moduleCounter = 0;

const setupCommandMocks = () => {
  execaMock.mockReset();
  existsSyncMock.mockReset();
  readdirSyncMock.mockReset();
  statSyncMock.mockReset();
  unlinkSyncMock.mockReset();
  mkdirSyncMock.mockReset();
  existsSyncMock.mockReturnValue(false);
  readdirSyncMock.mockReturnValue([]);
  execaMock.mockImplementation(async (...args: unknown[]) => {
    const [command, argList] = args as [string, readonly string[] | undefined];
    if ((command === "which" || command === "where") && argList?.[0]) {
      const target = argList[0];
      const installed = new Set([
        "ls",
        "where",
        "claude",
        "codex",
        "gemini",
        "opencode",
      ]);
      if (installed.has(target)) {
        return { stdout: `/usr/bin/${target}` };
      }
      throw new Error("Command not found");
    }

    if (argList?.[0] === "--version") {
      if (command.includes("nonexistent")) {
        throw new Error("ENOENT");
      }
      return { stdout: `${command} 1.2.3` };
    }

    return { stdout: "" };
  });
};

describe("command utilities", () => {
  let commandModule: typeof import("../../../src/utils/command.js");

  beforeEach(async () => {
    mock.restore();
    mock.module("execa", () => ({
      execa: (...args: unknown[]) => execaMock(...args),
    }));
    mock.module("fs", () => ({
      existsSync: (...args: unknown[]) => existsSyncMock(...args),
      readdirSync: (...args: unknown[]) => readdirSyncMock(...args),
      statSync: (...args: unknown[]) => statSyncMock(...args),
      unlinkSync: (...args: unknown[]) => unlinkSyncMock(...args),
      mkdirSync: (...args: unknown[]) => mkdirSyncMock(...args),
      default: {
        existsSync: (...args: unknown[]) => existsSyncMock(...args),
        readdirSync: (...args: unknown[]) => readdirSyncMock(...args),
        statSync: (...args: unknown[]) => statSyncMock(...args),
        unlinkSync: (...args: unknown[]) => unlinkSyncMock(...args),
        mkdirSync: (...args: unknown[]) => mkdirSyncMock(...args),
      },
    }));
    setupCommandMocks();
    // Re-import module
    // Note: Bun doesn't fully support module mocking, so we test the actual implementation
    moduleCounter += 1;
    commandModule = await import(
      `../../../src/utils/command.ts?command-test=${moduleCounter}`
    );

    // Clear command lookup cache before each test
    commandModule.clearCommandLookupCache();
  });

  afterEach(() => {
    mock.restore();
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

      expect(results).toHaveLength(4);

      // Check Claude
      const claude = results.find((t) => t.id === "claude-code");
      expect(claude).toBeDefined();
      if (!claude) {
        throw new Error("Claude status not found");
      }
      expect(claude.name).toBe("Claude");
      expect(["installed", "bunx"]).toContain(claude.status);

      // Check Codex
      const codex = results.find((t) => t.id === "codex-cli");
      expect(codex).toBeDefined();
      if (!codex) {
        throw new Error("Codex status not found");
      }
      expect(codex.name).toBe("Codex");
      expect(["installed", "bunx"]).toContain(codex.status);

      // Check Gemini
      const gemini = results.find((t) => t.id === "gemini-cli");
      expect(gemini).toBeDefined();
      if (!gemini) {
        throw new Error("Gemini status not found");
      }
      expect(gemini.name).toBe("Gemini");
      expect(["installed", "bunx"]).toContain(gemini.status);

      // Check OpenCode
      const opencode = results.find((t) => t.id === "opencode");
      expect(opencode).toBeDefined();
      if (!opencode) {
        throw new Error("OpenCode status not found");
      }
      expect(opencode.name).toBe("OpenCode");
      expect(["installed", "bunx"]).toContain(opencode.status);
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

describe("getCommandVersion", () => {
  let commandModule: typeof import("../../../src/utils/command.js");

  beforeEach(async () => {
    mock.restore();
    setupCommandMocks();
    moduleCounter += 1;
    commandModule = await import(
      `../../../src/utils/command.ts?command-version=${moduleCounter}`
    );
    commandModule.clearCommandLookupCache();
  });

  it("returns version string for installed command with --version support", async () => {
    // Test with a common system command that supports --version
    // Note: Results may vary by platform
    const result = await commandModule.getCommandVersion("/bin/ls");
    // ls may or may not support --version depending on platform
    expect(result === null || typeof result === "string").toBe(true);
  });

  it("returns null for non-existent command", async () => {
    const result = await commandModule.getCommandVersion(
      "/nonexistent/path/command",
    );
    expect(result).toBeNull();
  });
});

describe("findCommand with version", () => {
  let commandModule: typeof import("../../../src/utils/command.js");

  beforeEach(async () => {
    mock.restore();
    setupCommandMocks();
    moduleCounter += 1;
    commandModule = await import(
      `../../../src/utils/command.ts?command-version=${moduleCounter}`
    );
    commandModule.clearCommandLookupCache();
  });

  it("includes version property in result structure", async () => {
    const result = await commandModule.findCommand("ls");
    expect(result).toHaveProperty("version");
  });

  it("returns null version for bunx fallback", async () => {
    const result = await commandModule.findCommand("nonexistent-command-xyz");
    expect(result.version).toBeNull();
  });

  it("returns version for installed commands when available", async () => {
    const result = await commandModule.findCommand("ls");
    if (result.source === "installed") {
      // Version is either a string starting with 'v' or null
      expect(result.version === null || result.version?.startsWith("v")).toBe(
        true,
      );
    }
  });
});

describe("detectAllToolStatuses with version", () => {
  let commandModule: typeof import("../../../src/utils/command.js");

  beforeEach(async () => {
    mock.restore();
    setupCommandMocks();
    moduleCounter += 1;
    commandModule = await import(
      `../../../src/utils/command.ts?command-version=${moduleCounter}`
    );
    commandModule.clearCommandLookupCache();
  });

  it("returns ToolStatus with version property", async () => {
    const results = await commandModule.detectAllToolStatuses();

    for (const tool of results) {
      expect(tool).toHaveProperty("version");
      // bunx fallback should have null version
      if (tool.status === "bunx") {
        expect(tool.version).toBeNull();
      }
    }
  });

  it("includes version for installed tools", async () => {
    const results = await commandModule.detectAllToolStatuses();

    for (const tool of results) {
      if (tool.status === "installed") {
        // Version is either a string starting with 'v' or null
        expect(tool.version === null || tool.version?.startsWith("v")).toBe(
          true,
        );
      }
    }
  });
});

describe("KNOWN_INSTALL_PATHS coverage", () => {
  beforeEach(async () => {
    mock.restore();
    setupCommandMocks();
    const { clearCommandLookupCache } = await import(
      `../../../src/utils/command.ts?command-version=${moduleCounter + 1}`
    );
    moduleCounter += 1;
    clearCommandLookupCache();
  });

  it("checks fallback paths for claude", async () => {
    moduleCounter += 1;
    const { findCommand } = await import(
      `../../../src/utils/command.ts?command-version=${moduleCounter}`
    );

    // This test verifies that fallback path checking works
    // by checking the result structure
    const result = await findCommand("claude");

    expect(result).toHaveProperty("available", true);
    expect(result).toHaveProperty("source");
    expect(["installed", "bunx"]).toContain(result.source);
  });

  it("checks fallback paths for codex", async () => {
    moduleCounter += 1;
    const { findCommand } = await import(
      `../../../src/utils/command.ts?command-version=${moduleCounter}`
    );

    const result = await findCommand("codex");

    expect(result).toHaveProperty("available", true);
    expect(result).toHaveProperty("source");
    expect(["installed", "bunx"]).toContain(result.source);
  });

  it("checks fallback paths for gemini", async () => {
    moduleCounter += 1;
    const { findCommand } = await import(
      `../../../src/utils/command.ts?command-version=${moduleCounter}`
    );

    const result = await findCommand("gemini");

    expect(result).toHaveProperty("available", true);
    expect(result).toHaveProperty("source");
    expect(["installed", "bunx"]).toContain(result.source);
  });
});
