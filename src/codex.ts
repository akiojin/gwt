import { execa } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import {
  createChildStdio,
  getTerminalStreams,
  resetTerminalModes,
  writeTerminalLine,
} from "./utils/terminal.js";
import {
  findLatestCodexSession,
  waitForCodexSessionId,
} from "./utils/session.js";
import { findCommand } from "./utils/command.js";

const CODEX_CLI_PACKAGE = "@openai/codex";

/**
 * Reasoning effort levels supported by Codex CLI.
 */
export type CodexReasoningEffort = "low" | "medium" | "high" | "xhigh";

/**
 * Default Codex model used when no override is provided.
 */
export const DEFAULT_CODEX_MODEL = "gpt-5.2-codex";

/**
 * Default reasoning effort used when no override is provided.
 */
export const DEFAULT_CODEX_REASONING_EFFORT: CodexReasoningEffort = "high";

/**
 * Builds the default argument list for Codex CLI launch.
 *
 * @param model - Model name to pass via `--model`
 * @param reasoningEffort - Reasoning effort to pass via config
 */
export const buildDefaultCodexArgs = (
  model: string = DEFAULT_CODEX_MODEL,
  reasoningEffort: CodexReasoningEffort = DEFAULT_CODEX_REASONING_EFFORT,
): string[] => [
  "--enable",
  "web_search_request",
  "--enable",
  "skills",
  `--model=${model}`,
  "--sandbox",
  "workspace-write",
  "-c",
  `model_reasoning_effort=${reasoningEffort}`,
  "-c",
  "model_reasoning_summaries=detailed",
  "-c",
  "sandbox_workspace_write.network_access=true",
  "-c",
  "shell_environment_policy.inherit=all",
  "-c",
  "shell_environment_policy.ignore_default_excludes=true",
  "-c",
  "shell_environment_policy.experimental_use_profile=true",
];

/**
 * Error wrapper used by `launchCodexCLI` to preserve the original failure
 * while providing a user-friendly message.
 */
export class CodexError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "CodexError";
  }
}

/**
 * Launches Codex CLI in the given worktree path.
 *
 * This function resets terminal modes before and after the child process and
 * tries to detect a session id after launch (when supported).
 *
 * @param worktreePath - Worktree directory to run Codex CLI in
 * @param options - Launch options (mode/session/model/reasoning/env)
 * @returns Captured session id when available
 */
export async function launchCodexCLI(
  worktreePath: string,
  options: {
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
    bypassApprovals?: boolean;
    envOverrides?: Record<string, string>;
    model?: string;
    reasoningEffort?: CodexReasoningEffort;
    sessionId?: string | null;
    version?: string | null;
  } = {},
): Promise<{ sessionId?: string | null }> {
  const terminal = getTerminalStreams();
  const startedAt = Date.now();

  try {
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    writeTerminalLine(chalk.blue("🚀 Launching Codex CLI..."));
    writeTerminalLine(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];
    const model = options.model ?? DEFAULT_CODEX_MODEL;
    const reasoningEffort =
      options.reasoningEffort ?? DEFAULT_CODEX_REASONING_EFFORT;

    writeTerminalLine(chalk.green(`   🎯 Model: ${model}`));
    writeTerminalLine(chalk.green(`   🧠 Reasoning: ${reasoningEffort}`));

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
          args.push("resume", resumeSessionId);
          writeTerminalLine(
            chalk.cyan(
              `   ⏭️  Resuming specific Codex session: ${resumeSessionId}`,
            ),
          );
        } else {
          args.push("resume", "--last");
          writeTerminalLine(chalk.cyan("   ⏭️  Resuming last Codex session"));
        }
        break;
      case "resume":
        if (resumeSessionId) {
          args.push("resume", resumeSessionId);
          writeTerminalLine(
            chalk.cyan(`   🔄 Resuming Codex session: ${resumeSessionId}`),
          );
        } else {
          args.push("resume");
          writeTerminalLine(chalk.cyan("   🔄 Resume command"));
        }
        break;
      case "normal":
      default:
        writeTerminalLine(chalk.green("   ✨ Starting new session"));
        break;
    }

    if (options.bypassApprovals) {
      args.push("--yolo");
      writeTerminalLine(chalk.yellow("   ⚠️  Bypassing approvals and sandbox"));
    }

    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    const codexArgs = buildDefaultCodexArgs(model, reasoningEffort);

    args.push(...codexArgs);

    writeTerminalLine(chalk.gray(`   📋 Args: ${args.join(" ")}`));

    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);

    const childStdio = createChildStdio();

    const env = Object.fromEntries(
      Object.entries({
        ...process.env,
        ...(options.envOverrides ?? {}),
      }).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );

    // Auto-detect locally installed codex command
    const codexLookup = await findCommand("codex");

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

    // Determine execution strategy based on version selection
    // FR-063b: "installed" option only appears when local command exists
    const selectedVersion = options.version ?? "latest";

    // Log version information (FR-072)
    if (selectedVersion === "installed") {
      writeTerminalLine(chalk.green(`   📦 Version: installed`));
    } else {
      writeTerminalLine(chalk.green(`   📦 Version: @${selectedVersion}`));
    }

    try {
      if (selectedVersion === "installed" && codexLookup.path) {
        // FR-066: Use locally installed command when "installed" is selected
        // FR-063b guarantees local command exists when this option is shown
        writeTerminalLine(
          chalk.green("   ✨ Using locally installed codex command"),
        );
        const child = execa(codexLookup.path, args, {
          cwd: worktreePath,
          stdin: childStdio.stdin,
          stdout: childStdio.stdout,
          stderr: childStdio.stderr,
          env,
        });
        await execChild(child);
      } else {
        // FR-067, FR-068: Use bunx with version suffix for latest/specific versions
        const packageWithVersion = `${CODEX_CLI_PACKAGE}@${selectedVersion}`;
        writeTerminalLine(chalk.cyan(`   🔄 Using bunx ${packageWithVersion}`));

        const child = execa("bunx", [packageWithVersion, ...args], {
          cwd: worktreePath,
          stdin: childStdio.stdin,
          stdout: childStdio.stdout,
          stderr: childStdio.stderr,
          env,
        });
        await execChild(child);
      }
    } finally {
      childStdio.cleanup();
    }

    // File-based session detection only - no stdout capture
    // Use only file inspection with a short wait to avoid hanging
    let capturedSessionId: string | null = null;
    const finishedAt = Date.now();
    try {
      const latest = await findLatestCodexSession({
        since: startedAt,
        until: finishedAt + 30_000,
        preferClosestTo: finishedAt,
        windowMs: 10 * 60 * 1000,
        cwd: worktreePath,
      });
      const detectedSessionId = latest?.id ?? null;
      // When we explicitly resumed a specific session, keep that ID as the source of truth.
      capturedSessionId = usedExplicitSessionId
        ? resumeSessionId
        : detectedSessionId;
    } catch {
      capturedSessionId = usedExplicitSessionId ? resumeSessionId : null;
    }
    const shouldSkipWait =
      typeof process !== "undefined" &&
      (process.env?.NODE_ENV === "test" || Boolean(process.env?.VITEST));
    if (!capturedSessionId && !shouldSkipWait) {
      capturedSessionId = await waitForCodexSessionId({
        startedAt,
        timeoutMs: 15_000,
        pollIntervalMs: 1_000,
        cwd: worktreePath,
      });
    }

    if (capturedSessionId) {
      writeTerminalLine(chalk.cyan(`\n   🆔 Session ID: ${capturedSessionId}`));
      writeTerminalLine(
        chalk.gray(`   Resume command: codex resume ${capturedSessionId}`),
      );
    } else {
      writeTerminalLine(
        chalk.yellow(
          "\n   ℹ️  Could not determine Codex session ID automatically.",
        ),
      );
    }

    return capturedSessionId ? { sessionId: capturedSessionId } : {};
  } catch (error: unknown) {
    const err = error as NodeJS.ErrnoException;
    const details = error instanceof Error ? error.message : String(error);
    const errorMessage =
      err.code === "ENOENT"
        ? "bunx command not found. Please ensure Bun is installed so Codex CLI can run via bunx."
        : `Failed to launch Codex CLI: ${details || "Unknown error"}`;

    if (platform() === "win32") {
      writeTerminalLine(
        chalk.red("\n💡 Windows troubleshooting tips:"),
        "stderr",
      );
      writeTerminalLine(
        chalk.yellow(
          "   1. Confirm that Bun is installed and bunx is available",
        ),
        "stderr",
      );
      writeTerminalLine(
        chalk.yellow(
          '   2. Run "bunx @openai/codex@latest -- --help" to verify the setup',
        ),
        "stderr",
      );
      writeTerminalLine(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
        "stderr",
      );
    }

    throw new CodexError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
    resetTerminalModes(terminal.stdout);
  }
}

/**
 * Checks whether Codex CLI is available via `bunx` in the current environment.
 */
export async function isCodexAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [`${CODEX_CLI_PACKAGE}@latest`, "--help"]);
    return true;
  } catch (error: unknown) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === "ENOENT") {
      writeTerminalLine(chalk.yellow("\n⚠️  bunx command not found"), "stderr");
    }
    return false;
  }
}
