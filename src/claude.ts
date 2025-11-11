import { execa } from "execa";
import type { Options as ExecaOptions } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";
import {
  resolveClaudeCommand,
  AIToolResolutionError,
  type ResolvedCommand,
  type ClaudeCommandOptions,
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
  } = {},
): Promise<void> {
  const terminal = getTerminalStreams();
  let lastResolvedCommand: ResolvedCommand | null = null;

  try {
    // Check if the worktree path exists
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue("🚀 Launching Claude Code..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    // Handle execution mode (logging only; args are built in resolver)
    switch (options.mode) {
      case "continue":
        console.log(chalk.cyan("   📱 Continuing most recent conversation"));
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
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    // Detect root user for Docker/sandbox environments
    let isRoot = false;
    try {
      isRoot = process.getuid ? process.getuid() === 0 : false;
    } catch {
      // process.getuid() not available (e.g., Windows) - default to false
    }

    // Handle skip permissions
    if (options.skipPermissions) {
      console.log(chalk.yellow("   ⚠️  Skipping permissions check"));

      // Show additional warning for root users in Docker/sandbox environments
      if (isRoot) {
        console.log(
          chalk.yellow(
            "   ⚠️  Docker/サンドボックス環境として実行中（IS_SANDBOX=1）",
          ),
        );
      }
    }

    terminal.exitRawMode();

    const childStdio = createChildStdio();

    try {
      const resolverOptions: ClaudeCommandOptions = {};
      if (options.mode) {
        resolverOptions.mode = options.mode;
      }
      if (typeof options.skipPermissions !== "undefined") {
        resolverOptions.skipPermissions = options.skipPermissions;
      }
      if (options.extraArgs && options.extraArgs.length > 0) {
        resolverOptions.extraArgs = options.extraArgs;
      }

      lastResolvedCommand = await resolveClaudeCommand(resolverOptions);

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
          chalk.yellow(
            "      Windows: irm https://claude.ai/install.ps1 | iex",
          ),
        );
        console.log("");
        await new Promise((resolve) => setTimeout(resolve, 2000));
      } else {
        console.log(
          chalk.green("   ✨ Using locally installed claude command"),
        );
      }

      const envConfig =
        isRoot && options.skipPermissions
          ? { ...process.env, IS_SANDBOX: "1" }
          : process.env;

      const execaOptions: ExecaOptions = {
        cwd: worktreePath,
        shell: true,
        stdin: childStdio.stdin as ExecaOptions["stdin"],
        stdout: childStdio.stdout as ExecaOptions["stdout"],
        stderr: childStdio.stderr as ExecaOptions["stderr"],
        env: envConfig,
      };

      await execa(
        lastResolvedCommand.command,
        lastResolvedCommand.args,
        execaOptions,
      );
    } finally {
      childStdio.cleanup();
    }
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

    if (platform() === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      if (lastResolvedCommand && !lastResolvedCommand.usesFallback) {
        console.error(
          chalk.yellow(
            "   1. Claude Code がインストールされ claude コマンドが利用可能か確認",
          ),
        );
        console.error(
          chalk.yellow('   2. "claude --version" を実行してセットアップを確認'),
        );
      } else {
        console.error(
          chalk.yellow("   1. Bun がインストールされ bunx が利用可能か確認"),
        );
        console.error(
          chalk.yellow(
            '   2. "bunx @anthropic-ai/claude-code@latest -- --version" を実行してセットアップを確認',
          ),
        );
      }
      console.error(
        chalk.yellow("   3. ターミナルやIDEを再起動して PATH を更新"),
      );
    }

    throw new ClaudeError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
  }
}

export { isClaudeCodeAvailable } from "./services/aiToolResolver.js";
