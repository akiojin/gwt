import Fastify from "fastify";
import type { FastifyInstance } from "fastify";
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import type { ToolsConfig } from "../../../../src/types/tools.js";
import { registerConfigRoutes } from "../../../../src/web/server/routes/config.js";

const mockLoadToolsConfig = vi.fn<[], Promise<ToolsConfig>>();
const mockSaveToolsConfig = vi.fn<[], Promise<void>>();
const mockLoadEnvHistory = vi.fn<[], Promise<any[]>>();
const mockRecordEnvHistory = vi.fn<[], Promise<void>>();
const mockGetImportedEnvKeys = vi.fn<[], string[]>(() => []);

vi.mock("../../../../src/config/tools.ts", () => ({
  loadToolsConfig: () => mockLoadToolsConfig(),
  saveToolsConfig: (...args: unknown[]) => mockSaveToolsConfig(...args),
}));

vi.mock("../../../../src/config/env-history.ts", () => ({
  loadEnvHistory: () => mockLoadEnvHistory(),
  recordEnvHistory: (...args: unknown[]) => mockRecordEnvHistory(...args),
}));

vi.mock("../../../../src/web/server/env/importer.ts", () => ({
  getImportedEnvKeys: () => mockGetImportedEnvKeys(),
}));

describe("config routes", () => {
  let fastify: FastifyInstance;

  beforeEach(() => {
    mockLoadToolsConfig.mockReset();
    mockSaveToolsConfig.mockReset();
    mockLoadEnvHistory.mockReset();
    mockRecordEnvHistory.mockReset();
    mockGetImportedEnvKeys.mockReset();
    fastify = Fastify();
  });

  afterEach(() => {
    vi.resetModules();
    return fastify.close();
  });

  it("GET /api/config returns shared env metadata", async () => {
    const config: ToolsConfig = {
      version: "1.0.0",
      updatedAt: "2025-11-11T00:00:00Z",
      env: {
        OPENAI_API_KEY: "sk-test",
        HTTP_PROXY: "http://proxy:8080",
      },
      customTools: [
        {
          id: "custom-tool-a",
          displayName: "Custom Tool A",
          type: "command",
          command: "aider",
          modeArgs: { normal: [] },
        },
      ],
    };

    mockLoadToolsConfig.mockResolvedValue(config);
    mockLoadEnvHistory.mockResolvedValue([
      {
        key: "OPENAI_API_KEY",
        action: "add",
        source: "ui",
        timestamp: "2025-11-10T00:00:00Z",
      },
    ]);
    mockGetImportedEnvKeys.mockReturnValue(["HTTP_PROXY"]);

    await registerConfigRoutes(fastify);

    const response = await fastify.inject({
      method: "GET",
      url: "/api/config",
    });
    expect(response.statusCode).toBe(200);
    const body = response.json();
    expect(body.data.env).toEqual([
      {
        key: "OPENAI_API_KEY",
        value: "sk-test",
        lastUpdated: "2025-11-10T00:00:00Z",
      },
      {
        key: "HTTP_PROXY",
        value: "http://proxy:8080",
        lastUpdated: null,
        importedFromOs: true,
      },
    ]);
    expect(body.data.history).toHaveLength(1);
  });

  it("PUT /api/config saves shared env and records history", async () => {
    mockLoadToolsConfig.mockResolvedValue({
      version: "1.0.0",
      env: { LEGACY: "old" },
      customTools: [],
    });
    mockLoadEnvHistory.mockResolvedValue([]);

    await registerConfigRoutes(fastify);

    const response = await fastify.inject({
      method: "PUT",
      url: "/api/config",
      payload: {
        version: "1.1.0",
        env: [{ key: "NEW_KEY", value: "new-value" }],
        tools: [
          {
            id: "custom-tool",
            displayName: "Custom",
            icon: null,
            command: "custom",
            executionType: "command",
            defaultArgs: null,
            modeArgs: { normal: [] },
            permissionSkipArgs: null,
            env: [{ key: "SCOPE", value: "test" }],
            description: null,
            createdAt: null,
            updatedAt: null,
          },
        ],
      },
    });

    expect(response.statusCode).toBe(200);
    expect(mockSaveToolsConfig).toHaveBeenCalledTimes(1);
    expect(mockSaveToolsConfig.mock.calls[0][0]).toMatchObject({
      env: { NEW_KEY: "new-value" },
      customTools: [
        expect.objectContaining({
          type: "command",
          env: { SCOPE: "test" },
        }),
      ],
    });
    expect(mockRecordEnvHistory).toHaveBeenCalled();
  });
});
