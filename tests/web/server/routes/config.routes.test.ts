import Fastify from "fastify";
import { describe, expect, it, beforeEach, afterEach, mock } from "bun:test";
import type { EnvironmentHistoryEntry } from "../../../../src/types/api.js";
import type { CodingAgentsConfig } from "../../../../src/types/tools.js";
import type { WebFastifyInstance } from "../../../../src/web/server/types.js";
import { registerConfigRoutes } from "../../../../src/web/server/routes/config.js";

const mockLoadCodingAgentsConfig = mock();
const mockSaveCodingAgentsConfig = mock();
const mockLoadEnvHistory = mock();
const mockRecordEnvHistory =
  mock<(entries: EnvironmentHistoryEntry[]) => Promise<void>>();
const mockGetImportedEnvKeys = mock<() => string[]>(() => []);

mock.module("../../../../src/config/tools.ts", () => ({
  loadCodingAgentsConfig: () => mockLoadCodingAgentsConfig(),
  saveCodingAgentsConfig: (config: CodingAgentsConfig) =>
    mockSaveCodingAgentsConfig(config),
}));

mock.module("../../../../src/config/env-history.ts", () => ({
  loadEnvHistory: () => mockLoadEnvHistory(),
  recordEnvHistory: (entries: EnvironmentHistoryEntry[]) =>
    mockRecordEnvHistory(entries),
}));

mock.module("../../../../src/web/server/env/importer.ts", () => ({
  getImportedEnvKeys: () => mockGetImportedEnvKeys(),
}));

describe("config routes", () => {
  let fastify: WebFastifyInstance;

  beforeEach(() => {
    mockLoadCodingAgentsConfig.mockReset();
    mockSaveCodingAgentsConfig.mockReset();
    mockLoadEnvHistory.mockReset();
    mockRecordEnvHistory.mockReset();
    mockGetImportedEnvKeys.mockReset();
    fastify = Fastify() as WebFastifyInstance;
  });

  afterEach(() => {
    // resetModules not needed in bun;
    return fastify.close();
  });

  it("GET /api/config returns shared env metadata", async () => {
    const config: CodingAgentsConfig = {
      version: "1.0.0",
      updatedAt: "2025-11-11T00:00:00Z",
      env: {
        OPENAI_API_KEY: "sk-test",
        HTTP_PROXY: "http://proxy:8080",
      },
      customCodingAgents: [
        {
          id: "custom-tool-a",
          displayName: "Custom Tool A",
          type: "command",
          command: "aider",
          modeArgs: { normal: [] },
        },
      ],
    };

    mockLoadCodingAgentsConfig.mockResolvedValue(config);
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
    mockLoadCodingAgentsConfig.mockResolvedValue({
      version: "1.0.0",
      env: { LEGACY: "old" },
      customCodingAgents: [],
    });
    mockLoadEnvHistory.mockResolvedValue([]);

    await registerConfigRoutes(fastify);

    const response = await fastify.inject({
      method: "PUT",
      url: "/api/config",
      payload: {
        version: "1.1.0",
        env: [{ key: "NEW_KEY", value: "new-value" }],
        codingAgents: [
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
    expect(mockSaveCodingAgentsConfig).toHaveBeenCalledTimes(1);
    const saveCall = mockSaveCodingAgentsConfig.mock.calls[0];
    if (!saveCall) {
      throw new Error("Expected save config call");
    }
    expect(saveCall[0]).toMatchObject({
      env: { NEW_KEY: "new-value" },
      customCodingAgents: [
        expect.objectContaining({
          type: "command",
          env: { SCOPE: "test" },
        }),
      ],
    });
    expect(mockRecordEnvHistory).toHaveBeenCalled();
  });
});
