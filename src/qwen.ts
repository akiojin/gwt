import { execa } from "execa";
import chalk from "chalk";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";
import { findLatestQwenSessionId } from "./utils/session.js";

const QWEN_CLI_PACKAGE = "@qwen-code/qwen-code@latest";

export class QwenError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "QwenError";
  }
}

export async function launchQwenCLI(
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

  try {
    // Check if the worktree path exists
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue("🚀 Launching Qwen CLI..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = ["--checkpointing"];

    if (options.model) {
      args.push("--model", options.model);
      console.log(chalk.green(`   🎯 Model: ${options.model}`));
    }

    // Handle execution mode
    // Note: Qwen CLI doesn't have explicit continue/resume CLI options at startup.
    // Session management is done via /chat commands during interactive sessions.
    const resumeSessionId =
      options.sessionId && options.sessionId.trim().length > 0
        ? options.sessionId.trim()
        : null;
    switch (options.mode) {
      case "continue":
        console.log(
          chalk.cyan(
            resumeSessionId
              ? `   ⏭️  Starting session (then /chat resume ${resumeSessionId})`
              : "   ⏭️  Starting session (use /chat resume in the CLI to continue)",
          ),
        );
        break;
      case "resume":
        console.log(
          chalk.cyan(
            resumeSessionId
              ? `   🔄 Starting session (then /chat resume ${resumeSessionId})`
              : "   🔄 Starting session (use /chat resume in the CLI to continue)",
          ),
        );
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    // Handle skip permissions (YOLO mode)
    if (options.skipPermissions) {
      args.push("--yolo");
      console.log(
        chalk.yellow("   ⚠️  Auto-approving all actions (YOLO mode)"),
      );
    }

    // Append any pass-through arguments after our flags
    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    console.log(chalk.gray(`   📋 Args: ${args.join(" ")}`));

    terminal.exitRawMode();

    const baseEnv = {
      ...process.env,
      ...(options.envOverrides ?? {}),
    };

    const childStdio = createChildStdio();

    // Auto-detect locally installed qwen command
    const hasLocalQwen = await isQwenCommandAvailable();

    try {
      if (hasLocalQwen) {
        // Use locally installed qwen command
        console.log(chalk.green("   ✨ Using locally installed qwen command"));
        await execa("qwen", args, {
          cwd: worktreePath,
          shell: true,
          stdin: childStdio.stdin,
          stdout: childStdio.stdout,
          stderr: childStdio.stderr,
          env: baseEnv,
        } as any);
      } else {
        // Fallback to bunx
        console.log(
          chalk.cyan("   🔄 Falling back to bunx @qwen-code/qwen-code@latest"),
        );
        console.log(
          chalk.yellow(
            "   💡 Recommended: Install Qwen CLI globally for faster startup",
          ),
        );
        console.log(chalk.yellow("      npm install -g @qwen-code/qwen-code"));
        console.log("");
        // Wait 2 seconds to let user read the message
        await new Promise((resolve) => setTimeout(resolve, 2000));
        await execa("bunx", [QWEN_CLI_PACKAGE, ...args], {
          cwd: worktreePath,
          shell: true,
          stdin: childStdio.stdin,
          stdout: childStdio.stdout,
          stderr: childStdio.stderr,
          env: baseEnv,
        } as any);
      }
    } finally {
      childStdio.cleanup();
    }

    let capturedSessionId: string | null = null;
    try {
      capturedSessionId =
        (await findLatestQwenSessionId(worktreePath)) ??
        resumeSessionId ??
        null;
    } catch {
      capturedSessionId = resumeSessionId ?? null;
    }

    if (capturedSessionId) {
      console.log(chalk.cyan(`\n   🆔 Session tag: ${capturedSessionId}`));
      console.log(
        chalk.gray(`   Resume in Qwen CLI: /chat resume ${capturedSessionId}`),
      );
    } else {
      console.log(
        chalk.yellow(
          "\n   ℹ️  Could not determine Qwen session tag automatically.",
        ),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: any) {
    const hasLocalQwen = await isQwenCommandAvailable();
    let errorMessage: string;

    if (error.code === "ENOENT") {
      if (hasLocalQwen) {
        errorMessage =
          "qwen command not found. Please ensure Qwen CLI is properly installed.";
      } else {
        errorMessage =
          "bunx command not found. Please ensure Bun is installed so Qwen CLI can run via bunx.";
      }
    } else {
      errorMessage = `Failed to launch Qwen CLI: ${error.message || "Unknown error"}`;
    }

    if (process.platform === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      if (hasLocalQwen) {
        console.error(
          chalk.yellow(
            "   1. Confirm that Qwen CLI is installed and the 'qwen' command is on PATH",
          ),
        );
        console.error(
          chalk.yellow('   2. Run "qwen --version" to verify the setup'),
        );
      } else {
        console.error(
          chalk.yellow(
            "   1. Confirm that Bun is installed and bunx is available",
          ),
        );
        console.error(
          chalk.yellow(
            '   2. Run "bunx @qwen-code/qwen-code@latest -- --version" to verify the setup',
          ),
        );
      }
      console.error(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
      );
    }

    throw new QwenError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
  }
}

/**
 * Check if locally installed `qwen` command is available
 * @returns true if `qwen` command exists in PATH, false otherwise
 */
async function isQwenCommandAvailable(): Promise<boolean> {
  try {
    const command = process.platform === "win32" ? "where" : "which";
    await execa(command, ["qwen"], { shell: true });
    return true;
  } catch {
    // qwen command not found in PATH
    return false;
  }
}

export async function isQwenCLIAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [QWEN_CLI_PACKAGE, "--version"], { shell: true });
    return true;
  } catch (error: any) {
    if (error.code === "ENOENT") {
      console.error(chalk.yellow("\n⚠️  bunx command not found"));
      console.error(
        chalk.gray(
          "   Install Bun and confirm that bunx is available before continuing",
        ),
      );
    }
    return false;
  }
}
