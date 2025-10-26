#!/usr/bin/env node

import chalk from "chalk";
import {
  isGitRepository,
  getRepositoryRoot,
} from "./git.js";
import {
  createWorktree,
  worktreeExists,
} from "./worktree.js";
import {
  launchClaudeCode,
  ClaudeError,
} from "./claude.js";
import {
  launchCodexCLI,
  CodexError,
} from "./codex.js";
import {
  saveSession,
  type SessionData,
} from "./config/index.js";
import type {
  WorktreeConfig,
} from "./ui/types.js";
import type {
  AppResult,
  LaunchRequest,
} from "./ui/components/App.js";

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

Usage: claude-worktree

Description:
  Interactive Git worktree manager with Ink-based UI.

  - Select execution mode (normal / continue / resume)
  - Pick a branch or session
  - Launch Claude Code or Codex CLI from the chosen worktree

Notes:
  The CLI no longer accepts command-line flags. Launch without arguments and
  follow the on-screen prompts.
`);
}

/**
 * Main function for Ink.js UI
 */
async function mainInkUI(): Promise<AppResult> {
  const { render } = await import('ink');
  const React = await import('react');
  const { App } = await import('./ui/components/App.js');

  // Check if current directory is a Git repository
  if (!(await isGitRepository())) {
    printError("Current directory is not a Git repository.");
    process.exit(1);
  }

  const repoRoot = await getRepositoryRoot();
  let result: AppResult = { type: 'quit' };

  const { unmount, waitUntilExit } = render(
    React.createElement(App, {
      repoRoot,
      onExit: (appResult: AppResult) => {
        result = appResult;
      },
    })
  );

  // Wait for user to exit
  await waitUntilExit();
  unmount();

  return result;
}

export async function ensureWorktreeExists(
  launch: LaunchRequest,
): Promise<void> {
  if (!launch.createWorktree) {
    return;
  }

  const config: WorktreeConfig = {
    branchName: launch.branchName,
    worktreePath: launch.worktreePath,
    repoRoot: launch.repoRoot,
    isNewBranch: launch.isNewBranch,
    baseBranch: launch.baseBranch ?? launch.branchName,
  };

  printInfo(
    `Creating worktree for ${launch.branchName} at ${launch.worktreePath}`,
  );
  await createWorktree(config);

  const path = await worktreeExists(launch.branchName);
  if (!path) {
    throw new Error(
      `Worktree creation reported success but path not found: ${launch.worktreePath}`,
    );
  }
}

export async function launchTool(launch: LaunchRequest): Promise<void> {
  if (!launch.tool) {
    printInfo("No AI tool selected. Nothing to do.");
    return;
  }

  const sessionData: SessionData = {
    lastWorktreePath: launch.worktreePath,
    lastBranch: launch.branchName,
    repositoryRoot: launch.repoRoot,
    timestamp: Date.now(),
  };

  await saveSession(sessionData);

  switch (launch.tool) {
    case 'claude-code':
      await launchClaudeCode(launch.worktreePath, {
        mode: launch.mode,
        skipPermissions: launch.skipPermissions,
      });
      break;
    case 'codex-cli':
      await launchCodexCLI(launch.worktreePath, {
        mode: launch.mode,
        bypassApprovals: launch.skipPermissions,
      });
      break;
    default:
      throw new Error(`Unknown tool: ${launch.tool}`);
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

    if (args.length > 0 && !showHelpFlag) {
      printError(
        "This CLI no longer accepts command-line options. Run `claude-worktree --help` for details.",
      );
      process.exit(1);
    }

    if (showHelpFlag) {
      showHelp();
      return;
    }

    // Run Ink UI
    const result = await mainInkUI();

    if (result.type === 'quit') {
      return;
    }

    const launch = result.launch;

    try {
      await ensureWorktreeExists(launch);
    } catch (error) {
      printError(
        `Failed to prepare worktree: ${error instanceof Error ? error.message : String(error)}`,
      );
      process.exit(1);
    }

    try {
      await launchTool(launch);
    } catch (error) {
      if (error instanceof ClaudeError || error instanceof CodexError) {
        printError(error.message);
      } else if (error instanceof Error) {
        printError(error.message);
      } else {
        printError(String(error));
      }
      process.exit(1);
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
