import { execa } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";
import {
  resolveCodexCommand,
  AIToolResolutionError,
  type ResolvedCommand,
} from "./services/aiToolResolver.js";

export class CodexError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "CodexError";
  }
}

export async function launchCodexCLI(
  worktreePath: string,
  options: {
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
    bypassApprovals?: boolean;
    envOverrides?: Record<string, string>;
  } = {},
): Promise<void> {
  const terminal = getTerminalStreams();
  let lastResolvedCommand: ResolvedCommand | null = null;

  try {
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue("🚀 Launching Codex CLI..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    switch (options.mode) {
      case "continue":
        console.log(chalk.cyan("   ⏭️  Resuming last Codex session"));
        break;
      case "resume":
        console.log(chalk.cyan("   🔄 Resume command"));
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    if (options.bypassApprovals) {
      console.log(chalk.yellow("   ⚠️  Bypassing approvals and sandbox"));
    }

    terminal.exitRawMode();

    const childStdio = createChildStdio();

    const env = { ...process.env, ...(options.envOverrides ?? {}) };

    try {
      const resolverOptions: {
        mode?: "normal" | "continue" | "resume";
        bypassApprovals?: boolean;
        extraArgs?: string[];
      } = {};

      if (options.mode) {
        resolverOptions.mode = options.mode;
      }
      if (options.bypassApprovals !== undefined) {
        resolverOptions.bypassApprovals = options.bypassApprovals;
      }
      if (options.extraArgs && options.extraArgs.length > 0) {
        resolverOptions.extraArgs = options.extraArgs;
      }

      lastResolvedCommand = await resolveCodexCommand(resolverOptions);

      if (!lastResolvedCommand.usesFallback) {
        console.log(chalk.green("   ✨ Using locally installed codex command"));
      }

      const env = lastResolvedCommand.env
        ? { ...lastResolvedCommand.env }
        : { ...process.env };

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
      throw new CodexError(error.message, error);
    }

    const errorMessage =
      error.code === "ENOENT"
        ? lastResolvedCommand?.usesFallback === false
          ? "codex command not found. Please ensure Codex CLI is installed."
          : "bunx command not found. Please ensure Bun is installed so Codex CLI can run via bunx."
        : `Failed to launch Codex CLI: ${error.message || "Unknown error"}`;

    if (platform() === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      if (lastResolvedCommand && !lastResolvedCommand.usesFallback) {
        console.error(
          chalk.yellow(
            "   1. Confirm that Codex CLI is installed and available on PATH",
          ),
        );
        console.error(
          chalk.yellow('   2. Run "codex --help" to verify the setup'),
        );
      } else {
        console.error(
          chalk.yellow(
            "   1. Confirm that Bun is installed and bunx is available",
          ),
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
  }
}

export { isCodexAvailable } from "./services/aiToolResolver.js";
