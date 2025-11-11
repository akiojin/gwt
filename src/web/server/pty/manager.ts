/**
 * PTY Manager
 *
 * AI Toolセッションの疑似端末(PTY)を管理します。
 * node-ptyを使用してプロセスをスポーンし、WebSocketを通じて入出力を中継します。
 */

import * as pty from "node-pty";
import type { IPty } from "node-pty";
import { randomUUID } from "node:crypto";
import type { AIToolSession } from "../../../types/api.js";

export interface PTYInstance {
  ptyProcess: IPty;
  session: AIToolSession;
}

/**
 * PTYマネージャー - セッションとPTYプロセスのライフサイクル管理
 */
export class PTYManager {
  private instances: Map<string, PTYInstance> = new Map();

  /**
   * 新しいPTYセッションを作成
   */
  public spawn(
    toolType: "claude-code" | "codex-cli" | "custom",
    worktreePath: string,
    mode: "normal" | "continue" | "resume",
    toolName?: string | null,
    cols = 80,
    rows = 24,
  ): { sessionId: string; session: AIToolSession } {
    const sessionId = randomUUID();

    // AI Toolコマンドを構築
    const command = this.buildCommand(toolType, mode, toolName);
    const args = this.buildArgs(toolType, mode);

    // PTYプロセスをスポーン
    const ptyProcess = pty.spawn(command, args, {
      name: "xterm-256color",
      cols,
      rows,
      cwd: worktreePath,
      env: {
        ...process.env,
        TERM: "xterm-256color",
        COLORTERM: "truecolor",
      },
    });

    const session: AIToolSession = {
      sessionId,
      toolType,
      toolName: toolName || null,
      mode,
      worktreePath,
      ptyPid: ptyProcess.pid,
      websocketId: null,
      status: "pending",
      startedAt: new Date().toISOString(),
      endedAt: null,
      exitCode: null,
      errorMessage: null,
    };

    this.instances.set(sessionId, { ptyProcess, session });

    return { sessionId, session };
  }

  /**
   * セッションIDからPTYインスタンスを取得
   */
  public get(sessionId: string): PTYInstance | undefined {
    return this.instances.get(sessionId);
  }

  /**
   * セッションを削除
   */
  public delete(sessionId: string): boolean {
    const instance = this.instances.get(sessionId);
    if (!instance) {
      return false;
    }

    // PTYプロセスを終了
    try {
      instance.ptyProcess.kill();
    } catch {
      // プロセスが既に終了している場合は無視
    }

    this.instances.delete(sessionId);
    return true;
  }

  /**
   * セッションのステータスを更新
   */
  public updateStatus(
    sessionId: string,
    status: AIToolSession["status"],
    exitCode?: number,
    errorMessage?: string,
  ): boolean {
    const instance = this.instances.get(sessionId);
    if (!instance) {
      return false;
    }

    instance.session.status = status;
    if (exitCode !== undefined) {
      instance.session.exitCode = exitCode;
    }
    if (errorMessage !== undefined) {
      instance.session.errorMessage = errorMessage;
    }
    if (status === "completed" || status === "failed") {
      instance.session.endedAt = new Date().toISOString();
    }

    return true;
  }

  /**
   * すべてのセッション一覧を取得
   */
  public list(): AIToolSession[] {
    return Array.from(this.instances.values()).map((inst) => inst.session);
  }

  /**
   * AI Toolのコマンドを構築
   */
  private buildCommand(
    toolType: "claude-code" | "codex-cli" | "custom",
    mode: "normal" | "continue" | "resume",
    toolName?: string | null,
  ): string {
    if (toolType === "custom" && toolName) {
      // カスタムツールは別途config.jsonから取得する必要があるが、
      // ここでは簡易実装としてtoolNameをそのままコマンドとして使用
      return toolName;
    }

    if (toolType === "codex-cli") {
      return "codex";
    }

    // claude-code
    return "claude";
  }

  /**
   * AI Toolの引数を構築
   */
  private buildArgs(
    toolType: "claude-code" | "codex-cli" | "custom",
    mode: "normal" | "continue" | "resume",
  ): string[] {
    if (toolType === "custom") {
      // カスタムツールの引数は別途config.jsonから取得
      return [];
    }

    if (toolType === "codex-cli") {
      if (mode === "continue") {
        return ["--continue"];
      }
      if (mode === "resume") {
        return ["--resume"];
      }
      return [];
    }

    // claude-code
    if (mode === "continue") {
      return ["--continue"];
    }
    if (mode === "resume") {
      return ["--resume"];
    }
    return [];
  }
}
