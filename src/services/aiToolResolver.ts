import { execa } from "execa";
import { platform } from "os";
import { getToolById } from "../config/tools.js";
import { CLAUDE_CODE_TOOL } from "../config/builtin-coding-agents.js";
import {
  CODEX_DEFAULT_ARGS,
  CLAUDE_PERMISSION_SKIP_ARGS,
} from "../shared/codingAgentConstants.js";
import { prepareCustomToolExecution } from "./customToolResolver.js";
import type { LaunchOptions } from "../types/tools.js";
import { createLogger } from "../logging/logger.js";

const logger = createLogger({ category: "resolver" });

const DETECTION_COMMAND = platform() === "win32" ? "where" : "which";
const MIN_BUN_MAJOR = 1;

export const CLAUDE_CLI_PACKAGE = "@anthropic-ai/claude-code@latest";
export const CODEX_CLI_PACKAGE = "@openai/codex@latest";

export type ResolverErrorCode =
  | "COMMAND_NOT_FOUND"
  | "BUNX_NOT_FOUND"
  | "BUN_TOO_OLD"
  | "CUSTOM_TOOL_NOT_FOUND";

export interface ResolvedCommand {
  command: string;
  args: string[];
  usesFallback: boolean;
  env?: NodeJS.ProcessEnv;
}

export class AIToolResolutionError extends Error {
  constructor(
    public code: ResolverErrorCode,
    message: string,
    public hints?: string[],
  ) {
    super(message);
    this.name = "AIToolResolutionError";
  }
}

async function commandExists(command: string): Promise<boolean> {
  try {
    await execa(DETECTION_COMMAND, [command], { shell: true });
    logger.debug({ command, exists: true }, "Command check");
    return true;
  } catch {
    logger.debug({ command, exists: false }, "Command check");
    return false;
  }
}

/**
 * コマンドのフルパスを取得
 * node-ptyはシェルを経由しないため、フルパスが必要
 */
async function resolveCommandPath(command: string): Promise<string | null> {
  try {
    const { stdout } = await execa(DETECTION_COMMAND, [command], {
      shell: true,
    });
    const fullPath = stdout.trim().split("\n")[0];
    logger.debug({ command, fullPath }, "Command path resolved");
    return fullPath || null;
  } catch {
    logger.debug({ command, fullPath: null }, "Command path resolution failed");
    return null;
  }
}

let bunxCheckPromise: Promise<void> | null = null;

async function ensureBunxAvailable(): Promise<void> {
  if (!bunxCheckPromise) {
    bunxCheckPromise = (async () => {
      const bunxExists = await commandExists("bunx");
      if (!bunxExists) {
        throw new AIToolResolutionError(
          "BUNX_NOT_FOUND",
          "bunx command not found. Install Bun 1.0+ so bunx is available on PATH.",
          [
            "Install Bun: https://bun.sh/docs/installation",
            "After installation, restart your terminal so bunx is on PATH.",
          ],
        );
      }

      try {
        const { stdout } = await execa("bun", ["--version"]);
        const version = stdout.trim();
        logger.debug({ bunVersion: version }, "Bun version detected");
        const major = parseInt(version.split(".")[0] ?? "0", 10);
        if (!Number.isFinite(major) || major < MIN_BUN_MAJOR) {
          throw new AIToolResolutionError(
            "BUN_TOO_OLD",
            `Detected Bun ${version}. Bun ${MIN_BUN_MAJOR}.0+ is required for bunx fallback execution.`,
            [
              "Upgrade Bun: curl -fsSL https://bun.sh/install | bash",
              "Verify with 'bun --version' (needs >= 1.0)",
            ],
          );
        }
      } catch (error: unknown) {
        if (error instanceof AIToolResolutionError) {
          throw error;
        }
        const err = error as NodeJS.ErrnoException;
        if (err?.code === "ENOENT") {
          throw new AIToolResolutionError(
            "BUNX_NOT_FOUND",
            "bun command not found while verifying bunx. Install Bun 1.0+ and ensure it is on PATH.",
            [
              "Install Bun: https://bun.sh/docs/installation",
              "After installation, run 'bun --version' to confirm.",
            ],
          );
        }
        throw new AIToolResolutionError(
          "BUN_TOO_OLD",
          `Failed to verify Bun version: ${err?.message ?? "unknown error"}`,
        );
      }
    })();
  }

  try {
    await bunxCheckPromise;
  } catch (error) {
    bunxCheckPromise = null;
    throw error;
  }
}

export interface ClaudeCommandOptions {
  mode?: "normal" | "continue" | "resume";
  skipPermissions?: boolean;
  extraArgs?: string[];
}

export function buildClaudeArgs(options: ClaudeCommandOptions = {}): string[] {
  const args: string[] = [];

  switch (options.mode) {
    case "continue":
      args.push("-c");
      break;
    case "resume":
      args.push("-r");
      break;
    default:
      break;
  }

  if (options.skipPermissions) {
    args.push(...CLAUDE_PERMISSION_SKIP_ARGS);
  }

  if (options.extraArgs?.length) {
    args.push(...options.extraArgs);
  }

  return args;
}

export async function resolveClaudeCommand(
  options: ClaudeCommandOptions = {},
): Promise<ResolvedCommand> {
  const args = buildClaudeArgs(options);
  const envOverrides = CLAUDE_CODE_TOOL.env
    ? { env: { ...CLAUDE_CODE_TOOL.env } as NodeJS.ProcessEnv }
    : {};

  // フルパスを取得（node-ptyはシェルを経由しないため必要）
  const claudePath = await resolveCommandPath("claude");
  if (claudePath) {
    logger.info(
      { command: claudePath, usesFallback: false },
      "Claude command resolved",
    );
    return {
      command: claudePath,
      args,
      usesFallback: false,
      ...envOverrides,
    };
  }

  // bunxへフォールバック
  const bunxPath = await resolveCommandPath("bunx");
  if (!bunxPath) {
    await ensureBunxAvailable(); // エラーをスローする
  }

  logger.info(
    { command: bunxPath ?? "bunx", usesFallback: true },
    "Claude command resolved (fallback)",
  );
  return {
    command: bunxPath ?? "bunx",
    args: [CLAUDE_CLI_PACKAGE, ...args],
    usesFallback: true,
    ...envOverrides,
  };
}

export interface CodexCommandOptions {
  mode?: "normal" | "continue" | "resume";
  bypassApprovals?: boolean;
  extraArgs?: string[];
}

export function buildCodexArgs(options: CodexCommandOptions = {}): string[] {
  const args: string[] = [];

  switch (options.mode) {
    case "continue":
      args.push("resume", "--last");
      break;
    case "resume":
      args.push("resume");
      break;
    default:
      break;
  }

  if (options.bypassApprovals) {
    args.push("--yolo");
  }

  if (options.extraArgs?.length) {
    args.push(...options.extraArgs);
  }

  args.push(...CODEX_DEFAULT_ARGS);
  return args;
}

export async function resolveCodexCommand(
  options: CodexCommandOptions = {},
): Promise<ResolvedCommand> {
  const args = buildCodexArgs(options);

  // フルパスを取得（node-ptyはシェルを経由しないため必要）
  const codexPath = await resolveCommandPath("codex");
  if (codexPath) {
    logger.info(
      { command: codexPath, usesFallback: false },
      "Codex command resolved",
    );
    return {
      command: codexPath,
      args,
      usesFallback: false,
    };
  }

  // bunxへフォールバック
  const bunxPath = await resolveCommandPath("bunx");
  if (!bunxPath) {
    await ensureBunxAvailable(); // エラーをスローする
  }

  logger.info(
    { command: bunxPath ?? "bunx", usesFallback: true },
    "Codex command resolved (fallback)",
  );
  return {
    command: bunxPath ?? "bunx",
    args: [CODEX_CLI_PACKAGE, ...args],
    usesFallback: true,
  };
}

export interface CustomToolCommandOptions extends LaunchOptions {
  toolId: string;
}

export async function resolveCustomToolCommand(
  options: CustomToolCommandOptions,
): Promise<ResolvedCommand> {
  const tool = await getToolById(options.toolId);
  if (!tool) {
    logger.error({ toolId: options.toolId }, "Custom tool not found");
    throw new AIToolResolutionError(
      "CUSTOM_TOOL_NOT_FOUND",
      `Custom tool not found: ${options.toolId}`,
      [
        "Update ~/.gwt/tools.json to include this ID",
        "Reload the Web UI after editing the tools list",
      ],
    );
  }

  const execution = await prepareCustomToolExecution(tool, options);

  logger.info(
    { toolId: options.toolId, command: execution.command },
    "Custom tool command resolved",
  );

  return {
    command: execution.command,
    args: execution.args,
    usesFallback: tool.type === "bunx",
    ...(execution.env ? { env: execution.env } : {}),
  };
}

export async function isClaudeCodeAvailable(): Promise<boolean> {
  try {
    await resolveClaudeCommand();
    return true;
  } catch (error) {
    if (error instanceof AIToolResolutionError) {
      return false;
    }
    return false;
  }
}

export async function isCodexAvailable(): Promise<boolean> {
  try {
    await resolveCodexCommand();
    return true;
  } catch (error) {
    if (error instanceof AIToolResolutionError) {
      return false;
    }
    return false;
  }
}

/**
 * Test-helper: resets cached bunx availability check.
 * Not exported in type definitions to avoid production usage.
 */
export function __resetBunxCacheForTests(): void {
  bunxCheckPromise = null;
}
