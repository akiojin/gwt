/**
 * Worktree Routes
 *
 * Worktree関連のREST APIエンドポイント。
 */

import type { FastifyInstance } from "fastify";
import {
  listWorktrees,
  getWorktreeByPath,
  createNewWorktree,
  removeWorktree,
} from "../services/worktrees.js";
import type {
  ApiResponse,
  Worktree,
  CreateWorktreeRequest,
} from "../../../types/api.js";

/**
 * Worktree関連のルートを登録
 */
export async function registerWorktreeRoutes(
  fastify: FastifyInstance,
): Promise<void> {
  // GET /api/worktrees - すべてのWorktree一覧を取得
  fastify.get<{ Reply: ApiResponse<Worktree[]> }>(
    "/api/worktrees",
    async (request, reply) => {
      try {
        const worktrees = await listWorktrees();
        return { success: true, data: worktrees };
      } catch (error) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        reply.code(500);
        return {
          success: false,
          error: "Failed to fetch worktrees",
          details: errorMsg,
        };
      }
    },
  );

  // POST /api/worktrees - 新しいWorktreeを作成
  fastify.post<{
    Body: CreateWorktreeRequest;
    Reply: ApiResponse<Worktree>;
  }>("/api/worktrees", async (request, reply) => {
    try {
      const { branchName, createBranch = false } = request.body;

      const worktree = await createNewWorktree(branchName, createBranch);
      reply.code(201);
      return { success: true, data: worktree };
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      reply.code(500);
      return {
        success: false,
        error: "Failed to create worktree",
        details: errorMsg,
      };
    }
  });

  // DELETE /api/worktrees/delete - Worktreeを削除
  fastify.delete<{
    Querystring: { path: string; force?: boolean };
    Reply:
      | { success: true }
      | { success: false; error: string; details?: string | null };
  }>("/api/worktrees/delete", async (request, reply) => {
    try {
      const { path } = request.query;

      if (!path) {
        reply.code(400);
        return {
          success: false,
          error: "Missing required parameter: path",
          details: null,
        };
      }

      const worktree = await getWorktreeByPath(path);
      if (!worktree) {
        reply.code(404);
        return {
          success: false,
          error: "Worktree not found",
          details: `Worktree at ${path} does not exist`,
        };
      }

      await removeWorktree(path);
      return { success: true };
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      reply.code(500);
      return {
        success: false,
        error: "Failed to delete worktree",
        details: errorMsg,
      };
    }
  });
}
