import { execa } from "execa";
import type { Options as ExecaOptions } from "execa";
import chalk from "chalk";
import { existsSync } from "fs";
import {
  createChildStdio,
  getTerminalStreams,
  resetTerminalModes,
} from "./utils/terminal.js";
import { findLatestClaudeSession } from "./utils/session.js";
import { CLAUDE_PERMISSION_SKIP_ARGS } from "./shared/aiToolConstants.js";
import {
  resolveClaudeCommand,
  AIToolResolutionError,
  type ResolvedCommand,
} from "./services/aiToolResolver.js";

export class ClaudeError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "ClaudeError";
  }
}

export async function launchClaudeCode(
  worktreePath: string,
  options: {
    skipPermissions?: boolean;
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
    envOverrides?: Record<string, string>;
    model?: string;
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

    console.log(chalk.blue("🚀 Launching Claude Code..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];

    if (options.model && options.model !== "opus") {
      args.push("--model", options.model);
      console.log(chalk.green(`   🎯 Model: ${options.model}`));
    } else if (options.model === "opus") {
      console.log(chalk.green(`   🎯 Model: ${options.model} (Default)`));
    }

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
          args.push("--resume", resumeSessionId);
          console.log(
            chalk.cyan(`   📱 Continuing specific session: ${resumeSessionId}`),
          );
        } else {
          console.log(
            chalk.yellow(
              "   ℹ️  No saved session ID for this branch/tool. Starting new session.",
            ),
          );
        }
        break;
      case "resume":
        console.log(
          chalk.yellow(
            "   ⚠️  Resume conversation feature temporarily disabled (Ink UI migration)",
          ),
        );
        console.log(
          chalk.cyan("   ℹ️  Using default Claude Code resume behavior"),
        );

        if (resumeSessionId) {
          args.push("--resume", resumeSessionId);
          console.log(
            chalk.cyan(`   🔄 Resuming Claude session: ${resumeSessionId}`),
          );
        } else {
          args.push("-r");
        }
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    let isRoot = false;
    try {
      isRoot = process.getuid ? process.getuid() === 0 : false;
    } catch {
      // process.getuid() not available (e.g., Windows) - default to false
    }

    if (options.skipPermissions) {
      args.push(...CLAUDE_PERMISSION_SKIP_ARGS);
      console.log(chalk.yellow("   ⚠️  Skipping permissions check"));

      if (isRoot) {
        console.log(
          chalk.yellow(
            "   ⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
          ),
        );
      }
    }

    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    console.log(chalk.gray(`   📋 Args: ${args.join(" ")}`));

    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);

    const baseEnv = { ...process.env, ...(options.envOverrides ?? {}) };
    const launchEnvSource =
      options.skipPermissions && !baseEnv.IS_SANDBOX
        ? { ...baseEnv, IS_SANDBOX: "1" }
        : baseEnv;
    const launchEnv = Object.fromEntries(
      Object.entries(launchEnvSource).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );

    const childStdio = createChildStdio();

    try {
      lastResolvedCommand = await resolveClaudeCommand({ args });

      if (lastResolvedCommand.usesFallback) {
        console.log(
          chalk.cyan(
            "   🔄 Falling back to bunx @anthropic-ai/claude-code@latest",
          ),
        );
        console.log(
          chalk.yellow(
            "   💡 Recommended: Install Claude Code via official method for faster startup",
          ),
        );
        console.log(
          chalk.yellow("      macOS/Linux: brew install --cask claude-code"),
        );
        console.log(
          chalk.yellow(
            "      or: curl -fsSL https://claude.ai/install.sh | bash",
          ),
        );
        console.log(
          chalk.yellow("      Windows: irm https://claude.ai/install.ps1 | iex"),
        );
        console.log("");
        await new Promise((resolve) => setTimeout(resolve, 2000));
      } else {
        console.log(chalk.green("   ✨ Using locally installed claude command"));
      }

      const execaOptions: ExecaOptions = {
        cwd: worktreePath,
        shell: true,
        stdin: childStdio.stdin as ExecaOptions["stdin"],
        stdout: childStdio.stdout as ExecaOptions["stdout"],
        stderr: childStdio.stderr as ExecaOptions["stderr"],
        env: launchEnv,
      };

      await execa(
        lastResolvedCommand.command,
        lastResolvedCommand.args,
        execaOptions,
      );
    } finally {
      childStdio.cleanup();
    }

    let capturedSessionId: string | null = null;
    const finishedAt = Date.now();
    try {
      const latest = await findLatestClaudeSession(worktreePath, {
        since: startedAt,
        until: finishedAt + 30_000,
        preferClosestTo: finishedAt,
        windowMs: 10 * 60 * 1000,
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
        chalk.gray(`   Resume command: claude --resume ${capturedSessionId}`),
      );
    } else {
      console.log(
        chalk.yellow("\n   ℹ️  Could not determine Claude session ID automatically."),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: unknown) {
    if (error instanceof AIToolResolutionError) {
      throw new ClaudeError(error.message, error);
    }

    let errorMessage: string;
    const errorWithCode = error as NodeJS.ErrnoException;

    if (errorWithCode?.code === "ENOENT") {
      errorMessage = lastResolvedCommand?.usesFallback
        ? "bunx command not found. Please ensure Bun is installed so Claude Code can run via bunx."
        : "claude command not found. Please ensure Claude Code is properly installed.";
    } else {
      const fallbackMessage =
        error instanceof Error ? error.message : "Unknown error";
      errorMessage = `Failed to launch Claude Code: ${fallbackMessage}`;
    }

    if (process.platform === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      if (lastResolvedCommand && !lastResolvedCommand.usesFallback) {
        console.error(
          chalk.yellow(
            "   1. Confirm that Claude Code is installed and the 'claude' command is on PATH",
          ),
        );
        console.error(
          chalk.yellow('   2. Run "claude --version" to verify the setup'),
        );
      } else {
        console.error(
          chalk.yellow("   1. Confirm that Bun is installed and bunx is available"),
        );
        console.error(
          chalk.yellow(
            '   2. Run "bunx @anthropic-ai/claude-code@latest -- --version" to verify the setup',
          ),
        );
      }
      console.error(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
      );
    }

    throw new ClaudeError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);
  }
}

export { isClaudeCodeAvailable } from "./services/aiToolResolver.js";
