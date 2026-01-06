/**
 * Config Routes
 */

import {
  loadCodingAgentsConfig,
  saveCodingAgentsConfig,
} from "../../../config/tools.js";
import {
  loadEnvHistory,
  recordEnvHistory,
} from "../../../config/env-history.js";
import type {
  ApiResponse,
  ConfigPayload,
  ApiCodingAgent,
  EnvironmentHistoryEntry,
  EnvironmentVariable,
} from "../../../types/api.js";
import type { CodingAgent } from "../../../types/tools.js";
import { getImportedEnvKeys } from "../env/importer.js";
import type { WebFastifyInstance } from "../types.js";

function normalizeEnv(
  env: Record<string, string> | undefined,
  importedKeys: Set<string>,
  history: EnvironmentHistoryEntry[],
): EnvironmentVariable[] {
  if (!env) {
    return [];
  }

  const lastUpdated = new Map<string, string | null>();
  for (const entry of history) {
    lastUpdated.set(entry.key, entry.timestamp ?? null);
  }

  return Object.entries(env).map(([key, value]) => {
    const variable: EnvironmentVariable = {
      key,
      value,
      lastUpdated: lastUpdated.get(key) ?? null,
    };

    if (importedKeys.has(key)) {
      variable.importedFromOs = true;
    }

    return variable;
  });
}

function envArrayToRecord(
  env?: EnvironmentVariable[] | null,
): Record<string, string> {
  if (!env) {
    return {};
  }

  const record: Record<string, string> = {};
  for (const variable of env) {
    if (!variable.key) continue;
    record[variable.key] = variable.value;
  }
  return record;
}

function toApiCodingAgent(
  agent: CodingAgent,
  history: EnvironmentHistoryEntry[],
  importedKeys: Set<string>,
): ApiCodingAgent {
  return {
    id: agent.id,
    displayName: agent.displayName,
    icon: agent.icon ?? null,
    command: agent.command,
    executionType: agent.type,
    defaultArgs: agent.defaultArgs ?? null,
    modeArgs: agent.modeArgs,
    permissionSkipArgs: agent.permissionSkipArgs ?? null,
    env: normalizeEnv(agent.env, importedKeys, history),
    description: null,
    createdAt: null,
    updatedAt: null,
  };
}

function toFileCodingAgent(agent: ApiCodingAgent): CodingAgent {
  const envRecord = envArrayToRecord(agent.env);
  const fileAgent: CodingAgent = {
    id: agent.id,
    displayName: agent.displayName,
    type: agent.executionType,
    command: agent.command,
    modeArgs: agent.modeArgs,
  };

  if (agent.icon) {
    fileAgent.icon = agent.icon;
  }
  if (agent.defaultArgs && agent.defaultArgs.length > 0) {
    fileAgent.defaultArgs = agent.defaultArgs;
  }
  if (agent.permissionSkipArgs && agent.permissionSkipArgs.length > 0) {
    fileAgent.permissionSkipArgs = agent.permissionSkipArgs;
  }
  if (Object.keys(envRecord).length > 0) {
    fileAgent.env = envRecord;
  }

  return fileAgent;
}

function diffEnvHistory(
  previous: Record<string, string>,
  next: Record<string, string>,
  source: EnvironmentHistoryEntry["source"],
): EnvironmentHistoryEntry[] {
  const entries: EnvironmentHistoryEntry[] = [];
  const timestamp = new Date().toISOString();

  for (const [key, value] of Object.entries(next)) {
    if (!(key in previous)) {
      entries.push({ key, action: "add", source, timestamp });
    } else if (previous[key] !== value) {
      entries.push({ key, action: "update", source, timestamp });
    }
  }

  for (const key of Object.keys(previous)) {
    if (!(key in next)) {
      entries.push({ key, action: "delete", source, timestamp });
    }
  }

  return entries;
}

export async function registerConfigRoutes(
  fastify: WebFastifyInstance,
): Promise<void> {
  fastify.get<{ Reply: ApiResponse<ConfigPayload> }>(
    "/api/config",
    async (request, reply) => {
      try {
        const config = await loadCodingAgentsConfig();
        const history = await loadEnvHistory();
        const importedSet = new Set(getImportedEnvKeys());

        return {
          success: true,
          data: {
            version: config.version,
            updatedAt: config.updatedAt ?? null,
            env: normalizeEnv(config.env, importedSet, history),
            history,
            codingAgents: config.customCodingAgents.map((agent) =>
              toApiCodingAgent(agent, history, importedSet),
            ),
          },
        } satisfies ApiResponse<ConfigPayload>;
      } catch (error) {
        request.log.error(
          { err: error },
          "Failed to load coding agents config",
        );
        reply.code(500);
        return {
          success: false,
          error: "Failed to load config",
          details: error instanceof Error ? error.message : String(error),
        };
      }
    },
  );

  fastify.put<{
    Body: ConfigPayload;
    Reply: ApiResponse<ConfigPayload>;
  }>("/api/config", async (request, reply) => {
    try {
      const payload = request.body;
      const existing = await loadCodingAgentsConfig();
      const nextEnvRecord = envArrayToRecord(payload.env);
      const envHistory = diffEnvHistory(
        existing.env ?? {},
        nextEnvRecord,
        "ui",
      );

      await saveCodingAgentsConfig({
        version: payload.version || existing.version,
        env: nextEnvRecord,
        customCodingAgents: payload.codingAgents.map(toFileCodingAgent),
      });

      if (envHistory.length) {
        await recordEnvHistory(envHistory);
      }

      const history = await loadEnvHistory();
      const importedSet = new Set(getImportedEnvKeys());

      return {
        success: true,
        data: {
          version: payload.version || existing.version,
          updatedAt: new Date().toISOString(),
          env: normalizeEnv(nextEnvRecord, importedSet, history),
          history,
          codingAgents: payload.codingAgents,
        },
      } satisfies ApiResponse<ConfigPayload>;
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      request.log.error({ err: error }, "Failed to update config");
      reply.code(500);
      return {
        success: false,
        error: "Failed to update config",
        details: errorMsg,
      };
    }
  });
}
