import { execa, type Options as ExecaOptions } from "execa";
import chalk from "chalk";
import { existsSync, readFileSync } from "fs";
import {
  createChildStdio,
  getTerminalStreams,
  resetTerminalModes,
  writeTerminalLine,
} from "./utils/terminal.js";
import { findCommand } from "./utils/command.js";
import { findLatestClaudeSession } from "./utils/session.js";
import {
  runAgentWithPty,
  shouldCaptureAgentOutput,
} from "./logging/agentOutput.js";

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
 * - auto-detects a local `claude` command and falls back to `bunx` when needed
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
    version?: string | null;
  } = {},
): Promise<{ sessionId?: string | null }> {
  const terminal = getTerminalStreams();
  const startedAt = Date.now();

  try {
    // Check if the worktree path exists
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    writeTerminalLine(chalk.blue("🚀 Launching Claude Code..."));
    writeTerminalLine(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];

    if (options.model && options.model !== "opus") {
      args.push("--model", options.model);
      writeTerminalLine(chalk.green(`   🎯 Model: ${options.model}`));
    } else if (options.model === "opus") {
      writeTerminalLine(chalk.green(`   🎯 Model: ${options.model} (Default)`));
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
          writeTerminalLine(
            chalk.cyan(`   📱 Continuing specific session: ${resumeSessionId}`),
          );
        } else {
          writeTerminalLine(
            chalk.yellow(
              "   ℹ️  No saved session ID for this branch/tool. Starting new session.",
            ),
          );
        }
        break;
      case "resume":
        // TODO: Implement conversation selection in the OpenTUI UI
        // Legacy UI removed - this feature needs to be reimplemented
        writeTerminalLine(
          chalk.yellow(
            "   ⚠️  Resume conversation feature temporarily disabled (UI migration)",
          ),
        );
        writeTerminalLine(
          chalk.cyan("   ℹ️  Using default Claude Code resume behavior"),
        );

        // Fallback to default Claude Code resume
        /*
        try {
          const { selectClaudeConversation } = await import("./ui/legacy/prompts.js");
          const selectedConversation =
            await selectClaudeConversation(worktreePath);

          if (selectedConversation) {
            writeTerminalLine(
              chalk.green(`   ✨ Resuming: ${selectedConversation.title}`),
            );

            // Use specific session ID if available
            if (selectedConversation.sessionId) {
              args.push("--resume", selectedConversation.sessionId);
              writeTerminalLine(
                chalk.cyan(
                  `   🆔 Using session ID: ${selectedConversation.sessionId}`,
                ),
              );
            } else {
              // Fallback: try to use filename as session identifier
              const fileName = selectedConversation.id;
              writeTerminalLine(
                chalk.yellow(
                  `   ⚠️  No session ID found, trying filename: ${fileName}`,
                ),
              );
              args.push("--resume", fileName);
            }
          } else {
            // User cancelled - return without launching Claude
            writeTerminalLine(
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
          writeTerminalLine(
            chalk.cyan(`   🔄 Resuming Claude session: ${resumeSessionId}`),
          );
        } else {
          args.push("-r");
        }
        break;
      case "normal":
      default:
        writeTerminalLine(chalk.green("   ✨ Starting new session"));
        break;
    }

    // Handle Chrome extension integration
    if (options.chrome && isChromeIntegrationSupported()) {
      args.push("--chrome");
      writeTerminalLine(chalk.green("   🌐 Chrome integration enabled"));
    } else if (options.chrome) {
      writeTerminalLine(
        chalk.yellow(
          "   ⚠️  Chrome integration is not supported on this platform. Skipping --chrome.",
        ),
      );
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
      writeTerminalLine(chalk.yellow("   ⚠️  Skipping permissions check"));

      // Show additional warning for root users in Docker/sandbox environments
      if (isRoot) {
        writeTerminalLine(
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

    writeTerminalLine(chalk.gray(`   📋 Args: ${args.join(" ")}`));

    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);

    const baseEnv: Record<string, string | undefined> = {
      ...process.env,
      ...(options.envOverrides ?? {}),
      ENABLE_LSP_TOOL: "true", // Enable TypeScript LSP support in Claude Code
    };
    const launchEnvSource =
      options.skipPermissions && !baseEnv.IS_SANDBOX
        ? { ...baseEnv, IS_SANDBOX: "1" }
        : baseEnv;
    const launchEnv = Object.fromEntries(
      Object.entries(launchEnvSource).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );

    const captureOutput = shouldCaptureAgentOutput(launchEnv);
    const childStdio = captureOutput ? null : createChildStdio();

    // Auto-detect locally installed claude command
    const claudeLookup = await findCommand("claude");
    const npxLookup =
      process.platform === "win32" ? await findCommand("npx") : null;

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

    // Treat SIGHUP (1), SIGINT (2), SIGTERM (15) as normal exit signals
    // SIGHUP can occur when the PTY closes, SIGINT/SIGTERM are user interrupts
    const isNormalExitSignal = (signal?: number | null) =>
      signal === 1 || signal === 2 || signal === 15;

    const runCommand = async (file: string, fileArgs: string[]) => {
      if (captureOutput) {
        const result = await runAgentWithPty({
          command: file,
          args: fileArgs,
          cwd: worktreePath,
          env: launchEnv,
          agentId: "claude-code",
        });
        // Treat normal exit signals (SIGHUP, SIGINT, SIGTERM) as successful exit
        if (isNormalExitSignal(result.signal)) {
          return;
        }
        if (result.exitCode !== null && result.exitCode !== 0) {
          throw new Error(`Claude Code exited with code ${result.exitCode}`);
        }
        return;
      }

      if (!childStdio) {
        return;
      }

      await execInteractive(file, fileArgs, {
        cwd: worktreePath,
        stdin: childStdio.stdin,
        stdout: childStdio.stdout,
        stderr: childStdio.stderr,
        env: launchEnv,
      });
    };

    // Determine execution strategy based on version selection
    // FR-063b: "installed" option only appears when local command exists
    const requestedVersion = options.version ?? "latest";
    let selectedVersion = requestedVersion;

    if (requestedVersion === "installed" && !claudeLookup.path) {
      writeTerminalLine(
        chalk.yellow(
          "   ⚠️  Installed claude command not found. Falling back to latest.",
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

    try {
      if (selectedVersion === "installed" && claudeLookup.path) {
        // FR-066: Use locally installed command when "installed" is selected
        // FR-063b guarantees local command exists when this option is shown
        writeTerminalLine(
          chalk.green("   ✨ Using locally installed claude command"),
        );
        await runCommand(claudeLookup.path, args);
      } else {
        // FR-067, FR-068: Use bunx with version suffix for latest/specific versions
        const packageWithVersion = `@anthropic-ai/claude-code@${selectedVersion}`;

        const useNpx = npxLookup?.source === "installed" && npxLookup?.path;
        if (useNpx) {
          writeTerminalLine(
            chalk.cyan(`   🔄 Using npx ${packageWithVersion}`),
          );
        } else {
          writeTerminalLine(
            chalk.cyan(`   🔄 Using bunx ${packageWithVersion}`),
          );
        }

        if (useNpx && npxLookup?.path) {
          await runCommand(npxLookup.path, ["-y", packageWithVersion, ...args]);
        } else {
          await runCommand("bunx", [packageWithVersion, ...args]);
        }
      }
    } finally {
      childStdio?.cleanup();
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
      writeTerminalLine(chalk.cyan(`\n   🆔 Session ID: ${capturedSessionId}`));
      writeTerminalLine(
        chalk.gray(`   Resume command: claude --resume ${capturedSessionId}`),
      );
    } else {
      writeTerminalLine(
        chalk.yellow(
          "\n   ℹ️  Could not determine Claude session ID automatically.",
        ),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: unknown) {
    const claudeCheck = await findCommand("claude");
    const hasLocalClaude =
      claudeCheck.source === "installed" && claudeCheck.path !== null;
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
      writeTerminalLine(
        chalk.red("\n💡 Windows troubleshooting tips:"),
        "stderr",
      );
      if (hasLocalClaude) {
        writeTerminalLine(
          chalk.yellow(
            "   1. Confirm that Claude Code is installed and the 'claude' command is on PATH",
          ),
          "stderr",
        );
        writeTerminalLine(
          chalk.yellow('   2. Run "claude --version" to verify the setup'),
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
            '   2. Run "bunx @anthropic-ai/claude-code@latest -- --version" to verify the setup',
          ),
          "stderr",
        );
      }
      writeTerminalLine(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
        "stderr",
      );
    }

    throw new ClaudeError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);
  }
}

/**
 * Checks whether Claude Code is available via `bunx` in the current environment.
 *
 * @returns true if Claude Code can be resolved via bunx, false otherwise.
 */
export async function isClaudeCodeAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [CLAUDE_CLI_PACKAGE, "--version"]);
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

/**
 * Checks whether Chrome integration is supported on the current platform.
 *
 * Supported platforms:
 * - Windows (win32)
 * - macOS (darwin)
 * - Linux (non-WSL)
 */
function isChromeIntegrationSupported(): boolean {
  switch (process.platform) {
    case "win32":
    case "darwin":
      return true;
    case "linux":
      return !isWslEnvironment();
    default:
      return false;
  }
}

function isWslEnvironment(): boolean {
  if (process.platform !== "linux") {
    return false;
  }

  if (process.env.WSL_DISTRO_NAME || process.env.WSL_INTEROP) {
    return true;
  }

  try {
    const procVersion = readFileSync("/proc/version", "utf8");
    return /microsoft|wsl/i.test(procVersion);
  } catch {
    return false;
  }
}
