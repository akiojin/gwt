import { describe, it, expect, beforeEach, afterEach,  mock } from "bun:test";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import path from "node:path";

let tempHome = "";
const originalHomeEnv = process.env.GWT_HOME;

describe("shared environment config", () => {
  let loadCodingAgentsConfig: typeof import("../../../src/config/tools.js").loadCodingAgentsConfig;
  let saveCodingAgentsConfig: typeof import("../../../src/config/tools.js").saveCodingAgentsConfig;

  beforeEach(async () => {
    const base = path.join(process.cwd(), ".tmp-tests");
    await mkdir(base, { recursive: true });
    tempHome = await mkdtemp(path.join(base, "gwt-tools-"));
    process.env.GWT_HOME = tempHome;
    await // resetModules not needed in bun;
    const module = await import("../../../src/config/tools.js");
    loadCodingAgentsConfig = module.loadCodingAgentsConfig;
    saveCodingAgentsConfig = module.saveCodingAgentsConfig;
  });

  afterEach(async () => {
    await rm(tempHome, { recursive: true, force: true });
    if (originalHomeEnv === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalHomeEnv;
    }
    await // resetModules not needed in bun;
  });

  it("loadCodingAgentsConfig returns shared env and updatedAt when present", async () => {
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
        customCodingAgents: [],
      }),
      "utf8",
    );

    const config = await loadCodingAgentsConfig();

    expect(config.env).toEqual({
      GITHUB_TOKEN: "ghp_test",
      HTTP_PROXY: "http://proxy:8080",
    });
    expect(config.updatedAt).toBe("2025-11-11T00:00:00Z");
  });

  it("saveCodingAgentsConfig persists shared env and sets updatedAt", async () => {
    const configDir = path.join(tempHome, ".gwt");
    await mkdir(configDir, { recursive: true });

    await saveCodingAgentsConfig({
      version: "1.2.3",
      env: { OPENAI_API_KEY: "sk-test" },
      customCodingAgents: [],
    });

    const raw = await readFile(path.join(configDir, "tools.json"), "utf8");
    const data = JSON.parse(raw);

    expect(data.version).toBe("1.2.3");
    expect(data.env).toEqual({ OPENAI_API_KEY: "sk-test" });
    expect(typeof data.updatedAt).toBe("string");
  });
});
