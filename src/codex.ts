import { execa } from "execa";
import type { Options as ExecaOptions } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";
import {
  resolveCodexCommand,
  AIToolResolutionError,
  type ResolvedCommand,
  type CodexCommandOptions,
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

    try {
      const resolverOptions: CodexCommandOptions = {};
      if (options.mode) {
        resolverOptions.mode = options.mode;
      }
      if (typeof options.bypassApprovals !== "undefined") {
        resolverOptions.bypassApprovals = options.bypassApprovals;
      }
      if (options.extraArgs && options.extraArgs.length > 0) {
        resolverOptions.extraArgs = options.extraArgs;
      }

      lastResolvedCommand = await resolveCodexCommand(resolverOptions);

      if (!lastResolvedCommand.usesFallback) {
        console.log(chalk.green("   ✨ Using locally installed codex command"));
      }

      const execaOptions: ExecaOptions = {
        cwd: worktreePath,
        stdin: childStdio.stdin as ExecaOptions["stdin"],
        stdout: childStdio.stdout as ExecaOptions["stdout"],
        stderr: childStdio.stderr as ExecaOptions["stderr"],
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
  }
}

export { isCodexAvailable } from "./services/aiToolResolver.js";
