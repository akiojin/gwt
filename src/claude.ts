import { execa, type Options as ExecaOptions } from "execa";
import chalk from "chalk";
import { existsSync } from "fs";
import {
  createChildStdio,
  getTerminalStreams,
  resetTerminalModes,
} from "./utils/terminal.js";
import { isCommandAvailable } from "./utils/command.js";
import { findLatestClaudeSession } from "./utils/session.js";

const CLAUDE_CLI_PACKAGE = "@anthropic-ai/claude-code@latest";

/**
 * Error wrapper used by `launchClaudeCode` to preserve the original failure
 * while providing a user-friendly message.
 */
export class ClaudeError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "ClaudeError";
  }
}

/**
 * Launches Claude Code in the given worktree path.
 *
 * This function:
 * - validates the worktree path
 * - normalizes launch arguments (mode/model/session/extra args)
 * - resets terminal modes before and after the child process
 * - auto-detects a local `claude` command and falls back to `npx` (Windows) or
 *   `bunx` when needed
 *
 * @param worktreePath - Worktree directory to run Claude Code in
 * @param options - Launch options (mode/session/model/permissions/env)
 * @param options.chrome - Enable Chrome extension integration (adds --chrome flag)
 * @returns Captured session id when available
 */
export async function launchClaudeCode(
  worktreePath: string,
  options: {
    skipPermissions?: boolean;
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
    envOverrides?: Record<string, string>;
    model?: string;
    sessionId?: string | null;
    chrome?: boolean;
  } = {},
): Promise<{ sessionId?: string | null }> {
  const terminal = getTerminalStreams();
  const startedAt = Date.now();

  try {
    // Check if the worktree path exists
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

    // Handle execution mode
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
        // TODO: Implement conversation selection with Ink UI
        // Legacy UI removed - this feature needs to be reimplemented
        console.log(
          chalk.yellow(
            "   ⚠️  Resume conversation feature temporarily disabled (Ink UI migration)",
          ),
        );
        console.log(
          chalk.cyan("   ℹ️  Using default Claude Code resume behavior"),
        );

        // Fallback to default Claude Code resume
        /*
        try {
          const { selectClaudeConversation } = await import("./ui/legacy/prompts.js");
          const selectedConversation =
            await selectClaudeConversation(worktreePath);

          if (selectedConversation) {
            console.log(
              chalk.green(`   ✨ Resuming: ${selectedConversation.title}`),
            );

            // Use specific session ID if available
            if (selectedConversation.sessionId) {
              args.push("--resume", selectedConversation.sessionId);
              console.log(
                chalk.cyan(
                  `   🆔 Using session ID: ${selectedConversation.sessionId}`,
                ),
              );
            } else {
              // Fallback: try to use filename as session identifier
              const fileName = selectedConversation.id;
              console.log(
                chalk.yellow(
                  `   ⚠️  No session ID found, trying filename: ${fileName}`,
                ),
              );
              args.push("--resume", fileName);
            }
          } else {
            // User cancelled - return without launching Claude
            console.log(
              chalk.gray("   ↩️  Selection cancelled, returning to menu"),
            );
            return;
          }
        } catch (error) {
          console.warn(
            chalk.yellow(
              "   ⚠️  Failed to load conversation history, using standard resume",
            ),
          );
          args.push("-r");
        }
        */
        // Use standard Claude Code resume for now
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

    // Handle Chrome extension integration
    if (options.chrome) {
      args.push("--chrome");
      console.log(chalk.green("   🌐 Chrome integration enabled"));
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
      args.push("--dangerously-skip-permissions");
      console.log(chalk.yellow("   ⚠️  Skipping permissions check"));

      // Show additional warning for root users in Docker/sandbox environments
      if (isRoot) {
        console.log(
          chalk.yellow(
            "   ⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
          ),
        );
      }
    }
    // Append any pass-through arguments after our flags
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

    // Auto-detect locally installed claude command
    const hasLocalClaude = await isClaudeCommandAvailable();
    const hasNpx =
      process.platform === "win32" ? await isNpxCommandAvailable() : false;

    const execInteractive = async (
      file: string,
      fileArgs: string[],
      execOptions: Omit<ExecaOptions, "shell">,
    ) => {
      if (process.platform !== "win32") {
        await execa(file, fileArgs, { ...execOptions, shell: true });
        return;
      }

      try {
        await execa(file, fileArgs, { ...execOptions, shell: false });
        return;
      } catch (error: unknown) {
        const err = error as NodeJS.ErrnoException;
        if (err?.code === "ENOENT" || err?.code === "EINVAL") {
          await execa(file, fileArgs, { ...execOptions, shell: true });
          return;
        }
        throw error;
      }
    };

    try {
      if (hasLocalClaude) {
        console.log(
          chalk.green("   ✨ Using locally installed claude command"),
        );
        await execInteractive("claude", args, {
          cwd: worktreePath,
          stdin: childStdio.stdin,
          stdout: childStdio.stdout,
          stderr: childStdio.stderr,
          env: launchEnv,
        });
      } else {
        if (hasNpx) {
          console.log(
            chalk.cyan(
              "   🔄 Falling back to npx @anthropic-ai/claude-code@latest",
            ),
          );
        } else {
          console.log(
            chalk.cyan(
              "   🔄 Falling back to bunx @anthropic-ai/claude-code@latest",
            ),
          );
        }
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
        if (hasNpx) {
          await execInteractive("npx", ["-y", CLAUDE_CLI_PACKAGE, ...args], {
            cwd: worktreePath,
            stdin: childStdio.stdin,
            stdout: childStdio.stdout,
            stderr: childStdio.stderr,
            env: launchEnv,
          });
        } else {
          await execInteractive("bunx", [CLAUDE_CLI_PACKAGE, ...args], {
            cwd: worktreePath,
            stdin: childStdio.stdin,
            stdout: childStdio.stdout,
            stderr: childStdio.stderr,
            env: launchEnv,
          });
        }
      }
    } finally {
      childStdio.cleanup();
    }

    // File-based session detection only - no stdout capture
    // Use only findLatestClaudeSession with short timeout, skip sessionProbe to avoid hanging
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
      // When we explicitly resumed a specific session, keep that ID as the source of truth.
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
        chalk.yellow(
          "\n   ℹ️  Could not determine Claude session ID automatically.",
        ),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: unknown) {
    const hasLocalClaude = await isClaudeCommandAvailable();
    let errorMessage: string;
    const err = error as NodeJS.ErrnoException;

    if (err.code === "ENOENT") {
      if (hasLocalClaude) {
        errorMessage =
          "claude command not found. Please ensure Claude Code is properly installed.";
      } else {
        errorMessage =
          "bunx command not found. Please ensure Bun is installed so Claude Code can run via bunx.";
      }
    } else {
      const details = error instanceof Error ? error.message : String(error);
      errorMessage = `Failed to launch Claude Code: ${details || "Unknown error"}`;
    }

    if (process.platform === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      if (hasLocalClaude) {
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
    resetTerminalModes(terminal.stdout);
  }
}

async function isClaudeCommandAvailable(): Promise<boolean> {
  return isCommandAvailable("claude");
}

async function isNpxCommandAvailable(): Promise<boolean> {
  return isCommandAvailable("npx");
}

/**
 * Checks whether Claude Code is available via `bunx` in the current environment.
 *
 * @returns true if Claude Code can be resolved via bunx, false otherwise.
 */
export async function isClaudeCodeAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [CLAUDE_CLI_PACKAGE, "--version"], { shell: true });
    return true;
  } catch (error: unknown) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === "ENOENT") {
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
