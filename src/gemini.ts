import { execa } from "execa";
import chalk from "chalk";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";
import { findLatestGeminiSessionId } from "./utils/session.js";

const GEMINI_CLI_PACKAGE = "@google/gemini-cli@latest";

export class GeminiError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "GeminiError";
  }
}

export async function launchGeminiCLI(
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

    console.log(chalk.blue("🚀 Launching Gemini CLI..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];

    if (options.model) {
      args.push("--model", options.model);
      console.log(chalk.green(`   🎯 Model: ${options.model}`));
    }

    // Handle execution mode
    const resumeSessionId =
      options.sessionId && options.sessionId.trim().length > 0
        ? options.sessionId.trim()
        : null;

    switch (options.mode) {
      case "continue":
        if (resumeSessionId) {
          args.push("--resume", resumeSessionId);
          console.log(
            chalk.cyan(
              `   ⏭️  Continuing specific session: ${resumeSessionId}`,
            ),
          );
        } else {
          args.push("--resume");
          console.log(chalk.cyan("   ⏭️  Continuing most recent session"));
        }
        break;
      case "resume":
        if (resumeSessionId) {
          args.push("--resume", resumeSessionId);
          console.log(chalk.cyan(`   🔄 Resuming session: ${resumeSessionId}`));
        } else {
          args.push("--resume");
          console.log(chalk.cyan("   🔄 Resuming session (latest)"));
        }
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    // Handle skip permissions (YOLO mode)
    if (options.skipPermissions) {
      args.push("-y");
      console.log(
        chalk.yellow("   ⚠️  Auto-approving all actions (YOLO mode)"),
      );
    }

    // Append any pass-through arguments after our flags
    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    terminal.exitRawMode();

    const baseEnv = {
      ...process.env,
      ...(options.envOverrides ?? {}),
    };

    const childStdio = createChildStdio();

    // Auto-detect locally installed gemini command
    const hasLocalGemini = await isGeminiCommandAvailable();

    try {
      if (hasLocalGemini) {
        // Use locally installed gemini command
        console.log(
          chalk.green("   ✨ Using locally installed gemini command"),
        );
        await execa("gemini", args, {
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
          chalk.cyan("   🔄 Falling back to bunx @google/gemini-cli@latest"),
        );
        console.log(
          chalk.yellow(
            "   💡 Recommended: Install Gemini CLI globally for faster startup",
          ),
        );
        console.log(chalk.yellow("      npm install -g @google/gemini-cli"));
        console.log("");
        // Wait 2 seconds to let user read the message
        await new Promise((resolve) => setTimeout(resolve, 2000));
        await execa("bunx", [GEMINI_CLI_PACKAGE, ...args], {
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
        (await findLatestGeminiSessionId(worktreePath)) ??
        resumeSessionId ??
        null;
    } catch {
      capturedSessionId = resumeSessionId ?? null;
    }

    if (capturedSessionId) {
      console.log(chalk.cyan(`\n   🆔 Session ID: ${capturedSessionId}`));
      console.log(
        chalk.gray(`   Resume command: gemini --resume ${capturedSessionId}`),
      );
    } else {
      console.log(
        chalk.yellow(
          "\n   ℹ️  Could not determine Gemini session ID automatically.",
        ),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: any) {
    const hasLocalGemini = await isGeminiCommandAvailable();
    let errorMessage: string;

    if (error.code === "ENOENT") {
      if (hasLocalGemini) {
        errorMessage =
          "gemini command not found. Please ensure Gemini CLI is properly installed.";
      } else {
        errorMessage =
          "bunx command not found. Please ensure Bun is installed so Gemini CLI can run via bunx.";
      }
    } else {
      errorMessage = `Failed to launch Gemini CLI: ${error.message || "Unknown error"}`;
    }

    if (process.platform === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      if (hasLocalGemini) {
        console.error(
          chalk.yellow(
            "   1. Confirm that Gemini CLI is installed and the 'gemini' command is on PATH",
          ),
        );
        console.error(
          chalk.yellow('   2. Run "gemini --version" to verify the setup'),
        );
      } else {
        console.error(
          chalk.yellow(
            "   1. Confirm that Bun is installed and bunx is available",
          ),
        );
        console.error(
          chalk.yellow(
            '   2. Run "bunx @google/gemini-cli@latest -- --version" to verify the setup',
          ),
        );
      }
      console.error(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
      );
    }

    throw new GeminiError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
  }
}

/**
 * Check if locally installed `gemini` command is available
 * @returns true if `gemini` command exists in PATH, false otherwise
 */
async function isGeminiCommandAvailable(): Promise<boolean> {
  try {
    const command = process.platform === "win32" ? "where" : "which";
    await execa(command, ["gemini"], { shell: true });
    return true;
  } catch {
    // gemini command not found in PATH
    return false;
  }
}

export async function isGeminiCLIAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [GEMINI_CLI_PACKAGE, "--version"], { shell: true });
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
