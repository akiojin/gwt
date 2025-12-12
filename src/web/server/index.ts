/**
 * Web UI Server Entry Point
 *
 * Fastifyベースのウェブサーバーを起動し、REST APIとWebSocketを提供します。
 * 仕様: specs/SPEC-d5e56259/contracts/rest-api.yaml
 */

import Fastify from "fastify";
import fastifyStatic from "@fastify/static";
import fastifyWebsocket from "@fastify/websocket";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { PTYManager } from "./pty/manager.js";
import { WebSocketHandler } from "./websocket/handler.js";
import { registerRoutes } from "./routes/index.js";
import { importOsEnvIntoSharedConfig } from "./env/importer.js";
import { createLogger } from "../../logging/logger.js";
import type { WebFastifyInstance } from "./types.js";
import { startSystemTray } from "./tray.js";
import { resolveWebUiPort } from "../../utils/webui.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

/**
 * Webサーバーを起動
 */
export async function startWebServer(): Promise<void> {
  const serverLogger = createLogger({ category: "server" });

  const fastify: WebFastifyInstance = Fastify({
    loggerInstance: serverLogger,
  });

  // PTYマネージャーとWebSocketハンドラーを初期化
  const ptyManager = new PTYManager();
  const wsHandler = new WebSocketHandler(ptyManager, fastify.log);

  // WebSocketサポートを追加
  await fastify.register(fastifyWebsocket);

  // WebSocketエンドポイント
  fastify.register(async (fastify) => {
    fastify.get(
      "/api/sessions/:sessionId/terminal",
      { websocket: true },
      (connection, request) => {
        wsHandler.handle(connection, request);
      },
    );
  });

  // REST APIルートを登録
  await importOsEnvIntoSharedConfig();
  await registerRoutes(fastify, ptyManager);

  // 静的ファイル配信（Viteビルド成果物）
  const clientDistPath = join(__dirname, "../../../dist/client");
  await fastify.register(fastifyStatic, {
    root: clientDistPath,
    prefix: "/",
  });

  // サーバー起動
  try {
    const port = resolveWebUiPort();
    // Docker環境からホストOSでアクセスできるよう、0.0.0.0でリッスン
    // IPv4/IPv6両方対応のため、listenOnStart: false も検討可能
    const host = process.env.HOST || "0.0.0.0";

    await fastify.listen({ port, host });
    console.log(`Web UI server running at http://${host}:${port}`);
    const accessUrl = `http://localhost:${port}`;
    console.log(`Access from host: ${accessUrl}`);
    await startSystemTray(accessUrl);
  } catch (err) {
    fastify.log.error(err);
    process.exit(1);
  }
}
