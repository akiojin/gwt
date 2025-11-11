/**
 * Web UI Server Entry Point
 *
 * Fastifyベースのウェブサーバーを起動し、REST APIとWebSocketを提供します。
 * 仕様: specs/SPEC-d5e56259/contracts/rest-api.yaml
 */

import Fastify from "fastify";
import fastifyStatic from "@fastify/static";
import fastifyWebsocket from "@fastify/websocket";
import pino from "pino";
import type { FastifyLoggerOptions } from "fastify";
import { mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { PTYManager } from "./pty/manager.js";
import { WebSocketHandler } from "./websocket/handler.js";
import { registerRoutes } from "./routes/index.js";
import { importOsEnvIntoSharedConfig } from "./env/importer.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

/**
 * Webサーバーを起動
 */
export async function startWebServer(): Promise<void> {
  const fastify = Fastify({
    logger: await createLoggerOptions(),
  });

  // PTYマネージャーとWebSocketハンドラーを初期化
  const ptyManager = new PTYManager();
  const wsHandler = new WebSocketHandler(ptyManager);

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

  // SPA Fallback: serve index.html for non-API routes (e.g., refresh on nested paths)
  fastify.setNotFoundHandler((request, reply) => {
    const url = request.url || "";
    if (request.method === "GET" && !url.startsWith("/api")) {
      return reply.sendFile("index.html");
    }

    return reply.status(404).send({
      success: false,
      error: `Route ${request.method}:${url} not found`,
      details: null,
    });
  });

  // サーバー起動
  try {
    const port = process.env.PORT ? parseInt(process.env.PORT, 10) : 3000;
    // Docker環境からホストOSでアクセスできるよう、0.0.0.0でリッスン
    // IPv4/IPv6両方対応のため、listenOnStart: false も検討可能
    const host = process.env.HOST || "0.0.0.0";

    await fastify.listen({ port, host });
    console.log(`Web UI server running at http://${host}:${port}`);
    console.log(`Access from host: http://localhost:${port}`);
  } catch (err) {
    fastify.log.error(err);
    process.exit(1);
  }
}

async function createLoggerOptions(): Promise<FastifyLoggerOptions> {
  const level = process.env.LOG_LEVEL || "info";
  const logDir =
    process.env.CLAUDE_WORKTREE_LOG_DIR ??
    join(homedir(), ".claude-worktree", "logs");
  await mkdir(logDir, { recursive: true });
  const logFilePath = join(
    logDir,
    process.env.CLAUDE_WORKTREE_LOG_FILE ?? "webui.log",
  );

  const fileStream = pino.destination({ dest: logFilePath, sync: false });

  if (process.env.CLAUDE_WORKTREE_LOG_STDOUT === "false") {
    return {
      level,
      stream: fileStream,
    } satisfies FastifyLoggerOptions;
  }

  const combinedStream = pino.multistream([
    { stream: process.stdout },
    { stream: fileStream },
  ]);

  return {
    level,
    stream: combinedStream,
  } satisfies FastifyLoggerOptions;
}
