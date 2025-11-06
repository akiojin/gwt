#!/usr/bin/env node

import {
  isGitRepository,
  getRepositoryRoot,
  branchExists,
  fetchAllRemotes,
  pullFastForward,
  getBranchDivergenceStatuses,
} from "./git.js";
import { launchClaudeCode } from "./claude.js";
import { launchCodexCLI } from "./codex.js";
import {
  WorktreeOrchestrator,
  type EnsureWorktreeOptions,
} from "./services/WorktreeOrchestrator.js";
import chalk from "chalk";
import type { SelectionResult } from "./ui/components/App.js";
import { worktreeExists } from "./worktree.js";
import {
  getTerminalStreams,
  waitForUserAcknowledgement,
} from "./utils/terminal.js";
import { getToolById } from "./config/tools.js";
import { launchCustomAITool } from "./launcher.js";
import { saveSession } from "./config/index.js";
import { getPackageVersion } from "./utils.js";
import readline from "node:readline";

const ERROR_PROMPT = chalk.yellow(
  "エラー内容を確認したら Enter キーを押してください。",
);

async function waitForErrorAcknowledgement(): Promise<void> {
  await waitForUserAcknowledgement(ERROR_PROMPT);
}

/**
 * Simple print functions (replacing legacy UI display functions)
 */
function printError(message: string): void {
  console.error(chalk.red(`❌ ${message}`));
}

function printInfo(message: string): void {
  console.log(chalk.blue(`ℹ️  ${message}`));
}

function printWarning(message: string): void {
  console.warn(chalk.yellow(`⚠️  ${message}`));
}

async function waitForEnter(promptMessage: string): Promise<void> {
  if (!process.stdin.isTTY) {
    return;
  }

  await new Promise<void>((resolve) => {
    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout,
    });

    rl.question(`${promptMessage}\n`, () => {
      rl.close();
      resolve();
    });
  });
}

function showHelp(): void {
  console.log(`
Worktree Manager

Usage: claude-worktree [options]

Options:
  -h, --help      Show this help message
  -v, --version   Show version information

Description:
  Interactive Git worktree manager with AI tool selection (Claude Code / Codex CLI) and graphical branch selection.
  Launch without additional options to open the interactive menu.
`);
}

/**
 * Display application version
 * Reads version from package.json and outputs to stdout
 * Exits with code 1 if version cannot be retrieved
 */
async function showVersion(): Promise<void> {
  const version = await getPackageVersion();
  if (version) {
    console.log(version);
  } else {
    console.error("Error: Unable to retrieve version information");
    await waitForErrorAcknowledgement();
    process.exit(1);
  }
}

/**
 * Main function for Ink.js UI
 * Returns SelectionResult if user made selections, undefined if user quit
 */
async function mainInkUI(): Promise<SelectionResult | undefined> {
  const { render } = await import("ink");
  const React = await import("react");
  const { App } = await import("./ui/components/App.js");
  const terminal = getTerminalStreams();

  let selectionResult: SelectionResult | undefined;

  if (typeof terminal.stdin.resume === "function") {
    terminal.stdin.resume();
  }

  const { unmount, waitUntilExit } = render(
    React.createElement(App, {
      onExit: (result?: SelectionResult) => {
        selectionResult = result;
      },
    }),
    {
      stdin: terminal.stdin,
      stdout: terminal.stdout,
      stderr: terminal.stderr,
    },
  );

  // Wait for user to exit
  try {
    await waitUntilExit();
  } finally {
    terminal.exitRawMode();
    if (typeof terminal.stdin.pause === "function") {
      terminal.stdin.pause();
    }
    // Inkが残した data リスナーが子プロセス入力を奪わないようクリーンアップ
    terminal.stdin.removeAllListeners?.("data");
    terminal.stdin.removeAllListeners?.("keypress");
    terminal.stdin.removeAllListeners?.("readable");
    unmount();
  }

  return selectionResult;
}

/**
 * Handle AI tool workflow
 */
async function handleAIToolWorkflow(
  selectionResult: SelectionResult,
): Promise<void> {
  const {
    branch,
    displayName,
    branchType,
    remoteBranch,
    tool,
    mode,
    skipPermissions,
  } = selectionResult;

  const branchLabel = displayName ?? branch;
  printInfo(
    `Selected: ${branchLabel} with ${tool} (${mode} mode, skipPermissions: ${skipPermissions})`,
  );

  try {
    // Get repository root
    const repoRoot = await getRepositoryRoot();

    // Determine ensure options (local vs remote branch)
    const ensureOptions: EnsureWorktreeOptions = {};

    if (branchType === "remote") {
      const remoteRef = remoteBranch ?? branch;
      const localExists = await branchExists(branch);

      ensureOptions.baseBranch = remoteRef;
      ensureOptions.isNewBranch = !localExists;
    }

    const orchestrator = new WorktreeOrchestrator();

    const existingWorktree = await worktreeExists(branch);
    const willCreateWorktree = !existingWorktree;

    // Ensure worktree exists (using orchestrator)
    if (willCreateWorktree) {
      const targetLabel = ensureOptions.isNewBranch
        ? `base ${ensureOptions.baseBranch ?? branch}`
        : `branch ${branch}`;
      printInfo(
        `Creating worktree for ${targetLabel}. Progress indicator running...`,
      );
    }

    const worktreePath = await orchestrator.ensureWorktree(
      branch,
      repoRoot,
      ensureOptions,
    );

    if (existingWorktree) {
      printInfo(`Reusing existing worktree: ${existingWorktree}`);
    } else if (ensureOptions.isNewBranch) {
      const base = ensureOptions.baseBranch ?? "";
      printInfo(`Created new worktree from ${base}: ${worktreePath}`);
    } else if (willCreateWorktree) {
      printInfo(`Created worktree: ${worktreePath}`);
    }

    printInfo(`Worktree ready: ${worktreePath}`);

    // Update remotes and attempt fast-forward pull
    await fetchAllRemotes({ cwd: repoRoot });

    let fastForwardError: Error | null = null;
    try {
      await pullFastForward(worktreePath);
      printInfo(`Fast-forward pull finished for ${branch}.`);
    } catch (error) {
      fastForwardError = error instanceof Error ? error : new Error(String(error));
      printWarning(
        `Fast-forward pull failed for ${branch}. Checking for divergence before continuing...`,
      );
    }

    const divergenceBranches = new Set<string>();
    const sanitizeBranchName = (value: string | null | undefined) => {
      if (!value) return null;
      return value.replace(/^origin\//, "");
    };

    const sanitizedBranch = sanitizeBranchName(branch);
    if (sanitizedBranch) {
      divergenceBranches.add(sanitizedBranch);
    }

    const sanitizedRemoteBranch = sanitizeBranchName(remoteBranch);
    if (sanitizedRemoteBranch) {
      divergenceBranches.add(sanitizedRemoteBranch);
    }

    const divergenceStatuses = await getBranchDivergenceStatuses({
      cwd: repoRoot,
      branches: Array.from(divergenceBranches),
    });
    const divergedBranches = divergenceStatuses.filter(
      (status) => status.remoteAhead > 0 && status.localAhead > 0,
    );

    if (divergedBranches.length > 0) {
      printWarning(
        "Potential merge conflicts detected when pulling the following local branches:",
      );

      divergedBranches.forEach(({ branch: divergedBranch, remoteAhead, localAhead }) => {
        const highlight = divergedBranch === branch ? " (selected branch)" : "";
        console.warn(
          chalk.yellow(
            `   • ${divergedBranch}${highlight}  remote:+${remoteAhead}  local:+${localAhead}`,
          ),
        );
      });

      printWarning(
        "Resolve these divergences (e.g., rebase or merge) before launching to avoid conflicts.",
      );
      await waitForEnter("Press Enter to continue.");
    } else if (fastForwardError) {
      printWarning(
        `Fast-forward pull could not complete (${fastForwardError.message}). Continuing without blocking.`,
      );
    }

    // Get tool definition
    const toolConfig = await getToolById(tool);

    if (!toolConfig) {
      throw new Error(`Tool not found: ${tool}`);
    }

    // Launch selected AI tool
    // Builtin tools use their dedicated launch functions
    // Custom tools use the generic launchCustomAITool function
    if (tool === "claude-code") {
      await launchClaudeCode(worktreePath, {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        skipPermissions,
      });
    } else if (tool === "codex-cli") {
      await launchCodexCLI(worktreePath, {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        bypassApprovals: skipPermissions,
      });
    } else {
      // Custom tool
      printInfo(`Launching custom tool: ${toolConfig.displayName}`);
      await launchCustomAITool(toolConfig, {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        skipPermissions,
        cwd: worktreePath,
      });
    }

    // Save session with lastUsedTool
    await saveSession({
      lastWorktreePath: worktreePath,
      lastBranch: branch,
      lastUsedTool: tool,
      timestamp: Date.now(),
      repositoryRoot: repoRoot,
    });

    printInfo("Session completed successfully. Returning to main menu...");
    return;
  } catch (error) {
    if (error instanceof Error) {
      printError(`Error during workflow: ${error.message}`);
    } else {
      printError(`Unexpected error: ${String(error)}`);
    }
    await waitForErrorAcknowledgement();
    return;
  }
}

/**
 * Main entry point with loop
 */
export async function main(): Promise<void> {
  try {
    // Parse command line arguments
    const args = process.argv.slice(2);
    const showVersionFlag = args.includes("-v") || args.includes("--version");
    const showHelpFlag = args.includes("-h") || args.includes("--help");

    // Version flag has higher priority than help
    if (showVersionFlag) {
      await showVersion();
      return;
    }

    if (showHelpFlag) {
      showHelp();
      return;
    }

    // Check if current directory is a Git repository
    if (!(await isGitRepository())) {
      printError(`Current directory is not a Git repository: ${process.cwd()}`);
      printInfo(
        "Please run this command from within a Git repository or worktree directory.",
      );

      // Docker環境でよくある問題: safe.directory設定
      printInfo(
        "\\nIf you're running in Docker, you may need to configure Git safe.directory:",
      );
      printInfo("  git config --global --add safe.directory '*'");
      printInfo("\\nOr run with DEBUG=1 for more information:");
      printInfo("  DEBUG=1 bun run start");

      await waitForErrorAcknowledgement();
      process.exit(1);
    }

    // Main loop: UI → AI Tool → back to UI
    while (true) {
      const selectionResult = await mainInkUI();

      if (!selectionResult) {
        // User quit (pressed q without making selections)
        printInfo("Goodbye!");
        break;
      }

      // Handle AI tool workflow. The function internally manages error acknowledgement
      // and always resolves, so we can safely continue the loop afterwards.
      await handleAIToolWorkflow(selectionResult);
    }
  } catch (error) {
    if (error instanceof Error) {
      printError(error.message);
    } else {
      printError(String(error));
    }
    await waitForErrorAcknowledgement();
    process.exit(1);
  }
}

// Run the application if this module is executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch(async (error) => {
    console.error("Fatal error:", error);
    await waitForErrorAcknowledgement();
    process.exit(1);
  });
}
