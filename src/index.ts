#!/usr/bin/env node

import {
  isGitRepository,
  getRepositoryRoot,
  fetchAllRemotes,
  pullFastForward,
  getBranchDivergenceStatuses,
} from "./git.js";
import { launchClaudeCode } from "./claude.js";
import { launchCodexCLI } from "./codex.js";
import { WorktreeOrchestrator } from "./services/WorktreeOrchestrator.js";
import chalk from "chalk";
import type { SelectionResult } from "./ui/components/App.js";
import readline from "node:readline";

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
  -c              Continue from the last session (automatically open the last used worktree)
  -r, --resume    Resume a session - interactively select from available sessions
  --tool <name>   Select AI tool to launch in worktree (claude|codex)
  -h, --help      Show this help message

Description:
  Interactive Git worktree manager with AI tool selection (Claude Code / Codex CLI) and graphical branch selection.

  Without options: Opens the interactive menu to select branches and manage worktrees.
  With -c option: Automatically continues from where you left off in the last session.
  With -r option: Shows a list of recent sessions to choose from and resume.

Pass-through:
  Use "--" to pass additional args directly to the selected tool.
  Examples:
    claude-worktree --tool claude -- -r
    claude-worktree --tool codex -- resume --last
`);
}

/**
 * Main function for Ink.js UI
 * Returns SelectionResult if user made selections, undefined if user quit
 */
async function mainInkUI(): Promise<SelectionResult | undefined> {
  const { render } = await import("ink");
  const React = await import("react");
  const { App } = await import("./ui/components/App.js");

  let selectionResult: SelectionResult | undefined;

  const { unmount, waitUntilExit } = render(
    React.createElement(App, {
      onExit: (result?: SelectionResult) => {
        selectionResult = result;
      },
    }),
  );

  // Wait for user to exit
  await waitUntilExit();
  unmount();

  return selectionResult;
}

/**
 * Handle AI tool workflow
 */
async function handleAIToolWorkflow(
  selectionResult: SelectionResult,
): Promise<void> {
  const { branch, tool, mode, skipPermissions } = selectionResult;
  printInfo(
    `Selected: ${branch} with ${tool} (${mode} mode, skipPermissions: ${skipPermissions})`,
  );

  try {
    // Get repository root
    const repoRoot = await getRepositoryRoot();

    // Ensure worktree exists (using orchestrator)
    const orchestrator = new WorktreeOrchestrator();
    const worktreePath = await orchestrator.ensureWorktree(branch, repoRoot);
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

    const divergenceStatuses = await getBranchDivergenceStatuses({ cwd: repoRoot });
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
      await waitForEnter("Press Enter to continue anyway, or Ctrl+C to abort.");
    } else if (fastForwardError) {
      printWarning(
        `Fast-forward pull could not complete (${fastForwardError.message}). Continuing without blocking.`,
      );
    }

    // Launch selected AI tool
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
    }

    printInfo("Session completed successfully. Returning to main menu...");
  } catch (error) {
    if (error instanceof Error) {
      printError(`Error during workflow: ${error.message}`);
    } else {
      printError(`Unexpected error: ${String(error)}`);
    }
    throw error; // Re-throw to handle in main loop
  }
}

/**
 * Main entry point with loop
 */
export async function main(): Promise<void> {
  try {
    // Parse command line arguments
    const args = process.argv.slice(2);
    const showHelpFlag = args.includes("-h") || args.includes("--help");

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

      // Handle AI tool workflow
      try {
        await handleAIToolWorkflow(selectionResult);
        // After AI tool completes, loop back to UI
      } catch (error) {
        // Error during workflow, but don't exit - return to UI
        printError("Workflow error, returning to main menu...");
      }
    }
  } catch (error) {
    if (error instanceof Error) {
      printError(error.message);
    } else {
      printError(String(error));
    }
    process.exit(1);
  }
}

// Run the application if this module is executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
}
