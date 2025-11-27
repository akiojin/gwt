#!/usr/bin/env node

import {
  isGitRepository,
  getRepositoryRoot,
  branchExists,
  fetchAllRemotes,
  pullFastForward,
  getBranchDivergenceStatuses,
  GitError,
} from "./git.js";
import { launchClaudeCode } from "./claude.js";
import {
  launchCodexCLI,
  CodexError,
  type CodexReasoningEffort,
} from "./codex.js";
import { launchGeminiCLI, GeminiError } from "./gemini.js";
import { launchQwenCLI, QwenError } from "./qwen.js";
import {
  WorktreeOrchestrator,
  type EnsureWorktreeOptions,
} from "./services/WorktreeOrchestrator.js";
import chalk from "chalk";
import type { SelectionResult } from "./cli/ui/components/App.js";
import {
  worktreeExists,
  isProtectedBranchName,
  switchToProtectedBranch,
  WorktreeError,
} from "./worktree.js";
import {
  getTerminalStreams,
  waitForUserAcknowledgement,
} from "./utils/terminal.js";
import { getToolById, getSharedEnvironment } from "./config/tools.js";
import { launchCustomAITool } from "./launcher.js";
import { saveSession } from "./config/index.js";
import { getPackageVersion } from "./utils.js";
import readline from "node:readline";
import {
  installDependenciesForWorktree,
  DependencyInstallError,
  type DependencyInstallResult,
} from "./services/dependency-installer.js";

const ERROR_PROMPT = chalk.yellow(
  "Review the error details, then press Enter to continue.",
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

type GitStepResult<T> = { ok: true; value: T } | { ok: false };

function isGitRelatedError(error: unknown): boolean {
  if (!error) {
    return false;
  }

  if (error instanceof GitError || error instanceof WorktreeError) {
    return true;
  }

  if (error instanceof Error) {
    return error.name === "GitError" || error.name === "WorktreeError";
  }

  if (
    typeof error === "object" &&
    "name" in (error as Record<string, unknown>)
  ) {
    const name = (error as { name?: string }).name;
    return name === "GitError" || name === "WorktreeError";
  }

  return false;
}

function isRecoverableError(error: unknown): boolean {
  if (!error) {
    return false;
  }

  if (
    error instanceof GitError ||
    error instanceof WorktreeError ||
    error instanceof CodexError ||
    error instanceof GeminiError ||
    error instanceof QwenError ||
    error instanceof DependencyInstallError
  ) {
    return true;
  }

  if (error instanceof Error) {
    return (
      error.name === "GitError" ||
      error.name === "WorktreeError" ||
      error.name === "CodexError" ||
      error.name === "GeminiError" ||
      error.name === "QwenError" ||
      error.name === "DependencyInstallError"
    );
  }

  if (
    typeof error === "object" &&
    "name" in (error as Record<string, unknown>)
  ) {
    const name = (error as { name?: string }).name;
    return (
      name === "GitError" ||
      name === "WorktreeError" ||
      name === "CodexError" ||
      name === "GeminiError" ||
      name === "QwenError" ||
      name === "DependencyInstallError"
    );
  }

  return false;
}

async function runGitStep<T>(
  description: string,
  step: () => Promise<T>,
): Promise<GitStepResult<T>> {
  try {
    const value = await step();
    return { ok: true, value };
  } catch (error) {
    if (isGitRelatedError(error)) {
      const details = error instanceof Error ? error.message : String(error);
      printWarning(`Git operation failed (${description}). Error: ${details}`);
      await waitForErrorAcknowledgement();
      return { ok: false };
    }
    throw error;
  }
}

async function runDependencyInstallStep<T extends DependencyInstallResult>(
  description: string,
  step: () => Promise<T>,
): Promise<{ ok: true; value: T }> {
  try {
    const value = await step();
    return { ok: true, value };
  } catch (error) {
    if (error instanceof DependencyInstallError) {
      const details = error.message ?? "";
      // 依存インストールが失敗してもワークフロー自体は継続させる
      printError(`Failed to complete ${description}. ${details}`);
      await waitForErrorAcknowledgement();

      const fallbackResult = {
        skipped: true,
        manager: null,
        lockfile: null,
        reason: "unknown-error",
        message: details,
      } as T;

      return { ok: true, value: fallbackResult };
    }

    throw error;
  }
}

async function waitForEnter(promptMessage: string): Promise<void> {
  if (!process.stdin.isTTY) {
    // For non-interactive environments, resolve immediately.
    return;
  }

  // Ensure stdin is resumed and not in raw mode before using readline.
  // This is crucial for environments where stdin might be paused or in raw mode
  // by other libraries (like Ink.js).
  if (typeof process.stdin.resume === "function") {
    process.stdin.resume();
  }
  if (process.stdin.isRaw) {
    process.stdin.setRawMode(false);
  }

  await new Promise<void>((resolve) => {
    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout,
    });

    // Handle Ctrl+C to gracefully exit.
    rl.on("SIGINT", () => {
      rl.close();
      // Restore stdin to a paused state before exiting.
      if (typeof process.stdin.pause === "function") {
        process.stdin.pause();
      }
      process.exit(0);
    });

    rl.question(`${promptMessage}\n`, () => {
      rl.close();
      // Pause stdin again to allow other parts of the application
      // to take control if needed.
      if (typeof process.stdin.pause === "function") {
        process.stdin.pause();
      }
      resolve();
    });
  });
}

function showHelp(): void {
  console.log(`
Worktree Manager

Usage: gwt [options]

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
  const { App } = await import("./cli/ui/components/App.js");
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
export async function handleAIToolWorkflow(
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
    model,
    inferenceLevel,
  } = selectionResult;

  const branchLabel = displayName ?? branch;
  const modelInfo =
    model || inferenceLevel
      ? `, model=${model ?? "default"}${inferenceLevel ? `/${inferenceLevel}` : ""}`
      : "";
  printInfo(
    `Selected: ${branchLabel} with ${tool} (${mode} mode${modelInfo}, skipPermissions: ${skipPermissions})`,
  );

  try {
    // Get repository root
    const repoRootResult = await runGitStep("retrieve repository root", () =>
      getRepositoryRoot(),
    );
    if (!repoRootResult.ok) {
      return;
    }
    const repoRoot = repoRootResult.value;

    // Determine ensure options (local vs remote branch)
    const ensureOptions: EnsureWorktreeOptions = {};

    if (branchType === "remote") {
      const remoteRef = remoteBranch ?? branch;
      const localExists = await branchExists(branch);

      ensureOptions.baseBranch = remoteRef;
      ensureOptions.isNewBranch = !localExists;
    }

    const existingWorktree = await worktreeExists(branch);

    const isProtectedBranch =
      isProtectedBranchName(branch) ||
      (remoteBranch ? isProtectedBranchName(remoteBranch) : false);

    let protectedCheckoutResult: "none" | "local" | "remote" = "none";
    if (isProtectedBranch) {
      const protectedRemoteRef =
        remoteBranch ??
        (branchType === "remote" ? (displayName ?? branch) : null);
      const switchResult = await runGitStep(
        `check out protected branch '${branch}'`,
        () =>
          switchToProtectedBranch({
            branchName: branch,
            repoRoot,
            remoteRef: protectedRemoteRef ?? null,
          }),
      );
      if (!switchResult.ok) {
        return;
      }
      protectedCheckoutResult = switchResult.value;
      ensureOptions.isNewBranch = false;
    }

    const willCreateWorktree = !existingWorktree && !isProtectedBranch;

    const orchestrator = new WorktreeOrchestrator();

    // Ensure worktree exists (using orchestrator)
    if (willCreateWorktree) {
      const targetLabel = ensureOptions.isNewBranch
        ? `base ${ensureOptions.baseBranch ?? branch}`
        : `branch ${branch}`;
      printInfo(
        `Creating worktree for ${targetLabel}. Progress indicator running...`,
      );
    }

    const worktreeResult = await runGitStep(
      `prepare worktree (${branch})`,
      () => orchestrator.ensureWorktree(branch, repoRoot, ensureOptions),
    );
    if (!worktreeResult.ok) {
      return;
    }
    const worktreePath = worktreeResult.value;

    if (isProtectedBranch) {
      if (protectedCheckoutResult === "remote" && remoteBranch) {
        printInfo(
          `Created local tracking branch '${branch}' from ${remoteBranch} in repository root.`,
        );
      } else if (protectedCheckoutResult === "local") {
        printInfo(
          `Checked out protected branch '${branch}' in repository root.`,
        );
      } else {
        printInfo(`Using repository root for protected branch '${branch}'.`);
      }
    } else if (existingWorktree) {
      printInfo(`Reusing existing worktree: ${existingWorktree}`);
    } else if (ensureOptions.isNewBranch) {
      const base = ensureOptions.baseBranch ?? "";
      printInfo(`Created new worktree from ${base}: ${worktreePath}`);
    } else if (willCreateWorktree) {
      printInfo(`Created worktree: ${worktreePath}`);
    }

    printInfo(`Worktree ready: ${worktreePath}`);

    const dependencyResult = await runDependencyInstallStep(
      `dependency installation (${branch})`,
      () => installDependenciesForWorktree(worktreePath),
    );
    if (!dependencyResult.ok) {
      return;
    }
    const dependencyStatus = dependencyResult.value;

    if (dependencyStatus.skipped) {
      let warningMessage: string;
      switch (dependencyStatus.reason) {
        case "missing-lockfile":
          warningMessage =
            "Skipping automatic install because no lockfiles (bun.lock / pnpm-lock.yaml / package-lock.json) or package.json were found. Run the appropriate package-manager install command manually if needed.";
          break;
        case "missing-binary":
          warningMessage = `Package manager '${dependencyStatus.manager ?? "unknown"}' is not available in this environment; skipping automatic install.`;
          break;
        case "install-failed":
          warningMessage = `Dependency installation failed via ${dependencyStatus.manager ?? "unknown"}. Continuing without reinstall.`;
          break;
        case "lockfile-access-error":
          warningMessage =
            "Unable to read dependency lockfiles due to a filesystem error. Continuing without reinstall.";
          break;
        default:
          warningMessage =
            "Skipping automatic dependency install due to an unexpected error. Continuing without reinstall.";
      }

      if (dependencyStatus.message) {
        warningMessage = `${warningMessage}\nDetails: ${dependencyStatus.message}`;
      }

      printWarning(warningMessage);
    } else {
      printInfo(`Dependencies synced via ${dependencyStatus.manager}.`);
    }

    // Update remotes and attempt fast-forward pull
    const fetchResult = await runGitStep("fetch remotes", () =>
      fetchAllRemotes({ cwd: repoRoot }),
    );
    if (!fetchResult.ok) {
      return;
    }

    let fastForwardError: Error | null = null;
    try {
      await pullFastForward(worktreePath);
      printInfo(`Fast-forward pull finished for ${branch}.`);
    } catch (error) {
      fastForwardError =
        error instanceof Error ? error : new Error(String(error));
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

    const divergenceResult = await runGitStep("check branch divergence", () =>
      getBranchDivergenceStatuses({
        cwd: repoRoot,
        branches: Array.from(divergenceBranches),
      }),
    );
    if (!divergenceResult.ok) {
      return;
    }
    const divergenceStatuses = divergenceResult.value;
    const divergedBranches = divergenceStatuses.filter(
      (status) => status.remoteAhead > 0 && status.localAhead > 0,
    );

    if (divergedBranches.length > 0) {
      printWarning(
        "Potential merge conflicts detected when pulling the following local branches:",
      );

      divergedBranches.forEach(
        ({ branch: divergedBranch, remoteAhead, localAhead }) => {
          const highlight =
            divergedBranch === branch ? " (selected branch)" : "";
          console.warn(
            chalk.yellow(
              `   • ${divergedBranch}${highlight}  remote:+${remoteAhead}  local:+${localAhead}`,
            ),
          );
        },
      );

      printWarning(
        "Resolve these divergences (e.g., rebase or merge) before launching to avoid conflicts.",
      );
      await waitForEnter(
        "Press Enter to return to the main menu and resolve these issues manually.",
      );
      printWarning(
        "AI tool launch has been cancelled until divergences are resolved.",
      );
      return;
    } else if (fastForwardError) {
      printWarning(
        `Fast-forward pull could not complete (${fastForwardError.message}). Continuing without blocking.`,
      );
    }

    // Get tool definition and shared environment overrides
    const [toolConfig, sharedEnv] = await Promise.all([
      getToolById(tool),
      getSharedEnvironment(),
    ]);

    if (!toolConfig) {
      throw new Error(`Tool not found: ${tool}`);
    }

    // Save selection immediately so "last tool" is reflected even if the tool
    // is interrupted or killed mid-run (e.g., Ctrl+C).
    await saveSession({
      lastWorktreePath: worktreePath,
      lastBranch: branch,
      lastUsedTool: tool,
      toolLabel: toolConfig.displayName ?? tool,
      mode,
      model: model ?? null,
      timestamp: Date.now(),
      repositoryRoot: repoRoot,
    });

    // Launch selected AI tool
    // Builtin tools use their dedicated launch functions
    // Custom tools use the generic launchCustomAITool function
    if (tool === "claude-code") {
      const launchOptions: {
        mode?: "normal" | "continue" | "resume";
        skipPermissions?: boolean;
        envOverrides?: Record<string, string>;
        model?: string;
      } = {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        skipPermissions,
        envOverrides: sharedEnv,
      };
      if (model) {
        launchOptions.model = model;
      }
      await launchClaudeCode(worktreePath, launchOptions);
    } else if (tool === "codex-cli") {
      const launchOptions: {
        mode?: "normal" | "continue" | "resume";
        bypassApprovals?: boolean;
        envOverrides?: Record<string, string>;
        model?: string;
        reasoningEffort?: CodexReasoningEffort;
      } = {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        bypassApprovals: skipPermissions,
        envOverrides: sharedEnv,
      };
      if (model) {
        launchOptions.model = model;
      }
      if (inferenceLevel) {
        launchOptions.reasoningEffort = inferenceLevel as CodexReasoningEffort;
      }
      await launchCodexCLI(worktreePath, launchOptions);
    } else if (tool === "gemini-cli") {
      const launchOptions: {
        mode?: "normal" | "continue" | "resume";
        skipPermissions?: boolean;
        envOverrides?: Record<string, string>;
        model?: string;
      } = {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        skipPermissions,
        envOverrides: sharedEnv,
      };
      if (model) {
        launchOptions.model = model;
      }
      await launchGeminiCLI(worktreePath, launchOptions);
    } else if (tool === "qwen-cli") {
      const launchOptions: {
        mode?: "normal" | "continue" | "resume";
        skipPermissions?: boolean;
        envOverrides?: Record<string, string>;
        model?: string;
      } = {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        skipPermissions,
        envOverrides: sharedEnv,
      };
      if (model) {
        launchOptions.model = model;
      }
      await launchQwenCLI(worktreePath, launchOptions);
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
        sharedEnv,
      });
    }

    printInfo("Session completed successfully. Returning to main menu...");
    return;
  } catch (error) {
    // Handle recoverable errors (Git, Worktree, Codex errors)
    if (isRecoverableError(error)) {
      const details = error instanceof Error ? error.message : String(error);
      printError(`Error during workflow: ${details}`);
      await waitForErrorAcknowledgement();
      return;
    }
    // Re-throw non-recoverable errors
    throw error;
  }
}

type UIHandler = () => Promise<SelectionResult | undefined>;
type WorkflowHandler = (selection: SelectionResult) => Promise<void>;

function logLoopError(error: unknown, context: "ui" | "workflow"): void {
  const label = context === "ui" ? "UI" : "workflow";
  if (error instanceof Error) {
    printError(`${label} error: ${error.message}`);
  } else {
    printError(`${label} error: ${String(error)}`);
  }
}

export async function runInteractiveLoop(
  uiHandler: UIHandler = mainInkUI,
  workflowHandler: WorkflowHandler = handleAIToolWorkflow,
): Promise<void> {
  // Main loop: UI → AI Tool → back to UI
  while (true) {
    let selectionResult: SelectionResult | undefined;

    try {
      selectionResult = await uiHandler();
    } catch (error) {
      logLoopError(error, "ui");
      await waitForErrorAcknowledgement();
      continue;
    }

    if (!selectionResult) {
      // User quit (pressed q without making selections)
      printInfo("Goodbye!");
      break;
    }

    try {
      await workflowHandler(selectionResult);
    } catch (error) {
      logLoopError(error, "workflow");
      await waitForErrorAcknowledgement();
    }
  }
}

/**
 * Main entry point
 */
export async function main(): Promise<void> {
  // Parse command line arguments
  const args = process.argv.slice(2);
  const showVersionFlag = args.includes("-v") || args.includes("--version");
  const showHelpFlag = args.includes("-h") || args.includes("--help");
  const serveCommand = args.includes("serve");

  // Version flag has higher priority than help
  if (showVersionFlag) {
    await showVersion();
    return;
  }

  if (showHelpFlag) {
    showHelp();
    return;
  }

  // Start Web UI server if 'serve' command is provided
  if (serveCommand) {
    const { startWebServer } = await import("./web/server/index.js");
    await startWebServer();
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

  await runInteractiveLoop();
}

// Run the application if this module is executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch(async (error) => {
    console.error("Fatal error:", error);
    await waitForErrorAcknowledgement();
    process.exit(1);
  });
}
