/**
 * Config Routes
 *
 * カスタムAI Tool設定関連のREST APIエンドポイント。
 */

import type { FastifyInstance } from "fastify";
import type {
  ApiResponse,
  CustomAITool,
  UpdateConfigRequest,
} from "../../../types/api.js";

/**
 * 設定関連のルートを登録
 */
export async function registerConfigRoutes(
  fastify: FastifyInstance,
): Promise<void> {
  // GET /api/config - カスタムAI Tool設定を取得
  fastify.get<{ Reply: ApiResponse<{ tools: CustomAITool[] }> }>(
    "/api/config",
    async () => {
      // TODO: config.jsonから設定を読み込む
      // 現在は空の配列を返す
      return {
        success: true,
        data: {
          tools: [],
        },
      };
    },
  );

  // PUT /api/config - カスタムAI Tool設定を更新
  fastify.put<{
    Body: UpdateConfigRequest;
    Reply: ApiResponse<{ tools: CustomAITool[] }>;
  }>("/api/config", async (request, reply) => {
    try {
      const { tools } = request.body;

      // TODO: config.jsonに設定を保存する
      // 現在は受け取った設定をそのまま返す
      return {
        success: true,
        data: {
          tools,
        },
      };
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      reply.code(500);
      return {
        success: false,
        error: "Failed to update config",
        details: errorMsg,
      };
    }
  });
}
