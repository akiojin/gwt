import { execa } from "execa";
import chalk from "chalk";
import { existsSync } from "fs";
import {
  createChildStdio,
  getTerminalStreams,
  resetTerminalModes,
  writeTerminalLine,
} from "./utils/terminal.js";
import { findCommand } from "./utils/command.js";
import { findLatestGeminiSessionId } from "./utils/session.js";
import {
  runAgentWithPty,
  shouldCaptureAgentOutput,
} from "./logging/agentOutput.js";

const GEMINI_CLI_PACKAGE = "@google/gemini-cli";

/**
 * Error wrapper used by `launchGeminiCLI` to preserve the original failure
 * while providing a user-friendly message.
 */
export class GeminiError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "GeminiError";
  }
}

/**
 * Launches Gemini CLI in the given worktree path.
 *
 * This function resets terminal modes before and after the child process and
 * supports continue/resume modes when a session id is available.
 *
 * @param worktreePath - Worktree directory to run Gemini CLI in
 * @param options - Launch options (mode/session/model/permissions/env)
 * @returns Captured session id when available
 */
export async function launchGeminiCLI(
  worktreePath: string,
  options: {
    skipPermissions?: boolean;
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
    envOverrides?: Record<string, string>;
    model?: string;
    sessionId?: string | null;
    branch?: string | null;
    version?: string | null;
  } = {},
): Promise<{ sessionId?: string | null }> {
  const terminal = getTerminalStreams();

  try {
    // Check if the worktree path exists
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    writeTerminalLine(chalk.blue("🚀 Launching Gemini CLI..."));
    writeTerminalLine(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];

    if (options.model) {
      args.push("--model", options.model);
      writeTerminalLine(chalk.green(`   🎯 Model: ${options.model}`));
    }

    // Handle execution mode
    const resumeSessionId =
      options.sessionId && options.sessionId.trim().length > 0
        ? options.sessionId.trim()
        : null;
    const usedExplicitSessionId =
      Boolean(resumeSessionId) &&
      (options.mode === "continue" || options.mode === "resume");

    const buildArgs = (useResumeId: boolean) => {
      const a: string[] = [];
      if (options.model) {
        a.push("--model", options.model);
      }

      switch (options.mode) {
        case "continue":
        case "resume":
          if (useResumeId && resumeSessionId) {
            a.push("--resume", resumeSessionId);
          } else {
            a.push("--resume");
          }
          break;
        case "normal":
        default:
          break;
      }

      if (options.skipPermissions) {
        a.push("-y");
      }
      if (options.extraArgs && options.extraArgs.length > 0) {
        a.push(...options.extraArgs);
      }
      return a;
    };

    const argsPrimary = buildArgs(true);
    const argsFallback = buildArgs(false);

    // Log selected mode/ID
    switch (options.mode) {
      case "continue":
        if (resumeSessionId) {
          writeTerminalLine(
            chalk.cyan(
              `   ⏭️  Continuing specific session: ${resumeSessionId}`,
            ),
          );
        } else {
          writeTerminalLine(
            chalk.cyan("   ⏭️  Continuing most recent session"),
          );
        }
        break;
      case "resume":
        if (resumeSessionId) {
          writeTerminalLine(
            chalk.cyan(`   🔄 Resuming session: ${resumeSessionId}`),
          );
        } else {
          writeTerminalLine(chalk.cyan("   🔄 Resuming session (latest)"));
        }
        break;
      case "normal":
      default:
        writeTerminalLine(chalk.green("   ✨ Starting new session"));
        break;
    }

    // Handle skip permissions (YOLO mode)
    if (options.skipPermissions) {
      writeTerminalLine(
        chalk.yellow("   ⚠️  Auto-approving all actions (YOLO mode)"),
      );
    }
    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);

    const baseEnv = Object.fromEntries(
      Object.entries({
        ...process.env,
        ...(options.envOverrides ?? {}),
      }).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );
    const captureOutput = shouldCaptureAgentOutput(baseEnv);
    const childStdio = captureOutput ? null : createChildStdio();

    // Auto-detect locally installed gemini command
    const geminiLookup = await findCommand("gemini");

    // Preserve TTY for interactive UI (colors/width) by inheriting stdout/stderr.
    // Session ID is determined via file-based detection after exit.
    let capturedSessionId: string | null = null;

    // Determine execution strategy based on version selection
    // FR-063b: "installed" option only appears when local command exists
    const requestedVersion = options.version ?? "latest";
    let selectedVersion = requestedVersion;

    if (requestedVersion === "installed" && !geminiLookup.path) {
      writeTerminalLine(
        chalk.yellow(
          "   ⚠️  Installed gemini command not found. Falling back to latest.",
        ),
      );
      selectedVersion = "latest";
    }

    // Log version information (FR-072)
    if (selectedVersion === "installed") {
      writeTerminalLine(chalk.green(`   📦 Version: installed`));
    } else {
      writeTerminalLine(chalk.green(`   📦 Version: @${selectedVersion}`));
    }

    const runGemini = async (runArgs: string[]): Promise<void> => {
      const execChild = async (child: Promise<unknown>) => {
        try {
          await child;
        } catch (execError: unknown) {
          // Treat SIGINT/SIGTERM as normal exit (user pressed Ctrl+C)
          const signal = (execError as { signal?: unknown })?.signal;
          if (signal === "SIGINT" || signal === "SIGTERM") {
            return;
          }
          throw execError;
        }
      };
      // Treat SIGHUP (1), SIGINT (2), SIGTERM (15) as normal exit signals
      // SIGHUP can occur when the PTY closes, SIGINT/SIGTERM are user interrupts
      const isNormalExitSignal = (signal?: number | null) =>
        signal === 1 || signal === 2 || signal === 15;

      const run = async (cmd: string, args: string[]) => {
        if (captureOutput) {
          const result = await runAgentWithPty({
            command: cmd,
            args,
            cwd: worktreePath,
            env: baseEnv,
            agentId: "gemini-cli",
          });
          if (isNormalExitSignal(result.signal)) {
            return;
          }
          if (result.exitCode !== null && result.exitCode !== 0) {
            throw new Error(
              `Gemini CLI exited with code ${result.exitCode ?? "unknown"}`,
            );
          }
          return;
        }

        if (!childStdio) {
          return;
        }

        const child = execa(cmd, args, {
          cwd: worktreePath,
          stdin: childStdio.stdin,
          stdout: childStdio.stdout,
          stderr: childStdio.stderr,
          env: baseEnv,
        });
        await execChild(child);
      };

      if (selectedVersion === "installed" && geminiLookup.path) {
        // FR-066: Use locally installed command when "installed" is selected
        // FR-063b guarantees local command exists when this option is shown
        writeTerminalLine(
          chalk.green("   ✨ Using locally installed gemini command"),
        );
        return await run(geminiLookup.path, runArgs);
      }

      // FR-067, FR-068: Use bunx with version suffix for latest/specific versions
      const packageWithVersion = `${GEMINI_CLI_PACKAGE}@${selectedVersion}`;
      writeTerminalLine(chalk.cyan(`   🔄 Using bunx ${packageWithVersion}`));
      return await run("bunx", [packageWithVersion, ...runArgs]);
    };

    let fellBackToLatest = false;
    try {
      // Try with explicit session ID first (if any), then fallback to --resume (latest) once
      try {
        await runGemini(argsPrimary);
      } catch (err) {
        const shouldRetry =
          (options.mode === "resume" || options.mode === "continue") &&
          resumeSessionId;
        if (shouldRetry) {
          fellBackToLatest = true;
          writeTerminalLine(
            chalk.yellow(
              `   ⚠️  Failed to resume session ${resumeSessionId}. Retrying with latest session...`,
            ),
          );
          await runGemini(argsFallback);
        } else {
          throw err;
        }
      }
    } finally {
      childStdio?.cleanup();
    }

    const explicitResumeSucceeded = usedExplicitSessionId && !fellBackToLatest;

    // If we explicitly resumed a specific session (and did not fall back), keep that ID.
    if (explicitResumeSucceeded) {
      capturedSessionId = resumeSessionId;
    }

    // Fallback to file-based detection if stdout capture failed (and we don't have an explicit-resume ID)
    if (!capturedSessionId) {
      try {
        capturedSessionId =
          (await findLatestGeminiSessionId(worktreePath, {
            cwd: worktreePath,
            branch: options.branch ?? null,
            worktrees: options.branch
              ? [{ path: worktreePath, branch: options.branch }]
              : null,
          })) ?? null;
      } catch {
        capturedSessionId = null;
      }
    }

    if (capturedSessionId) {
      writeTerminalLine(chalk.cyan(`\n   🆔 Session ID: ${capturedSessionId}`));
      writeTerminalLine(
        chalk.gray(`   Resume command: gemini --resume ${capturedSessionId}`),
      );
    } else {
      writeTerminalLine(
        chalk.yellow(
          "\n   ℹ️  Could not determine Gemini session ID automatically.",
        ),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: unknown) {
    const geminiCheck = await findCommand("gemini");
    const hasLocalGemini =
      geminiCheck.source === "installed" && geminiCheck.path !== null;
    let errorMessage: string;
    const err = error as NodeJS.ErrnoException;

    if (err.code === "ENOENT") {
      if (hasLocalGemini) {
        errorMessage =
          "gemini command not found. Please ensure Gemini CLI is properly installed.";
      } else {
        errorMessage =
          "bunx command not found. Please ensure Bun is installed so Gemini CLI can run via bunx.";
      }
    } else {
      const details = error instanceof Error ? error.message : String(error);
      errorMessage = `Failed to launch Gemini CLI: ${details || "Unknown error"}`;
    }

    if (process.platform === "win32") {
      writeTerminalLine(
        chalk.red("\n💡 Windows troubleshooting tips:"),
        "stderr",
      );
      if (hasLocalGemini) {
        writeTerminalLine(
          chalk.yellow(
            "   1. Confirm that Gemini CLI is installed and the 'gemini' command is on PATH",
          ),
          "stderr",
        );
        writeTerminalLine(
          chalk.yellow('   2. Run "gemini --version" to verify the setup'),
          "stderr",
        );
      } else {
        writeTerminalLine(
          chalk.yellow(
            "   1. Confirm that Bun is installed and bunx is available",
          ),
          "stderr",
        );
        writeTerminalLine(
          chalk.yellow(
            '   2. Run "bunx @google/gemini-cli@latest -- --version" to verify the setup',
          ),
          "stderr",
        );
      }
      writeTerminalLine(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
        "stderr",
      );
    }

    throw new GeminiError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);
  }
}

/**
 * Checks whether Gemini CLI is available via `bunx` in the current environment.
 */
export async function isGeminiCLIAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [`${GEMINI_CLI_PACKAGE}@latest`, "--version"]);
    return true;
  } catch (error: unknown) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === "ENOENT") {
      writeTerminalLine(chalk.yellow("\n⚠️  bunx command not found"), "stderr");
      writeTerminalLine(
        chalk.gray(
          "   Install Bun and confirm that bunx is available before continuing",
        ),
        "stderr",
      );
    }
    return false;
  }
}
