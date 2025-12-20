#!/usr/bin/env node

import {
  isGitRepository,
  getRepositoryRoot,
  branchExists,
  fetchAllRemotes,
  pullFastForward,
  getBranchDivergenceStatuses,
  hasUncommittedChanges,
  hasUnpushedCommits,
  getUncommittedChangesCount,
  getUnpushedCommitsCount,
  pushBranchToRemote,
  GitError,
} from "./git.js";
import { launchClaudeCode } from "./claude.js";
import {
  launchCodexCLI,
  CodexError,
  type CodexReasoningEffort,
} from "./codex.js";
import { launchGeminiCLI, GeminiError } from "./gemini.js";
import {
  WorktreeOrchestrator,
  type EnsureWorktreeOptions,
} from "./services/WorktreeOrchestrator.js";
import chalk from "chalk";
import type { SelectionResult } from "./cli/ui/components/App.js";
import {
  isProtectedBranchName,
  switchToProtectedBranch,
  WorktreeError,
  resolveWorktreePathForBranch,
} from "./worktree.js";
import {
  getTerminalStreams,
  waitForUserAcknowledgement,
} from "./utils/terminal.js";
import { createLogger } from "./logging/logger.js";
import { getToolById, getSharedEnvironment } from "./config/tools.js";
import { launchCustomAITool } from "./launcher.js";
import { saveSession, loadSession } from "./config/index.js";
import {
  findLatestCodexSession,
  findLatestClaudeSession,
  findLatestGeminiSession,
} from "./utils/session.js";
import { getPackageVersion } from "./utils.js";
import { findLatestClaudeSessionId } from "./utils/session.js";
import { resolveContinueSessionId } from "./cli/ui/utils/continueSession.js";
import { normalizeModelId } from "./cli/ui/utils/modelOptions.js";
import {
  installDependenciesForWorktree,
  DependencyInstallError,
  type DependencyInstallResult,
} from "./services/dependency-installer.js";
import { confirmYesNo, waitForEnter } from "./utils/prompt.js";

const ERROR_PROMPT = chalk.yellow(
  "Review the error details, then press Enter to continue.",
);
const POST_SESSION_DELAY_MS = 3000;

// Category: cli
const appLogger = createLogger({ category: "cli" });

async function waitForErrorAcknowledgement(): Promise<void> {
  await waitForUserAcknowledgement(ERROR_PROMPT);
}

/**
 * Simple print functions (replacing legacy UI display functions)
 */
function printError(message: string): void {
  console.error(chalk.red(`❌ ${message}`));
  appLogger.error({ message });
}

function printInfo(message: string): void {
  console.log(chalk.blue(`ℹ️  ${message}`));
  appLogger.info({ message });
}

function printWarning(message: string): void {
  console.warn(chalk.yellow(`⚠️  ${message}`));
  appLogger.warn({ message });
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

function showHelp(): void {
  console.log(`
Worktree Manager

Usage: gwt [command] [options]

Commands:
  serve           Start Web UI server (http://localhost:3000)

Options:
  -h, --help      Show this help message
  -v, --version   Show version information

Description:
  Interactive Git worktree manager with AI tool selection (Claude Code / Codex CLI) and graphical branch selection.
  Launch without additional options to open the interactive CLI menu.
  Use 'gwt serve' to start the Web UI server for browser-based management.
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

  // Resume stdin to ensure it's ready for Ink.js
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
    sessionId: selectedSessionId,
  } = selectionResult;

  const branchLabel = displayName ?? branch;
  const normalizedModel = normalizeModelId(tool, model ?? null);
  const modelInfo =
    normalizedModel || inferenceLevel
      ? `, model=${normalizedModel ?? "default"}${inferenceLevel ? `/${inferenceLevel}` : ""}`
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

    const existingWorktreeResolution =
      await resolveWorktreePathForBranch(branch);
    const existingWorktree = existingWorktreeResolution.path;
    if (!existingWorktree && existingWorktreeResolution.mismatch) {
      const actualBranch =
        existingWorktreeResolution.mismatch.actualBranch ?? "unknown";
      printWarning(
        `Worktree mismatch detected: ${existingWorktreeResolution.mismatch.path} is checked out to '${actualBranch}'. Creating or reusing the correct worktree for '${branch}'.`,
      );
    }

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
    await saveSession(
      {
        lastWorktreePath: worktreePath,
        lastBranch: branch,
        lastUsedTool: tool,
        toolLabel: toolConfig.displayName ?? tool,
        mode,
        model: normalizedModel ?? null,
        reasoningLevel: inferenceLevel ?? null,
        skipPermissions: skipPermissions ?? null,
        timestamp: Date.now(),
        repositoryRoot: repoRoot,
      },
      { skipHistory: true },
    );

    // Lookup saved session ID for Continue (auto attach)
    let resumeSessionId: string | null =
      selectedSessionId && selectedSessionId.length > 0
        ? selectedSessionId
        : null;
    if (mode === "continue") {
      const existingSession = await loadSession(repoRoot);
      const history = existingSession?.history ?? [];

      resumeSessionId =
        resumeSessionId ??
        (await resolveContinueSessionId({
          history,
          sessionData: existingSession,
          branch,
          toolId: tool,
          repoRoot,
        }));

      if (!resumeSessionId) {
        printWarning(
          "No saved session ID found for this branch/tool. Falling back to tool default.",
        );
      }
    }

    const launchStartedAt = Date.now();

    // Launch selected AI tool
    // Builtin tools use their dedicated launch functions
    // Custom tools use the generic launchCustomAITool function
    let launchResult: { sessionId?: string | null } | void;
    if (tool === "claude-code") {
      const launchOptions: {
        mode?: "normal" | "continue" | "resume";
        skipPermissions?: boolean;
        envOverrides?: Record<string, string>;
        model?: string;
        sessionId?: string | null;
        chrome?: boolean;
      } = {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        skipPermissions,
        envOverrides: sharedEnv,
        sessionId: resumeSessionId,
        chrome: true,
      };
      if (normalizedModel) {
        launchOptions.model = normalizedModel;
      }
      launchResult = await launchClaudeCode(worktreePath, launchOptions);
    } else if (tool === "codex-cli") {
      const launchOptions: {
        mode?: "normal" | "continue" | "resume";
        bypassApprovals?: boolean;
        envOverrides?: Record<string, string>;
        model?: string;
        reasoningEffort?: CodexReasoningEffort;
        sessionId?: string | null;
      } = {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        bypassApprovals: skipPermissions,
        envOverrides: sharedEnv,
        sessionId: resumeSessionId,
      };
      if (normalizedModel) {
        launchOptions.model = normalizedModel;
      }
      if (inferenceLevel) {
        launchOptions.reasoningEffort = inferenceLevel as CodexReasoningEffort;
      }
      launchResult = await launchCodexCLI(worktreePath, launchOptions);
    } else if (tool === "gemini-cli") {
      const launchOptions: {
        mode?: "normal" | "continue" | "resume";
        skipPermissions?: boolean;
        envOverrides?: Record<string, string>;
        model?: string;
        sessionId?: string | null;
      } = {
        mode:
          mode === "resume"
            ? "resume"
            : mode === "continue"
              ? "continue"
              : "normal",
        skipPermissions,
        envOverrides: sharedEnv,
        sessionId: resumeSessionId,
      };
      if (normalizedModel) {
        launchOptions.model = normalizedModel;
      }
      launchResult = await launchGeminiCLI(worktreePath, launchOptions);
    } else {
      // Custom tool
      printInfo(`Launching custom tool: ${toolConfig.displayName}`);
      launchResult = await launchCustomAITool(toolConfig, {
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

    // Persist session with captured session ID (if any)
    let finalSessionId =
      (launchResult as { sessionId?: string | null } | undefined)?.sessionId ??
      resumeSessionId ??
      null;

    if (!finalSessionId && tool === "claude-code") {
      try {
        finalSessionId =
          (await findLatestClaudeSessionId(worktreePath)) ?? null;
      } catch {
        finalSessionId = null;
      }
    }
    const finishedAt = Date.now();

    if (!finalSessionId && tool === "codex-cli") {
      try {
        const latest = await findLatestCodexSession({
          since: launchStartedAt - 60_000,
          until: finishedAt + 60_000,
          preferClosestTo: finishedAt,
          windowMs: 60 * 60 * 1000,
          cwd: worktreePath,
        });
        if (latest) {
          finalSessionId = latest.id;
        }
      } catch {
        // ignore fallback failure
      }
    } else if (!finalSessionId && tool === "claude-code") {
      try {
        const latestClaude = await findLatestClaudeSession(worktreePath, {
          since: launchStartedAt - 60_000,
          until: finishedAt + 60_000,
          preferClosestTo: finishedAt,
          windowMs: 60 * 60 * 1000,
        });
        if (latestClaude) {
          finalSessionId = latestClaude.id;
        }
      } catch {
        // ignore
      }
    } else if (!finalSessionId && tool === "gemini-cli") {
      try {
        const latestGemini = await findLatestGeminiSession({
          since: launchStartedAt - 60_000,
          until: finishedAt + 60_000,
          preferClosestTo: finishedAt,
          windowMs: 60 * 60 * 1000,
          cwd: worktreePath,
        });
        if (latestGemini) {
          finalSessionId = latestGemini.id;
        }
      } catch {
        // ignore
      }
    }

    await saveSession({
      lastWorktreePath: worktreePath,
      lastBranch: branch,
      lastUsedTool: tool,
      toolLabel: toolConfig.displayName ?? tool,
      mode,
      model: normalizedModel ?? null,
      reasoningLevel: inferenceLevel ?? null,
      skipPermissions: skipPermissions ?? null,
      timestamp: Date.now(),
      repositoryRoot: repoRoot,
      lastSessionId: finalSessionId,
    });

    try {
      const [hasUncommitted, hasUnpushed] = await Promise.all([
        hasUncommittedChanges(worktreePath),
        hasUnpushedCommits(worktreePath, branch),
      ]);

      if (hasUncommitted) {
        const uncommittedCount = await getUncommittedChangesCount(worktreePath);
        const countLabel =
          uncommittedCount > 0 ? ` (${uncommittedCount}件)` : "";
        printWarning(`未コミットの変更があります${countLabel}。`);
      }

      if (hasUnpushed) {
        const unpushedCount = await getUnpushedCommitsCount(
          worktreePath,
          branch,
        );
        const countLabel = unpushedCount > 0 ? ` (${unpushedCount}件)` : "";
        const shouldPush = await confirmYesNo(
          `未プッシュのコミットがあります${countLabel}。プッシュしますか？`,
          { defaultValue: false },
        );
        if (shouldPush) {
          printInfo(`Pushing origin/${branch}...`);
          try {
            await pushBranchToRemote(worktreePath, branch);
            printInfo(`Push completed for ${branch}.`);
          } catch (error) {
            const details =
              error instanceof Error ? error.message : String(error);
            printWarning(`Push failed for ${branch}: ${details}`);
          }
        }
      }
    } catch (error) {
      const details = error instanceof Error ? error.message : String(error);
      printWarning(`Failed to check git status after session: ${details}`);
    }
    // Small buffer before returning to branch list to avoid abrupt screen swap
    await new Promise((resolve) => setTimeout(resolve, POST_SESSION_DELAY_MS));
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
