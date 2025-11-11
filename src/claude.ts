import { execa } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";
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
  } = {},
): Promise<void> {
  const terminal = getTerminalStreams();
  let lastResolvedCommand: ResolvedCommand | null = null;

  try {
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue("🚀 Launching Claude Code..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    switch (options.mode) {
      case "continue":
        console.log(chalk.cyan("   📱 Continuing most recent conversation"));
        break;
      case "resume":
        console.log(chalk.cyan("   🔄 Resuming previous Claude Code session"));
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    let isRoot = false;
    try {
      isRoot = typeof process.getuid === "function" && process.getuid() === 0;
    } catch {
      isRoot = false;
    }

    if (options.skipPermissions) {
      console.log(chalk.yellow("   ⚠️  Skipping permissions check"));
      if (isRoot) {
        console.log(
          chalk.yellow(
            "   ⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
          ),
        );
      }
    }

    const resolverOptions: {
      mode?: "normal" | "continue" | "resume";
      skipPermissions?: boolean;
      extraArgs?: string[];
    } = {};

    if (options.mode) {
      resolverOptions.mode = options.mode;
    }
    if (options.skipPermissions !== undefined) {
      resolverOptions.skipPermissions = options.skipPermissions;
    }
    if (options.extraArgs && options.extraArgs.length > 0) {
      resolverOptions.extraArgs = options.extraArgs;
    }

    terminal.exitRawMode();
    const childStdio = createChildStdio();

    try {
      lastResolvedCommand = await resolveClaudeCommand(resolverOptions);

      if (!lastResolvedCommand.usesFallback) {
        console.log(
          chalk.green("   ✨ Using locally installed claude command"),
        );
      } else {
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
      }

      const env = lastResolvedCommand.env
        ? { ...lastResolvedCommand.env }
        : { ...process.env };
      if (isRoot && options.skipPermissions) {
        env.IS_SANDBOX = "1";
      }

      await execa(lastResolvedCommand.command, lastResolvedCommand.args, {
        cwd: worktreePath,
        stdin: childStdio.stdin,
        stdout: childStdio.stdout,
        stderr: childStdio.stderr,
        env,
      } as any);
    } finally {
      childStdio.cleanup();
    }
  } catch (error: any) {
    if (error instanceof AIToolResolutionError) {
      const hints = error.hints?.length ? `\n${error.hints.join("\n")}` : "";
      throw new ClaudeError(`${error.message}${hints}`, error);
    }

    const errorMessage =
      error.code === "ENOENT"
        ? lastResolvedCommand?.usesFallback === false
          ? "claude command not found. Please ensure Claude Code is installed."
          : "bunx command not found. Please ensure Bun is installed so Claude CLI can run via bunx."
        : `Failed to launch Claude Code: ${error.message || "Unknown error"}`;

    if (platform() === "win32") {
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
          chalk.yellow(
            "   1. Confirm that Bun is installed and bunx is available",
          ),
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
  }
}

export { isClaudeCodeAvailable } from "./services/aiToolResolver.js";
