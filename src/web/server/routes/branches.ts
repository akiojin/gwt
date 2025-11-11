/**
 * Branch Routes
 *
 * ブランチ関連のREST APIエンドポイント。
 */

import type { FastifyInstance } from "fastify";
import { listBranches, getBranchByName } from "../services/branches.js";
import type { ApiResponse, Branch } from "../../../types/api.js";

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
}
