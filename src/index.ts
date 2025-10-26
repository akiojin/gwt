#!/usr/bin/env node

import { isGitRepository } from "./git.js";
import chalk from "chalk";

/**
 * Simple print functions (replacing legacy UI display functions)
 */
function printError(message: string): void {
  console.error(chalk.red(`❌ ${message}`));
}

function printInfo(message: string): void {
  console.log(chalk.blue(`ℹ️  ${message}`));
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
 */
async function mainInkUI(): Promise<void> {
  const { render } = await import("ink");
  const React = await import("react");
  const { App } = await import("./ui/components/App.js");

  // Check if current directory is a Git repository
  if (!(await isGitRepository())) {
    printError("Current directory is not a Git repository.");
    process.exit(1);
  }

  let selectedBranch: string | undefined;

  const { unmount, waitUntilExit } = render(
    React.createElement(App, {
      onExit: (branch?: string) => {
        selectedBranch = branch;
      },
    }),
  );

  // Wait for user to exit
  await waitUntilExit();
  unmount();

  // If a branch was selected, handle it
  if (selectedBranch) {
    printInfo(`Selected branch: ${selectedBranch}`);
    // TODO: Implement branch handling logic
    // For now, just exit
  }
}

/**
 * Main entry point
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

    // Run Ink UI
    await mainInkUI();
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
