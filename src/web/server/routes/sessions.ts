/**
 * Session Routes
 *
 * AI Toolセッション関連のREST APIエンドポイント。
 */

import type { FastifyInstance } from "fastify";
import type { PTYManager } from "../pty/manager.js";
import { AIToolResolutionError } from "../../../services/aiToolResolver.js";
import type {
  ApiResponse,
  AIToolSession,
  StartSessionRequest,
} from "../../../types/api.js";

/**
 * セッション関連のルートを登録
 */
export async function registerSessionRoutes(
  fastify: FastifyInstance,
  ptyManager: PTYManager,
): Promise<void> {
  // GET /api/sessions - すべてのセッション一覧を取得
  fastify.get<{ Reply: ApiResponse<AIToolSession[]> }>(
    "/api/sessions",
    async (request, reply) => {
      try {
        const sessions = ptyManager.list();
        return { success: true, data: sessions };
      } catch (error: unknown) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        reply.code(500);
        return {
          success: false,
          error: "Failed to fetch sessions",
          details: errorMsg,
        };
      }
    },
  );

  // POST /api/sessions - 新しいセッションを開始
  fastify.post<{
    Body: StartSessionRequest;
    Reply: ApiResponse<AIToolSession>;
  }>("/api/sessions", async (request, reply) => {
    try {
      const {
        toolType,
        toolName,
        mode,
        worktreePath,
        skipPermissions,
        bypassApprovals,
        extraArgs,
        customToolId,
      } = request.body;

      const spawnOptions: {
        toolName?: string | null;
        skipPermissions?: boolean;
        bypassApprovals?: boolean;
        extraArgs?: string[];
        customToolId?: string | null;
      } = {};

      if (typeof toolName !== "undefined") {
        spawnOptions.toolName = toolName;
      }
      if (typeof skipPermissions !== "undefined") {
        spawnOptions.skipPermissions = skipPermissions;
      }
      if (typeof bypassApprovals !== "undefined") {
        spawnOptions.bypassApprovals = bypassApprovals;
      }
      if (Array.isArray(extraArgs) && extraArgs.length > 0) {
        spawnOptions.extraArgs = extraArgs;
      }
      if (typeof customToolId !== "undefined") {
        spawnOptions.customToolId = customToolId;
      }

      if (toolType === "custom" && !toolName && !customToolId) {
        reply.code(400);
        return {
          success: false,
          error: "Custom tool requires toolName or customToolId",
          details: null,
        };
      }

      const { session } = await ptyManager.spawn(
        toolType,
        worktreePath,
        mode,
        spawnOptions,
      );

      reply.code(201);
      return { success: true, data: session };
    } catch (error: unknown) {
      if (error instanceof AIToolResolutionError) {
        reply.code(400);
        return {
          success: false,
          error: error.message,
          details: error.hints?.join("\n") ?? null,
        };
      }

      const errorMsg = error instanceof Error ? error.message : String(error);
      reply.code(500);
      return {
        success: false,
        error: "Failed to start session",
        details: errorMsg,
      };
    }
  });

  // GET /api/sessions/:sessionId - 特定のセッション情報を取得
  fastify.get<{
    Params: { sessionId: string };
    Reply: ApiResponse<AIToolSession>;
  }>("/api/sessions/:sessionId", async (request, reply) => {
    try {
      const { sessionId } = request.params;

      const instance = ptyManager.get(sessionId);
      if (!instance) {
        reply.code(404);
        return {
          success: false,
          error: "Session not found",
          details: `Session ${sessionId} does not exist`,
        };
      }

      return { success: true, data: instance.session };
    } catch (error: unknown) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      reply.code(500);
      return {
        success: false,
        error: "Failed to fetch session",
        details: errorMsg,
      };
    }
  });

  // DELETE /api/sessions/:sessionId - セッションを終了
  fastify.delete<{
    Params: { sessionId: string };
    Reply:
      | { success: true }
      | { success: false; error: string; details?: string | null };
  }>("/api/sessions/:sessionId", async (request, reply) => {
    try {
      const { sessionId } = request.params;

      const deleted = ptyManager.delete(sessionId);
      if (!deleted) {
        reply.code(404);
        return {
          success: false,
          error: "Session not found",
          details: `Session ${sessionId} does not exist`,
        };
      }

      return { success: true };
    } catch (error: unknown) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      reply.code(500);
      return {
        success: false,
        error: "Failed to delete session",
        details: errorMsg,
      };
    }
  });
}
