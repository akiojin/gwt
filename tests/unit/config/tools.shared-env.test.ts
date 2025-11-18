import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import path from "node:path";

let tempHome = "";
const originalHomeEnv = process.env.CLAUDE_WORKTREE_HOME;

describe("shared environment config", () => {
  let loadToolsConfig: typeof import("../../../src/config/tools.js").loadToolsConfig;
  let saveToolsConfig: typeof import("../../../src/config/tools.js").saveToolsConfig;

  beforeEach(async () => {
    const base = path.join(process.cwd(), ".tmp-tests");
    await mkdir(base, { recursive: true });
    tempHome = await mkdtemp(path.join(base, "cw-tools-"));
    process.env.CLAUDE_WORKTREE_HOME = tempHome;
    await vi.resetModules();
    const module = await import("../../../src/config/tools.js");
    loadToolsConfig = module.loadToolsConfig;
    saveToolsConfig = module.saveToolsConfig;
  });

  afterEach(async () => {
    await rm(tempHome, { recursive: true, force: true });
    if (originalHomeEnv === undefined) {
      delete process.env.CLAUDE_WORKTREE_HOME;
    } else {
      process.env.CLAUDE_WORKTREE_HOME = originalHomeEnv;
    }
    await vi.resetModules();
  });

  it("loadToolsConfig returns shared env and updatedAt when present", async () => {
    const configDir = path.join(tempHome, ".gwt");
    await mkdir(configDir, { recursive: true });
    await writeFile(
      path.join(configDir, "tools.json"),
      JSON.stringify({
        version: "1.0.0",
        updatedAt: "2025-11-11T00:00:00Z",
        env: {
          GITHUB_TOKEN: "ghp_test",
          HTTP_PROXY: "http://proxy:8080",
        },
        customTools: [],
      }),
      "utf8",
    );

    const config = await loadToolsConfig();

    expect(config.env).toEqual({
      GITHUB_TOKEN: "ghp_test",
      HTTP_PROXY: "http://proxy:8080",
    });
    expect(config.updatedAt).toBe("2025-11-11T00:00:00Z");
  });

  it("saveToolsConfig persists shared env and sets updatedAt", async () => {
    const configDir = path.join(tempHome, ".gwt");
    await mkdir(configDir, { recursive: true });

    await saveToolsConfig({
      version: "1.2.3",
      env: { OPENAI_API_KEY: "sk-test" },
      customTools: [],
    });

    const raw = await readFile(path.join(configDir, "tools.json"), "utf8");
    const data = JSON.parse(raw);

    expect(data.version).toBe("1.2.3");
    expect(data.env).toEqual({ OPENAI_API_KEY: "sk-test" });
    expect(typeof data.updatedAt).toBe("string");
  });
});
