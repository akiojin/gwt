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
import { disposeSystemTray, startSystemTray } from "./tray.js";
import { resolveWebUiPort } from "../../utils/webui.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

/**
 * Web UI サーバーのライフサイクル操作ハンドル。
 *
 * `close()` は Web UI サーバーを停止し、関連リソース（PTY/トレイ等）を解放します。
 */
export interface WebServerHandle {
  close: () => Promise<void>;
}

/**
 * `startWebServer` の起動オプション。
 */
export interface StartWebServerOptions {
  /**
   * true の場合、Web UI サーバーが CLI 本体の終了をブロックしないようにします。
   * （内部で `server.unref()` を呼び、サーバーの存在がプロセス生存を維持しないようにします）
   */
  background?: boolean;
}

/**
 * Web UI サーバーを起動します。
 *
 * @param options - 起動オプション
 * @returns Web UI サーバー停止用のハンドル
 * @throws サーバー起動（listen/初期化）に失敗した場合
 */
export async function startWebServer(
  options: StartWebServerOptions = {},
): Promise<WebServerHandle> {
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

  // SPAフォールバック: 未知のルートにindex.htmlを返す
  fastify.setNotFoundHandler(async (request, reply) => {
    // APIリクエストは404を返す
    if (request.url.startsWith("/api/")) {
      return reply.status(404).send({ error: "Not Found", statusCode: 404 });
    }
    // それ以外はindex.htmlを返してReact Routerに処理を委譲
    return reply.sendFile("index.html");
  });

  // サーバー起動
  const port = resolveWebUiPort();
  // Docker環境からホストOSでアクセスできるよう、0.0.0.0でリッスン
  // IPv4/IPv6両方対応のため、listenOnStart: false も検討可能
  const host = process.env.HOST || "0.0.0.0";

  await fastify.listen({ port, host });
  const accessUrl = `http://localhost:${port}`;
  serverLogger.info({ host, port, accessUrl }, "Web UI server started");
  await startSystemTray(accessUrl);

  if (options.background) {
    fastify.server?.unref?.();
  }

  let closed = false;
  return {
    close: async () => {
      if (closed) return;
      closed = true;

      try {
        try {
          disposeSystemTray();
        } catch (err) {
          serverLogger.warn({ err }, "System tray cleanup failed");
        }

        for (const session of ptyManager.list()) {
          try {
            ptyManager.delete(session.sessionId);
          } catch (err) {
            serverLogger.warn(
              { err, sessionId: session.sessionId },
              "Failed to delete PTY session",
            );
          }
        }
      } finally {
        await fastify.close();
      }
    },
  };
}
