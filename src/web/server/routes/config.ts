/**
 * Config Routes
 *
 * カスタムAI Tool設定関連のREST APIエンドポイント。
 */

import type { FastifyInstance } from "fastify";
import type {
  ApiResponse,
  CustomAITool as ApiCustomAITool,
  UpdateConfigRequest,
} from "../../../types/api.js";
import { loadToolsConfig, saveToolsConfig } from "../../../config/tools.js";
import type { CustomAITool as ConfigCustomAITool } from "../../../types/tools.js";
import { sanitizeEnvRecord } from "../../../config/shared-env.js";

/**
 * 設定関連のルートを登録
 */
export async function registerConfigRoutes(
  fastify: FastifyInstance,
): Promise<void> {
  // GET /api/config - カスタムAI Tool設定を取得
  fastify.get<{ Reply: ApiResponse<{ tools: ApiCustomAITool[] }> }>(
    "/api/config",
    async (request, reply) => {
      try {
        const config = await loadToolsConfig();
        return {
          success: true,
          data: {
            tools: config.customTools.map(mapConfigToolToApi),
            env: config.env ?? {},
          },
        };
      } catch (error) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        reply.code(500);
        return {
          success: false,
          error: "Failed to load tools configuration",
          details: errorMsg,
        };
      }
    },
  );

  // PUT /api/config - カスタムAI Tool設定を更新
  fastify.put<{
    Body: UpdateConfigRequest;
    Reply: ApiResponse<{ tools: ApiCustomAITool[] }>;
  }>("/api/config", async (request, reply) => {
    try {
      const { tools, env } = request.body;
      const now = new Date().toISOString();
      const normalizedTools: ConfigCustomAITool[] = tools.map((tool) =>
        mapApiToolToConfig({
          ...tool,
          displayName: tool.displayName,
          createdAt: tool.createdAt ?? now,
          updatedAt: tool.updatedAt ?? now,
        }),
      );

      const existing = await loadToolsConfig();
      const nextEnv = sanitizeEnvRecord(env ?? existing.env ?? {});
      await saveToolsConfig({
        version: existing.version ?? "1.0.0",
        customTools: normalizedTools,
        env: nextEnv,
      });

      return {
        success: true,
        data: {
          tools: normalizedTools.map(mapConfigToolToApi),
          env: nextEnv,
        },
      };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const isValidationError =
        error instanceof Error &&
        /Required field|Invalid|Duplicate|modeArgs/i.test(error.message);
      reply.code(isValidationError ? 400 : 500);
      return {
        success: false,
        error: isValidationError
          ? "Invalid tools configuration"
          : "Failed to update config",
        details: message,
      };
    }
  });
}

function mapConfigToolToApi(tool: ConfigCustomAITool): ApiCustomAITool {
  return {
    id: tool.id,
    displayName: tool.displayName,
    icon: tool.icon ?? null,
    executionType: tool.type,
    command: tool.command,
    defaultArgs: tool.defaultArgs ?? null,
    modeArgs: {
      normal: tool.modeArgs?.normal ?? [],
      continue: tool.modeArgs?.continue ?? [],
      resume: tool.modeArgs?.resume ?? [],
    },
    permissionSkipArgs: tool.permissionSkipArgs ?? null,
    env: tool.env ?? null,
    description: tool.description ?? null,
    createdAt: tool.createdAt ?? new Date(0).toISOString(),
    updatedAt: tool.updatedAt ?? tool.createdAt ?? new Date(0).toISOString(),
  };
}

function mapApiToolToConfig(tool: ApiCustomAITool): ConfigCustomAITool {
  const modeArgs: ConfigCustomAITool["modeArgs"] = {};
  if (tool.modeArgs?.normal !== undefined) {
    modeArgs.normal = tool.modeArgs.normal;
  }
  if (tool.modeArgs?.continue !== undefined) {
    modeArgs.continue = tool.modeArgs.continue;
  }
  if (tool.modeArgs?.resume !== undefined) {
    modeArgs.resume = tool.modeArgs.resume;
  }

  const configTool: ConfigCustomAITool = {
    id: tool.id,
    displayName: tool.displayName,
    type: tool.executionType,
    command: tool.command,
    modeArgs,
    createdAt: tool.createdAt,
    updatedAt: tool.updatedAt,
  };

  if (tool.icon !== null && tool.icon !== undefined) {
    configTool.icon = tool.icon;
  }

  if (tool.description !== null && tool.description !== undefined) {
    configTool.description = tool.description;
  }

  if (tool.defaultArgs !== null && tool.defaultArgs !== undefined) {
    configTool.defaultArgs = tool.defaultArgs;
  }

  if (
    tool.permissionSkipArgs !== null &&
    tool.permissionSkipArgs !== undefined
  ) {
    configTool.permissionSkipArgs = tool.permissionSkipArgs;
  }

  if (tool.env !== null && tool.env !== undefined) {
    configTool.env = tool.env;
  }

  return configTool;
}
