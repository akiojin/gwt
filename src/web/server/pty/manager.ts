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
import {
  resolveClaudeCommand,
  resolveCodexCommand,
  resolveCustomToolCommand,
  AIToolResolutionError,
  type ResolvedCommand,
} from "../../../services/aiToolResolver.js";

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
  public async spawn(
    toolType: "claude-code" | "codex-cli" | "custom",
    worktreePath: string,
    mode: "normal" | "continue" | "resume",
    options: {
      toolName?: string | null;
      cols?: number;
      rows?: number;
      skipPermissions?: boolean;
      bypassApprovals?: boolean;
      extraArgs?: string[];
      customToolId?: string | null;
    } = {},
  ): Promise<{ sessionId: string; session: AIToolSession }> {
    const cols = options.cols ?? 80;
    const rows = options.rows ?? 24;
    const toolName = options.toolName ?? null;
    const sessionId = randomUUID();

    const resolverOptions: {
      toolName?: string | null;
      skipPermissions?: boolean;
      bypassApprovals?: boolean;
      extraArgs?: string[];
      customToolId?: string | null;
    } = {};

    if (toolName !== null) {
      resolverOptions.toolName = toolName;
    }
    if (options.skipPermissions !== undefined) {
      resolverOptions.skipPermissions = options.skipPermissions;
    }
    if (options.bypassApprovals !== undefined) {
      resolverOptions.bypassApprovals = options.bypassApprovals;
    }
    if (options.extraArgs && options.extraArgs.length > 0) {
      resolverOptions.extraArgs = options.extraArgs;
    }
    if (options.customToolId !== undefined) {
      resolverOptions.customToolId = options.customToolId;
    }

    const resolved = await this.resolveCommand(toolType, mode, resolverOptions);

    const env: NodeJS.ProcessEnv = {
      ...process.env,
      TERM: "xterm-256color",
      COLORTERM: "truecolor",
    };

    if (resolved.env) {
      Object.assign(env, resolved.env);
    }

    if (toolType === "claude-code" && options.skipPermissions && isRootUser()) {
      env.IS_SANDBOX = "1";
    }

    // PTYプロセスをスポーン
    const ptyProcess = pty.spawn(resolved.command, resolved.args, {
      name: "xterm-256color",
      cols,
      rows,
      cwd: worktreePath,
      env,
    });

    const session: AIToolSession = {
      sessionId,
      toolType,
      toolName: options.customToolId ?? toolName ?? null,
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

  private async resolveCommand(
    toolType: "claude-code" | "codex-cli" | "custom",
    mode: "normal" | "continue" | "resume",
    options: {
      toolName?: string | null;
      skipPermissions?: boolean;
      bypassApprovals?: boolean;
      extraArgs?: string[];
      customToolId?: string | null;
    },
  ): Promise<ResolvedCommand> {
    if (toolType === "custom") {
      const toolId = options.customToolId ?? options.toolName;
      if (!toolId) {
        throw new AIToolResolutionError(
          "COMMAND_NOT_FOUND",
          "Custom tool identifier is required to start a session.",
        );
      }

      return resolveCustomToolCommand({
        toolId,
        mode,
        ...(options.skipPermissions !== undefined
          ? { skipPermissions: options.skipPermissions }
          : {}),
        ...(options.extraArgs ? { extraArgs: options.extraArgs } : {}),
      });
    }

    if (toolType === "codex-cli") {
      const codexOptions: {
        mode: "normal" | "continue" | "resume";
        bypassApprovals?: boolean;
        extraArgs?: string[];
      } = { mode };

      if (options.bypassApprovals !== undefined) {
        codexOptions.bypassApprovals = options.bypassApprovals;
      }
      if (options.extraArgs && options.extraArgs.length > 0) {
        codexOptions.extraArgs = options.extraArgs;
      }

      return resolveCodexCommand(codexOptions);
    }

    const claudeOptions: {
      mode: "normal" | "continue" | "resume";
      skipPermissions?: boolean;
      extraArgs?: string[];
    } = { mode };

    if (options.skipPermissions !== undefined) {
      claudeOptions.skipPermissions = options.skipPermissions;
    }
    if (options.extraArgs && options.extraArgs.length > 0) {
      claudeOptions.extraArgs = options.extraArgs;
    }

    return resolveClaudeCommand(claudeOptions);
  }
}

function isRootUser(): boolean {
  try {
    return typeof process.getuid === "function" && process.getuid() === 0;
  } catch {
    return false;
  }
}
