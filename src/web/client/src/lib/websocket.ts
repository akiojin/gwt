/**
 * WebSocket Client
 *
 * PTYセッションとの双方向通信を管理するクライアント。
 * 仕様: specs/SPEC-d5e56259/contracts/websocket.md
 */

import type {
  ServerMessage,
  InputMessage,
  ResizeMessage,
  PingMessage,
  OutputMessage,
  ExitMessage,
  ErrorMessage,
} from "../../../../types/api.js";

export type WebSocketEventHandler = {
  onOutput?: (data: string) => void;
  onExit?: (code: number, signal?: string) => void;
  onError?: (message: string) => void;
  onPong?: () => void;
  onOpen?: () => void;
  onClose?: () => void;
};

/**
 * PTY WebSocketクライアント
 */
export class PTYWebSocket {
  private ws: WebSocket | null = null;
  private handlers: WebSocketEventHandler;
  private sessionId: string;

  constructor(sessionId: string, handlers: WebSocketEventHandler) {
    this.sessionId = sessionId;
    this.handlers = handlers;
  }

  /**
   * WebSocket接続を確立
   */
  public connect(): void {
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const host = window.location.host;
    const url = `${protocol}//${host}/api/sessions/${this.sessionId}/terminal?sessionId=${this.sessionId}`;

    this.ws = new WebSocket(url);

    this.ws.onopen = () => {
      this.handlers.onOpen?.();
    };

    this.ws.onmessage = (event) => {
      try {
        const message: ServerMessage = JSON.parse(event.data);
        this.handleServerMessage(message);
      } catch (error) {
        console.error("Failed to parse WebSocket message:", error);
      }
    };

    this.ws.onerror = (event) => {
      console.error("WebSocket error:", event);
      this.handlers.onError?.("WebSocket connection error");
    };

    this.ws.onclose = () => {
      this.handlers.onClose?.();
    };
  }

  /**
   * サーバーメッセージを処理
   */
  private handleServerMessage(message: ServerMessage): void {
    switch (message.type) {
      case "output": {
        const outputMsg = message as OutputMessage;
        this.handlers.onOutput?.(outputMsg.data);
        break;
      }
      case "exit": {
        const exitMsg = message as ExitMessage;
        this.handlers.onExit?.(exitMsg.data.code, exitMsg.data.signal);
        break;
      }
      case "error": {
        const errorMsg = message as ErrorMessage;
        this.handlers.onError?.(errorMsg.data.message);
        break;
      }
      case "pong": {
        this.handlers.onPong?.();
        break;
      }
      default: {
        const unknownMsg = message as { type: string };
        console.warn("Unknown server message type:", unknownMsg.type);
        break;
      }
    }
  }

  /**
   * 入力データを送信
   */
  public sendInput(data: string): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket is not connected");
      return;
    }

    const message: InputMessage = {
      type: "input",
      data,
      timestamp: new Date().toISOString(),
    };

    this.ws.send(JSON.stringify(message));
  }

  /**
   * ターミナルサイズ変更を送信
   */
  public sendResize(cols: number, rows: number): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket is not connected");
      return;
    }

    const message: ResizeMessage = {
      type: "resize",
      data: { cols, rows },
      timestamp: new Date().toISOString(),
    };

    this.ws.send(JSON.stringify(message));
  }

  /**
   * Pingを送信
   */
  public sendPing(): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket is not connected");
      return;
    }

    const message: PingMessage = {
      type: "ping",
      timestamp: new Date().toISOString(),
    };

    this.ws.send(JSON.stringify(message));
  }

  /**
   * WebSocket接続を切断
   */
  public disconnect(): void {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  /**
   * 接続状態を取得
   */
  public isConnected(): boolean {
    return this.ws !== null && this.ws.readyState === WebSocket.OPEN;
  }
}
