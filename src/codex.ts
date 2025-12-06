import { execa } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";

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
  } = {},
): Promise<void> {
  const terminal = getTerminalStreams();

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

    switch (options.mode) {
      case "continue":
        args.push("resume", "--last");
        console.log(chalk.cyan("   ⏭️  Resuming last Codex session"));
        break;
      case "resume":
        args.push("resume");
        console.log(chalk.cyan("   🔄 Resume command"));
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

    const env = { ...process.env, ...(options.envOverrides ?? {}) };

    try {
      await execa("bunx", [CODEX_CLI_PACKAGE, ...args], {
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
    const errorMessage =
      error.code === "ENOENT"
        ? "bunx command not found. Please ensure Bun is installed so Codex CLI can run via bunx."
        : `Failed to launch Codex CLI: ${error.message || "Unknown error"}`;

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
  }
}

export async function isCodexAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [CODEX_CLI_PACKAGE, "--help"]);
    return true;
  } catch (error: any) {
    if (error.code === "ENOENT") {
      console.error(chalk.yellow("\n⚠️  bunx command not found"));
    }
    return false;
  }
}
