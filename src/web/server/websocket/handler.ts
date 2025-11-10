/**
 * WebSocket Handler
 *
 * PTYプロセスとブラウザ間のWebSocket通信を仲介します。
 * 仕様: specs/SPEC-d5e56259/contracts/websocket.md
 */

import type { FastifyRequest } from "fastify";
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
  constructor(private ptyManager: PTYManager) {}

  /**
   * WebSocket接続を処理
   */
  public handle(connection: WebSocket, request: FastifyRequest): void {
    const url = new URL(
      request.url,
      `http://${request.hostname || "localhost"}`,
    );
    const sessionId = url.searchParams.get("sessionId");

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

    const { ptyProcess } = instance;

    // セッションステータスを更新
    this.ptyManager.updateStatus(sessionId, "running");

    // PTYプロセスからの出力をWebSocketに転送
    ptyProcess.onData((data) => {
      this.sendOutput(connection, data);
    });

    // PTYプロセス終了時の処理
    ptyProcess.onExit(({ exitCode, signal }) => {
      this.ptyManager.updateStatus(sessionId, "completed", exitCode);
      this.sendExit(connection, exitCode, signal);
      connection.close();
    });

    // クライアントからのメッセージを処理
    connection.on("message", (rawMessage) => {
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
      console.error(`WebSocket error for session ${sessionId}:`, error);
      this.ptyManager.updateStatus(
        sessionId,
        "failed",
        undefined,
        error.message,
      );
    });

    // 接続クローズ時の処理
    connection.on("close", () => {
      console.log(`WebSocket closed for session ${sessionId}`);
      // PTYプロセスは残す（バックグラウンドで実行継続）
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
        ptyProcess.write(inputMsg.data);
        break;
      }
      case "resize": {
        const resizeMsg = message as ResizeMessage;
        ptyProcess.resize(resizeMsg.data.cols, resizeMsg.data.rows);
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
}
