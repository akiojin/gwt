/**
 * Config Routes
 */

import type { FastifyInstance } from "fastify";
import { loadToolsConfig, saveToolsConfig } from "../../../config/tools.js";
import {
  loadEnvHistory,
  recordEnvHistory,
} from "../../../config/env-history.js";
import type {
  ApiResponse,
  ConfigPayload,
  CustomAITool as ApiCustomAITool,
  EnvironmentHistoryEntry,
  EnvironmentVariable,
} from "../../../types/api.js";
import type { CustomAITool as FileCustomAITool } from "../../../types/tools.js";
import { getImportedEnvKeys } from "../env/importer.js";

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

function toApiTool(
  tool: FileCustomAITool,
  history: EnvironmentHistoryEntry[],
  importedKeys: Set<string>,
): ApiCustomAITool {
  return {
    id: tool.id,
    displayName: tool.displayName,
    icon: tool.icon ?? null,
    command: tool.command,
    executionType: tool.type,
    defaultArgs: tool.defaultArgs ?? null,
    modeArgs: tool.modeArgs,
    permissionSkipArgs: tool.permissionSkipArgs ?? null,
    env: normalizeEnv(tool.env, importedKeys, history),
    description: null,
    createdAt: null,
    updatedAt: null,
  };
}

function toFileTool(tool: ApiCustomAITool): FileCustomAITool {
  const envRecord = envArrayToRecord(tool.env);
  const fileTool: FileCustomAITool = {
    id: tool.id,
    displayName: tool.displayName,
    type: tool.executionType,
    command: tool.command,
    modeArgs: tool.modeArgs,
  };

  if (tool.icon) {
    fileTool.icon = tool.icon;
  }
  if (tool.defaultArgs && tool.defaultArgs.length > 0) {
    fileTool.defaultArgs = tool.defaultArgs;
  }
  if (tool.permissionSkipArgs && tool.permissionSkipArgs.length > 0) {
    fileTool.permissionSkipArgs = tool.permissionSkipArgs;
  }
  if (Object.keys(envRecord).length > 0) {
    fileTool.env = envRecord;
  }

  return fileTool;
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
  fastify: FastifyInstance,
): Promise<void> {
  fastify.get<{ Reply: ApiResponse<ConfigPayload> }>(
    "/api/config",
    async (request, reply) => {
      try {
        const config = await loadToolsConfig();
        const history = await loadEnvHistory();
        const importedSet = new Set(getImportedEnvKeys());

        return {
          success: true,
          data: {
            version: config.version,
            updatedAt: config.updatedAt ?? null,
            env: normalizeEnv(config.env, importedSet, history),
            history,
            tools: config.customTools.map((tool) =>
              toApiTool(tool, history, importedSet),
            ),
          },
        } satisfies ApiResponse<ConfigPayload>;
      } catch (error) {
        request.log.error({ err: error }, "Failed to load custom tool config");
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
      const existing = await loadToolsConfig();
      const nextEnvRecord = envArrayToRecord(payload.env);
      const envHistory = diffEnvHistory(
        existing.env ?? {},
        nextEnvRecord,
        "ui",
      );

      await saveToolsConfig({
        version: payload.version || existing.version,
        env: nextEnvRecord,
        customTools: payload.tools.map(toFileTool),
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
          tools: payload.tools,
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
