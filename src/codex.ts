import { execa } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { createChildStdio, getTerminalStreams } from "./utils/terminal.js";

const CODEX_CLI_PACKAGE = "@openai/codex@latest";
const DEFAULT_CODEX_ARGS = [
  "--enable",
  "web_search_request",
  "--model=gpt-5-codex",
  "--sandbox",
  "workspace-write",
  "-c",
  "model_reasoning_effort=high",
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

    args.push(...DEFAULT_CODEX_ARGS);

    terminal.exitRawMode();

    const childStdio = createChildStdio();

    try {
      await execa("bunx", [CODEX_CLI_PACKAGE, ...args], {
        cwd: worktreePath,
        stdin: childStdio.stdin,
        stdout: childStdio.stdout,
        stderr: childStdio.stderr,
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
        chalk.yellow("   1. Bun がインストールされ bunx が利用可能か確認"),
      );
      console.error(
        chalk.yellow(
          '   2. "bunx @openai/codex@latest -- --help" を実行してセットアップを確認',
        ),
      );
      console.error(
        chalk.yellow("   3. ターミナルやIDEを再起動して PATH を更新"),
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
      console.error(chalk.yellow("\n⚠️  bunx コマンドが見つかりません"));
    }
    return false;
  }
}
