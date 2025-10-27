import { execa } from "execa";
import chalk from "chalk";
import { platform } from "os";
import { existsSync } from "fs";
import { getTerminalStreams } from "./utils/terminal.js";

const CLAUDE_CLI_PACKAGE = "@anthropic-ai/claude-code@latest";
export class ClaudeError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "ClaudeError";
  }
}

export async function launchClaudeCode(
  worktreePath: string,
  options: {
    skipPermissions?: boolean;
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
  } = {},
): Promise<void> {
  const terminal = getTerminalStreams();

  try {
    // Check if the worktree path exists
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue("🚀 Launching Claude Code..."));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];

    // Handle execution mode
    switch (options.mode) {
      case "continue":
        args.push("-c");
        console.log(chalk.cyan("   📱 Continuing most recent conversation"));
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
        args.push("-r");
        break;
      case "normal":
      default:
        console.log(chalk.green("   ✨ Starting new session"));
        break;
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
            "   ⚠️  Docker/サンドボックス環境として実行中（IS_SANDBOX=1）",
          ),
        );
      }
    }
    // Append any pass-through arguments after our flags
    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    terminal.exitRawMode();

    await execa("bunx", [CLAUDE_CLI_PACKAGE, ...args], {
      cwd: worktreePath,
      shell: true,
      stdin: terminal.stdin,
      stdout: terminal.stdout,
      stderr: terminal.stderr,
      env:
        isRoot && options.skipPermissions
          ? { ...process.env, IS_SANDBOX: "1" }
          : process.env,
    });
  } catch (error: any) {
    const errorMessage =
      error.code === "ENOENT"
        ? "bunx command not found. Please ensure Bun is installed so Claude Code can run via bunx."
        : `Failed to launch Claude Code: ${error.message || "Unknown error"}`;

    if (platform() === "win32") {
      console.error(chalk.red("\n💡 Windows troubleshooting tips:"));
      console.error(
        chalk.yellow("   1. Bun がインストールされ bunx が利用可能か確認"),
      );
      console.error(
        chalk.yellow(
          '   2. "bunx @anthropic-ai/claude-code@latest -- --version" を実行してセットアップを確認',
        ),
      );
      console.error(
        chalk.yellow("   3. ターミナルやIDEを再起動して PATH を更新"),
      );
    }

    throw new ClaudeError(errorMessage, error);
  } finally {
    terminal.exitRawMode();
  }
}

export async function isClaudeCodeAvailable(): Promise<boolean> {
  try {
    await execa("bunx", [CLAUDE_CLI_PACKAGE, "--version"], { shell: true });
    return true;
  } catch (error: any) {
    if (error.code === "ENOENT") {
      console.error(chalk.yellow("\n⚠️  bunx コマンドが見つかりません"));
      console.error(
        chalk.gray(
          "   Bun をインストールして bunx が使用可能か確認してください",
        ),
      );
    }
    return false;
  }
}
