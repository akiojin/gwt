/**
 * Branch Routes
 *
 * ブランチ関連のREST APIエンドポイント。
 */

import type { FastifyInstance } from "fastify";
import {
  listBranches,
  getBranchByName,
  syncBranchState,
} from "../services/branches.js";
import type {
  ApiResponse,
  Branch,
  BranchSyncRequest,
  BranchSyncResult,
} from "../../../types/api.js";

/**
 * ブランチ関連のルートを登録
 */
export async function registerBranchRoutes(
  fastify: FastifyInstance,
): Promise<void> {
  // GET /api/branches - すべてのブランチ一覧を取得
  fastify.get<{ Reply: ApiResponse<Branch[]> }>(
    "/api/branches",
    async (request, reply) => {
      try {
        const branches = await listBranches();
        return { success: true, data: branches };
      } catch (error) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        reply.code(500);
        return {
          success: false,
          error: "Failed to fetch branches",
          details: errorMsg,
        };
      }
    },
  );

  // GET /api/branches/:branchName - 特定のブランチ情報を取得
  fastify.get<{
    Params: { branchName: string };
    Reply: ApiResponse<Branch>;
  }>("/api/branches/:branchName", async (request, reply) => {
    try {
      const { branchName } = request.params;
      const decodedBranchName = decodeURIComponent(branchName);

      const branch = await getBranchByName(decodedBranchName);
      if (!branch) {
        reply.code(404);
        return {
          success: false,
          error: "Branch not found",
          details: `Branch ${decodedBranchName} does not exist`,
        };
      }

      return { success: true, data: branch };
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      reply.code(500);
      return {
        success: false,
        error: "Failed to fetch branch",
        details: errorMsg,
      };
    }
  });

  // POST /api/branches/:branchName/sync - Fetch & fast-forward pull
  fastify.post<{
    Params: { branchName: string };
    Body: BranchSyncRequest;
    Reply: ApiResponse<BranchSyncResult>;
  }>("/api/branches/:branchName/sync", async (request, reply) => {
    const { branchName } = request.params;
    const decodedBranchName = decodeURIComponent(branchName);
    const { worktreePath } = request.body;

    if (!worktreePath) {
      reply.code(400);
      return {
        success: false,
        error: "worktreePath is required",
        details: null,
      };
    }

    try {
      const result = await syncBranchState(decodedBranchName, worktreePath);
      return { success: true, data: result };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("Branch not found")) {
        reply.code(404);
        return {
          success: false,
          error: "Branch not found",
          details: message,
        };
      }

      if (message.includes("Worktree path is required")) {
        reply.code(400);
        return {
          success: false,
          error: "Invalid request",
          details: message,
        };
      }

      reply.code(500);
      return {
        success: false,
        error: "Failed to sync branch",
        details: message,
      };
    }
  });
}
