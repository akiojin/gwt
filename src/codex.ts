import { execa } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";
import { findLatestCodexSession } from "./utils/session.js";

const CODEX_CLI_PACKAGE = "@openai/codex@latest";

export type CodexReasoningEffort = "low" | "medium" | "high" | "xhigh";

export const DEFAULT_CODEX_MODEL = "gpt-5.1-codex";
export const DEFAULT_CODEX_REASONING_EFFORT: CodexReasoningEffort = "high";

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
    model?: string;
    reasoningEffort?: CodexReasoningEffort;
    sessionId?: string | null;
  } = {},
): Promise<{ sessionId?: string | null }> {
  const terminal = getTerminalStreams();
  const startedAt = Date.now();

  try {
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue("🚀 Launching Codex CLI..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];
    const model = options.model ?? DEFAULT_CODEX_MODEL;
    const reasoningEffort =
      options.reasoningEffort ?? DEFAULT_CODEX_REASONING_EFFORT;

    console.log(chalk.green(`   🎯 Model: ${model}`));
    console.log(chalk.green(`   🧠 Reasoning: ${reasoningEffort}`));

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
          console.log(
            chalk.cyan(
              `   ⏭️  Resuming specific Codex session: ${resumeSessionId}`,
            ),
          );
        } else {
          args.push("resume", "--last");
          console.log(chalk.cyan("   ⏭️  Resuming last Codex session"));
        }
        break;
      case "resume":
        if (resumeSessionId) {
          args.push("resume", resumeSessionId);
          console.log(
            chalk.cyan(`   🔄 Resuming Codex session: ${resumeSessionId}`),
          );
        } else {
          args.push("resume");
          console.log(chalk.cyan("   🔄 Resume command"));
        }
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
    }

    if (options.bypassApprovals) {
      args.push("--yolo");
      console.log(chalk.yellow("   ⚠️  Bypassing approvals and sandbox"));
    }

    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    const codexArgs = buildDefaultCodexArgs(model, reasoningEffort);

    args.push(...codexArgs);

    console.log(chalk.gray(`   📋 Args: ${args.join(" ")}`));

    terminal.exitRawMode();

    const childStdio = createChildStdio();

    const env = Object.fromEntries(
      Object.entries({
        ...process.env,
        ...(options.envOverrides ?? {}),
      }).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );

    try {
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

      const child = execa("bunx", [CODEX_CLI_PACKAGE, ...args], {
        cwd: worktreePath,
        shell: true,
        stdin: childStdio.stdin,
        stdout: childStdio.stdout,
        stderr: childStdio.stderr,
        env,
      });
      await execChild(child);
    } finally {
      childStdio.cleanup();
    }

    // File-based session detection only - no stdout capture
    // Use only findLatestCodexSession with short timeout, skip sessionProbe to avoid hanging
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

    if (capturedSessionId) {
      console.log(chalk.cyan(`\n   🆔 Session ID: ${capturedSessionId}`));
      console.log(
        chalk.gray(`   Resume command: codex resume ${capturedSessionId}`),
      );
    } else {
      console.log(
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
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
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
      console.error(
        chalk.yellow("   3. Restart your terminal or IDE to refresh PATH"),
      );
    }

    throw new CodexError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
  }
}

export async function isCodexAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [CODEX_CLI_PACKAGE, "--help"]);
    return true;
  } catch (error: unknown) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === "ENOENT") {
      console.error(chalk.yellow("\n⚠️  bunx command not found"));
    }
    return false;
  }
}
