/**
 * PTY Manager
 *
 * コーディングエージェントセッションの疑似端末(PTY)を管理します。
 * node-ptyを使用してプロセスをスポーンし、WebSocketを通じて入出力を中継します。
 */

import * as pty from "node-pty";
import type { IPty } from "node-pty";
import { randomUUID } from "node:crypto";
import { execa } from "execa";
import type { CodingAgentSession } from "../../../types/api.js";
import {
  resolveClaudeCommand,
  resolveCodexCommand,
  resolveCodingAgentCommand,
  CodingAgentResolutionError,
  type ResolvedCommand,
} from "../../../services/codingAgentResolver.js";
import { loadCodingAgentsConfig } from "../../../config/tools.js";
import { saveSession } from "../../../config/index.js";
import {
  findLatestClaudeSession,
  findLatestCodexSession,
} from "../../../utils/session.js";
import { createLogger } from "../../../logging/logger.js";

const logger = createLogger({ category: "pty" });

export interface PTYInstance {
  ptyProcess: IPty;
  session: CodingAgentSession;
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
      resumeSessionId?: string | null;
    } = {},
  ): Promise<{ sessionId: string; session: CodingAgentSession }> {
    const cols = options.cols ?? 80;
    const rows = options.rows ?? 24;
    const toolName = options.toolName ?? null;
    const sessionId = randomUUID();
    const startedAt = Date.now();
    const resumeSessionId =
      options.resumeSessionId && options.resumeSessionId.trim().length > 0
        ? options.resumeSessionId.trim()
        : null;

    logger.debug(
      { sessionId, toolType, mode, worktreePath, cols, rows },
      "Spawning PTY session",
    );

    const resolverOptions: {
      toolName?: string | null;
      skipPermissions?: boolean;
      bypassApprovals?: boolean;
      extraArgs?: string[];
      customToolId?: string | null;
      sessionId?: string | null;
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
    if (options.resumeSessionId !== undefined) {
      resolverOptions.sessionId = options.resumeSessionId;
    }

    const resolved = await this.resolveCommand(toolType, mode, resolverOptions);

    logger.debug(
      {
        toolType,
        command: resolved.command,
        argsCount: resolved.args.length,
        usesFallback: resolved.usesFallback,
        hasEnv: !!resolved.env,
      },
      "Command resolved",
    );

    const sharedEnv = await this.loadSharedEnv();

    const env: NodeJS.ProcessEnv = {
      ...process.env,
      ...sharedEnv,
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

    logger.info(
      {
        sessionId,
        command: resolved.command,
        argsCount: resolved.args.length,
        pid: ptyProcess.pid,
        cwd: worktreePath,
        envKeys: Object.keys(env).filter(
          (k) => !k.startsWith("npm_") && !k.startsWith("BUN_"),
        ),
      },
      "PTY process spawned",
    );

    const session: CodingAgentSession = {
      sessionId,
      agentType: toolType,
      agentName: options.customToolId ?? toolName ?? null,
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

    ptyProcess.onExit(() => {
      void this.persistSessionIdOnExit({
        session,
        startedAt,
        resumeSessionId,
      });
    });

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
      logger.warn({ sessionId, reason: "not found" }, "Session delete failed");
      return false;
    }

    // PTYプロセスを終了
    try {
      instance.ptyProcess.kill();
    } catch {
      // プロセスが既に終了している場合は無視
    }

    this.instances.delete(sessionId);
    logger.info({ sessionId }, "Session deleted");
    return true;
  }

  /**
   * セッションのステータスを更新
   */
  public updateStatus(
    sessionId: string,
    status: CodingAgentSession["status"],
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

    logger.debug({ sessionId, status, exitCode }, "Session status updated");
    return true;
  }

  /**
   * すべてのセッション一覧を取得
   */
  public list(): CodingAgentSession[] {
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
      sessionId?: string | null;
    },
  ): Promise<ResolvedCommand> {
    if (toolType === "custom") {
      const agentId = options.customToolId ?? options.toolName;
      if (!agentId) {
        throw new CodingAgentResolutionError(
          "COMMAND_NOT_FOUND",
          "Coding agent identifier is required to start a session.",
        );
      }

      return resolveCodingAgentCommand({
        agentId,
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
        sessionId?: string | null;
      } = { mode };

      if (options.bypassApprovals !== undefined) {
        codexOptions.bypassApprovals = options.bypassApprovals;
      }
      if (options.extraArgs && options.extraArgs.length > 0) {
        codexOptions.extraArgs = options.extraArgs;
      }
      if (options.sessionId) {
        codexOptions.sessionId = options.sessionId;
      }

      return resolveCodexCommand(codexOptions);
    }

    const claudeOptions: {
      mode: "normal" | "continue" | "resume";
      skipPermissions?: boolean;
      extraArgs?: string[];
      sessionId?: string | null;
    } = { mode };

    if (options.skipPermissions !== undefined) {
      claudeOptions.skipPermissions = options.skipPermissions;
    }
    if (options.extraArgs && options.extraArgs.length > 0) {
      claudeOptions.extraArgs = options.extraArgs;
    }
    if (options.sessionId) {
      claudeOptions.sessionId = options.sessionId;
    }

    return resolveClaudeCommand(claudeOptions);
  }

  private async loadSharedEnv(): Promise<Record<string, string>> {
    const config = await loadCodingAgentsConfig();
    const sharedEnv = { ...(config.env ?? {}) };
    logger.debug(
      { keyCount: Object.keys(sharedEnv).length },
      "Shared env loaded",
    );
    return sharedEnv;
  }

  private async persistSessionIdOnExit(params: {
    session: CodingAgentSession;
    startedAt: number;
    resumeSessionId: string | null;
  }): Promise<void> {
    const { session, startedAt, resumeSessionId } = params;
    if (
      session.agentType !== "claude-code" &&
      session.agentType !== "codex-cli"
    ) {
      return;
    }

    const finishedAt = Date.now();
    let detectedSessionId: string | null = null;

    try {
      if (session.agentType === "codex-cli") {
        const latest = await findLatestCodexSession({
          since: startedAt - 60_000,
          until: finishedAt + 60_000,
          preferClosestTo: finishedAt,
          windowMs: 60 * 60 * 1000,
          cwd: session.worktreePath,
        });
        detectedSessionId = latest?.id ?? null;
      } else if (session.agentType === "claude-code") {
        const latest = await findLatestClaudeSession(session.worktreePath, {
          since: startedAt - 60_000,
          until: finishedAt + 60_000,
          preferClosestTo: finishedAt,
          windowMs: 60 * 60 * 1000,
        });
        detectedSessionId = latest?.id ?? null;
      }
    } catch (error: unknown) {
      logger.debug(
        { err: error, sessionId: session.sessionId },
        "Failed to detect session ID",
      );
    }

    const finalSessionId = detectedSessionId ?? resumeSessionId;
    if (!finalSessionId) {
      return;
    }

    try {
      const context = await resolveGitContext(session.worktreePath);
      if (!context.repoRoot) {
        return;
      }

      await saveSession({
        lastWorktreePath: session.worktreePath,
        lastBranch: context.branchName,
        lastUsedTool:
          session.agentType === "custom"
            ? (session.agentName ?? "custom")
            : session.agentType,
        toolLabel:
          session.agentType === "custom"
            ? (session.agentName ?? "Custom")
            : agentLabelFromType(session.agentType),
        mode: session.mode,
        timestamp: Date.now(),
        repositoryRoot: context.repoRoot,
        lastSessionId: finalSessionId,
      });
    } catch (error: unknown) {
      logger.debug(
        { err: error, sessionId: session.sessionId },
        "Failed to persist session ID",
      );
    }
  }
}

function isRootUser(): boolean {
  try {
    return typeof process.getuid === "function" && process.getuid() === 0;
  } catch {
    return false;
  }
}

async function resolveGitContext(worktreePath: string): Promise<{
  repoRoot: string | null;
  branchName: string | null;
}> {
  try {
    const { stdout: repoRoot } = await execa(
      "git",
      ["rev-parse", "--show-toplevel"],
      { cwd: worktreePath },
    );
    const normalizedRoot = repoRoot.trim();
    let branchName: string | null = null;
    try {
      const { stdout: branchStdout } = await execa(
        "git",
        ["rev-parse", "--abbrev-ref", "HEAD"],
        { cwd: worktreePath },
      );
      branchName = branchStdout.trim() || null;
    } catch {
      branchName = null;
    }

    return { repoRoot: normalizedRoot || null, branchName };
  } catch {
    return { repoRoot: null, branchName: null };
  }
}

function agentLabelFromType(agentType: "claude-code" | "codex-cli" | "custom") {
  if (agentType === "claude-code") return "Claude";
  if (agentType === "codex-cli") return "Codex";
  return "Custom";
}
