/**
 * Session Routes
 *
 * コーディングエージェントセッション関連のREST APIエンドポイント。
 */

import type { PTYManager } from "../pty/manager.js";
import { CodingAgentResolutionError } from "../../../services/codingAgentResolver.js";
import type {
  ApiResponse,
  CodingAgentSession,
  StartSessionRequest,
} from "../../../types/api.js";
import { saveSession } from "../../../config/index.js";
import { execa } from "execa";
import type { WebFastifyInstance } from "../types.js";
import { createLogger } from "../../../logging/logger.js";

const logger = createLogger({ category: "sessions" });

/**
 * セッション関連のルートを登録
 */
export async function registerSessionRoutes(
  fastify: WebFastifyInstance,
  ptyManager: PTYManager,
): Promise<void> {
  // GET /api/sessions - すべてのセッション一覧を取得
  fastify.get<{ Reply: ApiResponse<CodingAgentSession[]> }>(
    "/api/sessions",
    async (request, reply) => {
      try {
        const sessions = ptyManager.list();
        logger.debug({ count: sessions.length }, "Sessions listed");
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
    Reply: ApiResponse<CodingAgentSession>;
  }>("/api/sessions", async (request, reply) => {
    try {
      const {
        agentType,
        agentName,
        mode,
        worktreePath,
        skipPermissions,
        bypassApprovals,
        extraArgs,
        customAgentId,
      } = request.body;

      logger.debug(
        { agentType, mode, worktreePath },
        "Session start requested",
      );

      const spawnOptions: {
        toolName?: string | null;
        skipPermissions?: boolean;
        bypassApprovals?: boolean;
        extraArgs?: string[];
        customToolId?: string | null;
      } = {};

      if (typeof agentName !== "undefined") {
        spawnOptions.toolName = agentName;
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
      if (typeof customAgentId !== "undefined") {
        spawnOptions.customToolId = customAgentId;
      }

      if (agentType === "custom" && !agentName && !customAgentId) {
        reply.code(400);
        return {
          success: false,
          error: "Custom coding agent requires agentName or customAgentId",
          details: null,
        };
      }

      const { session } = await ptyManager.spawn(
        agentType,
        worktreePath,
        mode,
        spawnOptions,
      );

      // 履歴を永続化（best-effort）
      try {
        const { stdout: repoRoot } = await execa(
          "git",
          ["rev-parse", "--show-toplevel"],
          {
            cwd: worktreePath,
          },
        );
        let branchName: string | null = null;
        try {
          const { stdout: branchStdout } = await execa(
            "git",
            ["rev-parse", "--abbrev-ref", "HEAD"],
            { cwd: worktreePath },
          );
          branchName = branchStdout.trim() || null;
        } catch {
          branchName = null;
        }

        await saveSession({
          lastWorktreePath: worktreePath,
          lastBranch: branchName,
          lastUsedTool:
            agentType === "custom" ? (agentName ?? "custom") : agentType,
          toolLabel:
            agentType === "custom"
              ? (agentName ?? "Custom")
              : agentLabelFromType(agentType),
          mode,
          timestamp: Date.now(),
          repositoryRoot: repoRoot.trim(),
        });
      } catch {
        // ignore persistence errors
      }

      logger.info(
        { sessionId: session.sessionId, agentType },
        "Session created",
      );
      reply.code(201);
      return { success: true, data: session };
    } catch (error: unknown) {
      if (error instanceof CodingAgentResolutionError) {
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
    Reply: ApiResponse<CodingAgentSession>;
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
        logger.warn(
          { sessionId, reason: "not found" },
          "Session delete failed",
        );
        reply.code(404);
        return {
          success: false,
          error: "Session not found",
          details: `Session ${sessionId} does not exist`,
        };
      }

      logger.info({ sessionId }, "Session deleted via API");
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

function agentLabelFromType(agentType: "claude-code" | "codex-cli" | "custom") {
  if (agentType === "claude-code") return "Claude";
  if (agentType === "codex-cli") return "Codex";
  return "Custom";
}
