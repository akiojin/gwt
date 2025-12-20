import { execa } from "execa";
import type { Options as ExecaOptions } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import {
  createChildStdio,
  getTerminalStreams,
  resetTerminalModes,
} from "./utils/terminal.js";
import { findLatestCodexSession } from "./utils/session.js";
import {
  resolveCodexCommand,
  AIToolResolutionError,
  type ResolvedCommand,
} from "./services/aiToolResolver.js";

/**
 * Reasoning effort levels supported by Codex CLI.
 */
export type CodexReasoningEffort = "low" | "medium" | "high" | "xhigh";

/**
 * Default Codex model used when no override is provided.
 */
export const DEFAULT_CODEX_MODEL = "gpt-5.2-codex";

/**
 * Default reasoning effort used when no override is provided.
 */
export const DEFAULT_CODEX_REASONING_EFFORT: CodexReasoningEffort = "high";

/**
 * Builds the default argument list for Codex CLI launch.
 *
 * @param model - Model name to pass via `--model`
 * @param reasoningEffort - Reasoning effort to pass via config
 */
export const buildDefaultCodexArgs = (
  model: string = DEFAULT_CODEX_MODEL,
  reasoningEffort: CodexReasoningEffort = DEFAULT_CODEX_REASONING_EFFORT,
): string[] => [
  "--enable",
  "web_search_request",
  "--enable",
  "skills",
  `--model=${model}`,
  "--sandbox",
  "workspace-write",
  "-c",
  `model_reasoning_effort=${reasoningEffort}`,
  "-c",
  "model_reasoning_summaries=detailed",
  "-c",
  "sandbox_workspace_write.network_access=true",
  "-c",
  "shell_environment_policy.inherit=all",
  "-c",
  "shell_environment_policy.ignore_default_excludes=true",
  "-c",
  "shell_environment_policy.experimental_use_profile=true",
];

/**
 * Error wrapper used by `launchCodexCLI` to preserve the original failure
 * while providing a user-friendly message.
 */
export class CodexError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "CodexError";
  }
}

/**
 * Launches Codex CLI in the given worktree path.
 *
 * This function resets terminal modes before and after the child process and
 * tries to detect a session id after launch (when supported).
 *
 * @param worktreePath - Worktree directory to run Codex CLI in
 * @param options - Launch options (mode/session/model/reasoning/env)
 * @returns Captured session id when available
 */
export async function launchCodexCLI(
  worktreePath: string,
  options: {
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
    bypassApprovals?: boolean;
    envOverrides?: Record<string, string>;
    model?: string;
    reasoningEffort?: CodexReasoningEffort;
    sessionId?: string | null;
  } = {},
): Promise<{ sessionId?: string | null }> {
  const terminal = getTerminalStreams();
  const startedAt = Date.now();
  let lastResolvedCommand: ResolvedCommand | null = null;

  try {
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue("🚀 Launching Codex CLI..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];
    const model = options.model ?? DEFAULT_CODEX_MODEL;
    const reasoningEffort =
      options.reasoningEffort ?? DEFAULT_CODEX_REASONING_EFFORT;

    console.log(chalk.green(`   🎯 Model: ${model}`));
    console.log(chalk.green(`   🧠 Reasoning: ${reasoningEffort}`));

    const resumeSessionId =
      options.sessionId && options.sessionId.trim().length > 0
        ? options.sessionId.trim()
        : null;
    const usedExplicitSessionId =
      Boolean(resumeSessionId) &&
      (options.mode === "continue" || options.mode === "resume");

    switch (options.mode) {
      case "continue":
        if (resumeSessionId) {
          args.push("resume", resumeSessionId);
          console.log(
            chalk.cyan(
              `   ⏭️  Resuming specific Codex session: ${resumeSessionId}`,
            ),
          );
        } else {
          args.push("resume", "--last");
          console.log(chalk.cyan("   ⏭️  Resuming last Codex session"));
        }
        break;
      case "resume":
        if (resumeSessionId) {
          args.push("resume", resumeSessionId);
          console.log(
            chalk.cyan(`   🔄 Resuming Codex session: ${resumeSessionId}`),
          );
        } else {
          args.push("resume");
          console.log(chalk.cyan("   🔄 Resume command"));
        }
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    if (options.bypassApprovals) {
      args.push("--yolo");
      console.log(chalk.yellow("   ⚠️  Bypassing approvals and sandbox"));
    }

    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    args.push(...buildDefaultCodexArgs(model, reasoningEffort));

    console.log(chalk.gray(`   📋 Args: ${args.join(" ")}`));

    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);

    const env = Object.fromEntries(
      Object.entries({
        ...process.env,
        ...(options.envOverrides ?? {}),
      }).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );

    const childStdio = createChildStdio();

    try {
      lastResolvedCommand = await resolveCodexCommand({ args });

      if (lastResolvedCommand.usesFallback) {
        console.log(chalk.cyan("   🔄 Falling back to bunx @openai/codex@latest"));
      } else {
        console.log(chalk.green("   ✨ Using locally installed codex command"));
      }

      const execaOptions: ExecaOptions = {
        cwd: worktreePath,
        stdin: childStdio.stdin as ExecaOptions["stdin"],
        stdout: childStdio.stdout as ExecaOptions["stdout"],
        stderr: childStdio.stderr as ExecaOptions["stderr"],
        env,
      };

      try {
        await execa(
          lastResolvedCommand.command,
          lastResolvedCommand.args,
          execaOptions,
        );
      } catch (execError: unknown) {
        const signal = (execError as { signal?: unknown })?.signal;
        if (signal === "SIGINT" || signal === "SIGTERM") {
          return {};
        }
        throw execError;
      }
    } finally {
      childStdio.cleanup();
    }

    let capturedSessionId: string | null = null;
    const finishedAt = Date.now();
    try {
      const latest = await findLatestCodexSession({
        since: startedAt,
        until: finishedAt + 30_000,
        preferClosestTo: finishedAt,
        windowMs: 10 * 60 * 1000,
        cwd: worktreePath,
      });
      const detectedSessionId = latest?.id ?? null;
      capturedSessionId = usedExplicitSessionId
        ? resumeSessionId
        : detectedSessionId;
    } catch {
      capturedSessionId = usedExplicitSessionId ? resumeSessionId : null;
    }

    if (capturedSessionId) {
      console.log(chalk.cyan(`\n   🆔 Session ID: ${capturedSessionId}`));
      console.log(
        chalk.gray(`   Resume command: codex resume ${capturedSessionId}`),
      );
    } else {
      console.log(
        chalk.yellow("\n   ℹ️  Could not determine Codex session ID automatically."),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: unknown) {
    if (error instanceof AIToolResolutionError) {
      throw new CodexError(error.message, error);
    }

    const errorMessage =
      (error as NodeJS.ErrnoException)?.code === "ENOENT"
        ? lastResolvedCommand?.usesFallback === false
          ? "codex command not found. Please ensure Codex CLI is installed."
          : "bunx command not found. Please ensure Bun is installed so Codex CLI can run via bunx."
        : `Failed to launch Codex CLI: ${
            error instanceof Error ? error.message : "Unknown error"
          }`;

    if (platform() === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      if (lastResolvedCommand && !lastResolvedCommand.usesFallback) {
        console.error(
          chalk.yellow(
            "   1. Confirm that Codex CLI is installed and the 'codex' command is on PATH",
          ),
        );
        console.error(
          chalk.yellow('   2. Run "codex --help" to verify the setup'),
        );
      } else {
        console.error(
          chalk.yellow("   1. Confirm that Bun is installed and bunx is available"),
        );
        console.error(
          chalk.yellow(
            '   2. Run "bunx @openai/codex@latest -- --help" to verify the setup',
          ),
        );
      }
      console.error(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
      );
    }

    throw new CodexError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);
  }
}

export { isCodexAvailable } from "./services/aiToolResolver.js";
