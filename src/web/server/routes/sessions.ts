/**
 * Session Routes
 *
 * AI Toolセッション関連のREST APIエンドポイント。
 */

import type { FastifyInstance } from "fastify";
import type { PTYManager } from "../pty/manager.js";
import type {
  ApiResponse,
  AIToolSession,
  StartSessionRequest,
} from "../../../types/api.js";
import { saveSession } from "../../../config/index.js";
import { execa } from "execa";

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
      } catch (error) {
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
      const { toolType, toolName, mode, worktreePath } = request.body;

      const { session } = ptyManager.spawn(
        toolType,
        worktreePath,
        mode,
        toolName,
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
            toolType === "custom" ? (toolName ?? "custom") : toolType,
          toolLabel:
            toolType === "custom"
              ? (toolName ?? "Custom")
              : toolLabelFromType(toolType),
          mode,
          timestamp: Date.now(),
          repositoryRoot: repoRoot.trim(),
        });
      } catch {
        // ignore persistence errors
      }

      reply.code(201);
      return { success: true, data: session };
    } catch (error) {
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
    } catch (error) {
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
    } catch (error) {
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

function toolLabelFromType(toolType: "claude-code" | "codex-cli" | "custom") {
  if (toolType === "claude-code") return "Claude";
  if (toolType === "codex-cli") return "Codex";
  return "Custom";
}
