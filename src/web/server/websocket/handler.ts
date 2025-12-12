/**
 * WebSocket Handler
 *
 * PTYプロセスとブラウザ間のWebSocket通信を仲介します。
 * 仕様: specs/SPEC-d5e56259/contracts/websocket.md
 */

import type { FastifyRequest, FastifyBaseLogger } from "fastify";
import type { WebSocket } from "@fastify/websocket";
import type { PTYManager } from "../pty/manager.js";
import type {
  ClientMessage,
  InputMessage,
  ResizeMessage,
  OutputMessage,
  ExitMessage,
  ErrorMessage,
  PongMessage,
} from "../../../types/api.js";

/**
 * WebSocketハンドラー
 */
export class WebSocketHandler {
  private cleanupTimers: Map<string, NodeJS.Timeout> = new Map();

  constructor(
    private ptyManager: PTYManager,
    private logger: FastifyBaseLogger,
  ) {}

  /**
   * WebSocket接続を処理
   */
  public handle(connection: WebSocket, request: FastifyRequest): void {
    const sessionId = resolveSessionId(request);

    if (!sessionId) {
      this.sendError(connection, "Missing sessionId parameter");
      connection.close();
      return;
    }

    const instance = this.ptyManager.get(sessionId);
    if (!instance) {
      this.sendError(connection, `Session not found: ${sessionId}`);
      connection.close();
      return;
    }

    this.logger.info(
      `WebSocket connection established for session ${sessionId} (pid=${instance.ptyProcess.pid})`,
    );

    this.clearCleanupTimer(sessionId);

    const { ptyProcess } = instance;
    let hasExited = false;

    // セッションステータスを更新
    this.ptyManager.updateStatus(sessionId, "running");

    // PTYプロセスからの出力をWebSocketに転送
    ptyProcess.onData((data) => {
      this.sendOutput(connection, data);
    });

    // PTYプロセス終了時の処理
    ptyProcess.onExit(({ exitCode, signal }) => {
      hasExited = true;
      this.clearCleanupTimer(sessionId);
      this.ptyManager.updateStatus(sessionId, "completed", exitCode);
      this.sendExit(connection, exitCode, signal);
      connection.close();
    });

    // クライアントからのメッセージを処理
    connection.on("message", (rawMessage) => {
      if (hasExited) {
        this.sendError(connection, "Session already exited");
        return;
      }
      try {
        const message: ClientMessage = JSON.parse(rawMessage.toString());
        this.handleClientMessage(message, ptyProcess, connection);
      } catch (error) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        this.sendError(connection, `Invalid message format: ${errorMsg}`);
      }
    });

    // 接続エラー時の処理
    connection.on("error", (error) => {
      this.logger.error(
        { err: error, sessionId },
        `WebSocket error for session ${sessionId}`,
      );
      this.ptyManager.updateStatus(
        sessionId,
        "failed",
        undefined,
        error.message,
      );
    });

    // 接続クローズ時の処理
    connection.on("close", (code, reason) => {
      const reasonText = reason?.toString()?.trim();
      this.logger.info(
        `WebSocket closed for session ${sessionId} (code=${code}${reasonText ? `, reason=${reasonText}` : ""})`,
      );

      if (hasExited) {
        return;
      }

      this.scheduleCleanup(sessionId);
    });
  }

  /**
   * クライアントメッセージを処理
   */
  private handleClientMessage(
    message: ClientMessage,
    ptyProcess: ReturnType<typeof import("node-pty").spawn>,
    connection: WebSocket,
  ): void {
    switch (message.type) {
      case "input": {
        const inputMsg = message as InputMessage;
        try {
          ptyProcess.write(inputMsg.data);
        } catch (error) {
          const reason = error instanceof Error ? error.message : String(error);
          this.sendError(connection, `Failed to write to session: ${reason}`);
        }
        break;
      }
      case "resize": {
        const resizeMsg = message as ResizeMessage;
        try {
          ptyProcess.resize(resizeMsg.data.cols, resizeMsg.data.rows);
        } catch (error) {
          const reason = error instanceof Error ? error.message : String(error);
          this.sendError(connection, `Failed to resize terminal: ${reason}`);
        }
        break;
      }
      case "ping": {
        this.sendPong(connection);
        break;
      }
      default: {
        const unknownMsg = message as { type: string };
        this.sendError(connection, `Unknown message type: ${unknownMsg.type}`);
        break;
      }
    }
  }

  /**
   * 出力メッセージを送信
   */
  private sendOutput(connection: WebSocket, data: string): void {
    const message: OutputMessage = {
      type: "output",
      data,
      timestamp: new Date().toISOString(),
    };
    connection.send(JSON.stringify(message));
  }

  /**
   * 終了メッセージを送信
   */
  private sendExit(connection: WebSocket, code: number, signal?: number): void {
    const exitData: { code: number; signal?: string } = { code };
    if (signal !== undefined) {
      exitData.signal = String(signal);
    }

    const message: ExitMessage = {
      type: "exit",
      data: exitData,
      timestamp: new Date().toISOString(),
    };
    connection.send(JSON.stringify(message));
  }

  /**
   * エラーメッセージを送信
   */
  private sendError(connection: WebSocket, errorMsg: string): void {
    const message: ErrorMessage = {
      type: "error",
      data: {
        message: errorMsg,
      },
      timestamp: new Date().toISOString(),
    };
    connection.send(JSON.stringify(message));
  }

  /**
   * Pongメッセージを送信
   */
  private sendPong(connection: WebSocket): void {
    const message: PongMessage = {
      type: "pong",
      timestamp: new Date().toISOString(),
    };
    connection.send(JSON.stringify(message));
  }

  private scheduleCleanup(sessionId: string): void {
    this.clearCleanupTimer(sessionId);
    const timer = setTimeout(() => {
      this.cleanupTimers.delete(sessionId);
      const tracked = this.ptyManager.get(sessionId);
      if (!tracked) {
        return;
      }

      if (
        tracked.session.status === "running" ||
        tracked.session.status === "pending"
      ) {
        this.logger.warn(
          `Auto-cleaning session ${sessionId} after unexpected client disconnect`,
        );
        this.ptyManager.updateStatus(
          sessionId,
          "failed",
          undefined,
          "Client disconnected",
        );
        this.ptyManager.delete(sessionId);
      }
    }, CLEANUP_GRACE_PERIOD_MS);
    this.cleanupTimers.set(sessionId, timer);
  }

  private clearCleanupTimer(sessionId: string): void {
    const timer = this.cleanupTimers.get(sessionId);
    if (timer) {
      clearTimeout(timer);
      this.cleanupTimers.delete(sessionId);
      this.logger.info(`Cleared cleanup timer for session ${sessionId}`);
    }
  }
}

const CLEANUP_GRACE_PERIOD_MS = Number(process.env.WS_CLEANUP_GRACE_MS ?? 3000);

interface RequestLike {
  params?: { sessionId?: string } | undefined;
  url: string;
  hostname?: string;
  headers?: { host?: string | undefined };
}

export function resolveSessionId(
  request: FastifyRequest | RequestLike,
): string | null {
  const paramsId = (request as RequestLike).params?.sessionId;
  if (typeof paramsId === "string" && paramsId.length > 0) {
    return paramsId;
  }

  try {
    const host =
      (request as FastifyRequest).headers?.host ??
      (request as RequestLike).hostname ??
      "localhost";
    const parsed = new URL(request.url, `http://${host}`);
    const queryId = parsed.searchParams.get("sessionId");
    if (queryId && queryId.length > 0) {
      return queryId;
    }
  } catch {
    // ignore
  }

  return null;
}
