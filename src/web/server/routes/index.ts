/**
 * REST API Routes
 *
 * すべてのREST APIエンドポイントを登録します。
 * 仕様: specs/SPEC-d5e56259/contracts/rest-api.yaml
 */

import type { FastifyInstance } from "fastify";
import type { PTYManager } from "../pty/manager.js";
import { registerBranchRoutes } from "./branches.js";
import { registerWorktreeRoutes } from "./worktrees.js";
import { registerSessionRoutes } from "./sessions.js";
import { registerConfigRoutes } from "./config.js";
import type { HealthResponse } from "../../../types/api.js";

/**
 * すべてのルートを登録
 */
export async function registerRoutes(
  fastify: FastifyInstance,
  ptyManager: PTYManager,
): Promise<void> {
  // ヘルスチェック
  fastify.get<{ Reply: HealthResponse }>("/api/health", async () => {
    return {
      success: true,
      status: "ok",
      timestamp: new Date().toISOString(),
    };
  });

  // 各エンドポイントグループを登録
  await registerBranchRoutes(fastify);
  await registerWorktreeRoutes(fastify);
  await registerSessionRoutes(fastify, ptyManager);
  await registerConfigRoutes(fastify);
}
