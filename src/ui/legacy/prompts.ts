import { select, input, confirm } from "@inquirer/prompts";
import chalk from "chalk";
import stringWidth from "string-width";
import {
  BranchInfo,
  BranchType,
  NewBranchConfig,
  CleanupTarget,
} from "../types.js";
import { SessionData } from "../../config/index.js";

function stripAnsi(value: string): string {
  // eslint-disable-next-line no-control-regex
  return value.replace(/\u001B\[[0-9;]*m/g, "");
}

type SelectChoice<TValue> = {
  name: string;
  value: TValue;
  description?: string;
  disabled?: boolean | string;
};

type SelectPromptConfig<TValue> = {
  message: string;
  choices: Array<SelectChoice<TValue>>;
};
/**
 * Custom select prompt with q key support for going back
 * @param config - prompt configuration
 * @returns selected value or null if user pressed q
 */
async function createQuitableSelect<T>(config: {
  message: string;
  choices: Array<{
    name: string;
    value: T;
    description?: string;
    disabled?: boolean | string;
  }>;
  pageSize?: number;
}): Promise<T | null> {
  const {
    createPrompt,
    useState,
    useKeypress,
    isEnterKey,
    usePrefix,
    isUpKey,
    isDownKey,
  } = await import("@inquirer/core");

  const customSelect = createPrompt<T | null, typeof config>(
    (promptConfig, done) => {
      const [selectedIndex, setSelectedIndex] = useState<number>(0);
      const [status, setStatus] = useState<"idle" | "done">("idle");
      const prefix = usePrefix({});

      useKeypress((key) => {
        if (status === "done") return;

        if (key.name === "q" || (key.name === "c" && key.ctrl)) {
          setStatus("done");
          done(null);
          return;
        }

        if (isEnterKey(key)) {
          const selectedChoice = promptConfig.choices[selectedIndex];
          if (!selectedChoice) return;
          if (selectedChoice.disabled) {
            return;
          }
          setStatus("done");
          done(selectedChoice.value);
          return;
        }

        if (isUpKey(key)) {
          let newIndex =
            selectedIndex > 0
              ? selectedIndex - 1
              : promptConfig.choices.length - 1;
          // Skip disabled items
          while (
            promptConfig.choices[newIndex]?.disabled &&
            newIndex !== selectedIndex
          ) {
            newIndex =
              newIndex > 0 ? newIndex - 1 : promptConfig.choices.length - 1;
          }
          setSelectedIndex(newIndex);
        } else if (isDownKey(key)) {
          let newIndex =
            selectedIndex < promptConfig.choices.length - 1
              ? selectedIndex + 1
              : 0;
          // Skip disabled items
          while (
            promptConfig.choices[newIndex]?.disabled &&
            newIndex !== selectedIndex
          ) {
            newIndex =
              newIndex < promptConfig.choices.length - 1 ? newIndex + 1 : 0;
          }
          setSelectedIndex(newIndex);
        }
      });

      if (status === "done") {
        return "";
      }

      const message = promptConfig.message;
      const choicesDisplay = promptConfig.choices
        .map((choice, index) => {
          const isSelected = index === selectedIndex;
          const pointer = isSelected ? "❯" : " ";
          const nameDisplay = choice.disabled
            ? chalk.gray(choice.name)
            : isSelected
              ? chalk.cyan(choice.name)
              : choice.name;
          const description =
            choice.description && isSelected
              ? `\n  ${choice.disabled ? chalk.gray(choice.description) : chalk.gray(choice.description)}`
              : "";
          return `${pointer} ${nameDisplay}${description}`;
        })
        .join("\n");

      return `${prefix} ${message}\n${choicesDisplay}`;
    },
  );

  return await customSelect(config);
}

export async function selectFromTable(
  choices: Array<{
    name: string;
    value: string;
    description?: string;
    disabled?: boolean;
  }>,
  statistics?: {
    branches: BranchInfo[];
    worktrees: import("../../worktree.js").WorktreeInfo[];
  },
): Promise<string> {
  // Display statistics if provided
  if (statistics) {
    const { printStatistics, printWelcome } = await import("./display.js");
    console.clear();
    await printWelcome();
    await printStatistics(statistics.branches, statistics.worktrees);
  }

  return await selectBranchWithShortcuts(choices);
}

async function selectBranchWithShortcuts(
  allChoices: Array<{
    name: string;
    value: string;
    description?: string;
    disabled?: boolean;
  }>,
): Promise<string> {
  const { createPrompt, useState, useKeypress, isEnterKey, usePrefix } =
    await import("@inquirer/core");

  const supportsColor = chalk.level > 0;

  const branchSelectPrompt = createPrompt<
    string,
    {
      message: string;
      choices: Array<{
        name: string;
        value: string;
        description?: string;
        disabled?: boolean;
      }>;
      pageSize?: number;
    }
  >((config, done) => {
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [status, setStatus] = useState<"idle" | "done">("idle");
    const prefix = usePrefix({});

    useKeypress((key) => {
      if (key.name === "n") {
        setStatus("done");
        done("__create_new__");
        return;
      }
      if (key.name === "m") {
        setStatus("done");
        done("__manage_worktrees__");
        return;
      }
      if (key.name === "c") {
        setStatus("done");
        done("__cleanup_prs__");
        return;
      }
      if (key.name === "q") {
        setStatus("done");
        done("__exit__");
        return;
      }

      if (key.name === "up" || key.name === "k") {
        // 最上部で停止（ループしない）
        if (selectedIndex > 0) {
          setSelectedIndex(selectedIndex - 1);
        }
        return;
      }
      if (key.name === "down" || key.name === "j") {
        // 選択可能な項目数に基づいて制限
        const selectableChoices = config.choices.filter(
          (c) =>
            c.value !== "__header__" &&
            c.value !== "__separator__" &&
            !c.disabled,
        );
        // 最下部で停止（ループしない）
        if (selectedIndex < selectableChoices.length - 1) {
          setSelectedIndex(selectedIndex + 1);
        }
        return;
      }

      if (isEnterKey(key)) {
        // 選択可能な項目のみから選択
        const selectableChoices = config.choices.filter(
          (c) =>
            c.value !== "__header__" &&
            c.value !== "__separator__" &&
            !c.disabled,
        );
        const selectedChoice = selectableChoices[selectedIndex];
        if (selectedChoice) {
          setStatus("done");
          done(selectedChoice.value);
        }
        return;
      }
    });

    if (status === "done") {
      return `${prefix} ${config.message}`;
    }

    // ヘッダー行とセパレーター行を探す
    const headerChoice = config.choices.find((c) => c.value === "__header__");
    const separatorChoice = config.choices.find(
      (c) => c.value === "__separator__",
    );

    // 選択可能な項目のみをフィルタリング
    const selectableChoices = config.choices.filter(
      (c) =>
        c.value !== "__header__" && c.value !== "__separator__" && !c.disabled,
    );

    const maxChoiceWidth = selectableChoices.reduce((max, choice) => {
      const width = stringWidth(stripAnsi(choice.name));
      return width > max ? width : max;
    }, 0);

    const pageSize = config.pageSize || 15;

    let output = `${prefix} ${config.message}\n`;
    output +=
      "Actions: (n) Create new branch, (m) Manage worktrees, (c) Clean up merged PRs, (a) Account management, (q) Exit\n";
    output += "\n";

    // ヘッダー行とセパレーター行を表示
    if (headerChoice) {
      output += `  ${headerChoice.name}\n`;
    }
    if (separatorChoice) {
      output += `  ${separatorChoice.name}\n`;
    }

    // 選択可能な項目のみを表示（ページネーション付き）
    const selectableStartIndex = Math.max(
      0,
      selectedIndex - Math.floor(pageSize / 2),
    );
    const selectableEndIndex = Math.min(
      selectableChoices.length,
      selectableStartIndex + pageSize,
    );
    const visibleSelectableChoices = selectableChoices.slice(
      selectableStartIndex,
      selectableEndIndex,
    );

    visibleSelectableChoices.forEach((choice, index) => {
      const globalIndex = selectableStartIndex + index;
      const line = formatBranchChoiceLine(choice.name, {
        isSelected: globalIndex === selectedIndex,
        supportsColor,
        maxWidth: maxChoiceWidth,
      });
      output += `${line}\n`;
    });

    return output;
  });

  return await branchSelectPrompt({
    message: "Select a branch:",
    choices: allChoices,
    pageSize: 15,
  });
}

type HighlightOptions = {
  isSelected: boolean;
  supportsColor: boolean;
  maxWidth: number;
};

function padToWidth(value: string, width: number): string {
  const currentWidth = stringWidth(value);
  if (currentWidth >= width) {
    return value;
  }
  return value + " ".repeat(width - currentWidth);
}

export function formatBranchChoiceLine(
  name: string,
  { isSelected, maxWidth }: HighlightOptions,
): string {
  const plain = stripAnsi(name);
  const paddedPlain = padToWidth(plain, maxWidth);
  if (isSelected) {
    return `> ${paddedPlain}`;
  }

  return `  ${paddedPlain}`;
}

export async function selectBranchType(): Promise<BranchType> {
  return await select({
    message: "Select branch type:",
    choices: [
      {
        name: "🚀 Feature",
        value: "feature",
        description: "A new feature branch",
      },
      {
        name: "🔥 Hotfix",
        value: "hotfix",
        description: "A critical bug fix",
      },
      {
        name: "📦 Release",
        value: "release",
        description: "A release preparation branch",
      },
    ],
  });
}

export async function selectVersionBumpType(
  currentVersion: string,
): Promise<"patch" | "minor" | "major"> {
  const versionParts = currentVersion.split(".");
  const major = parseInt(versionParts[0] || "0");
  const minor = parseInt(versionParts[1] || "0");
  const patch = parseInt(versionParts[2] || "0");

  return await select({
    message: `Current version: ${currentVersion}. Select version bump type:`,
    choices: [
      {
        name: `📌 Patch (${major}.${minor}.${patch + 1})`,
        value: "patch",
        description: "Bug fixes and minor changes",
      },
      {
        name: `📈 Minor (${major}.${minor + 1}.0)`,
        value: "minor",
        description: "New features, backwards compatible",
      },
      {
        name: `🚀 Major (${major + 1}.0.0)`,
        value: "major",
        description: "Breaking changes",
      },
    ],
  });
}

export async function inputBranchName(type: BranchType): Promise<string> {
  return await input({
    message: `Enter ${type} name:`,
    validate: (value: string) => {
      if (!value.trim()) {
        return "Branch name cannot be empty";
      }
      if (/[\s\\/:*?"<>|]/.test(value.trim())) {
        return 'Branch name cannot contain spaces or special characters (\\/:*?"<>|)';
      }
      return true;
    },
    transformer: (value: string) => value.trim(),
  });
}

export async function selectBaseBranch(
  branches: BranchInfo[],
): Promise<string> {
  const mainBranches = branches.filter(
    (b) =>
      b.type === "local" &&
      (b.branchType === "main" || b.branchType === "develop"),
  );

  if (mainBranches.length === 0) {
    throw new Error("No main or develop branch found");
  }

  if (mainBranches.length === 1 && mainBranches[0]) {
    return mainBranches[0].name;
  }

  return await select({
    message: "Select base branch:",
    choices: mainBranches.map((branch) => ({
      name: branch.name,
      value: branch.name,
      description: `${branch.branchType} branch`,
    })),
  });
}

export async function confirmWorktreeCreation(
  branchName: string,
  worktreePath: string,
): Promise<boolean> {
  return await confirm({
    message: `Create worktree for "${branchName}" at "${worktreePath}"?`,
    default: true,
  });
}

export async function confirmWorktreeRemoval(
  worktreePath: string,
): Promise<boolean> {
  return await confirm({
    message: `Remove worktree at "${worktreePath}"?`,
    default: false,
  });
}

export async function getNewBranchConfig(): Promise<NewBranchConfig> {
  const type = await selectBranchType();

  // リリースブランチの場合は、バージョン選択後に自動生成されるため、
  // ここでは仮の値を返す
  if (type === "release") {
    return {
      type,
      taskName: "version-placeholder",
      branchName: "release/version-placeholder",
    };
  }

  const taskName = await inputBranchName(type);
  const branchName = `${type}/${taskName}`;

  return {
    type,
    taskName,
    branchName,
  };
}

export async function confirmSkipPermissions(): Promise<boolean> {
  return await confirm({
    message:
      "Skip Claude Code permissions check (--dangerously-skip-permissions)?",
    default: false,
  });
}

export async function selectWorktreeForManagement(
  worktrees: Array<{
    branch: string;
    path: string;
    isAccessible?: boolean;
    invalidReason?: string;
  }>,
): Promise<string | "back"> {
  const {
    createPrompt,
    useState,
    useKeypress,
    isEnterKey,
    usePrefix,
    isUpKey,
    isDownKey,
  } = await import("@inquirer/core");

  // Custom prompt that handles 'q' key
  const customSelect = createPrompt<string | null, SelectPromptConfig<string>>(
    (config, done) => {
      const [selectedIndex, setSelectedIndex] = useState<number>(0);
      const [status, setStatus] = useState<"idle" | "done">("idle");
      const prefix = usePrefix({});

      useKeypress((key) => {
        if (status === "done") return;

        if (key.name === "q" || (key.name === "c" && key.ctrl)) {
          setStatus("done");
          done(null); // Return null for cancelled
          return;
        }

        if (isEnterKey(key)) {
          const selectedChoice = config.choices[selectedIndex];
          if (!selectedChoice) {
            return;
          }
          if (selectedChoice.disabled) {
            // Don't allow selection of disabled items
            return;
          }
          setStatus("done");
          done(selectedChoice.value);
          return;
        }

        if (isUpKey(key)) {
          let newIndex =
            selectedIndex > 0 ? selectedIndex - 1 : config.choices.length - 1;
          // Skip disabled items
          while (
            config.choices[newIndex]?.disabled &&
            newIndex !== selectedIndex
          ) {
            newIndex = newIndex > 0 ? newIndex - 1 : config.choices.length - 1;
          }
          setSelectedIndex(newIndex);
        } else if (isDownKey(key)) {
          let newIndex =
            selectedIndex < config.choices.length - 1 ? selectedIndex + 1 : 0;
          // Skip disabled items
          while (
            config.choices[newIndex]?.disabled &&
            newIndex !== selectedIndex
          ) {
            newIndex = newIndex < config.choices.length - 1 ? newIndex + 1 : 0;
          }
          setSelectedIndex(newIndex);
        }
      });

      if (status === "done") {
        return "";
      }

      const message = config.message;
      const choicesDisplay = config.choices
        .map((choice, index) => {
          const isSelected = index === selectedIndex;
          const pointer = isSelected ? "❯" : " ";
          const nameDisplay = choice.disabled
            ? chalk.gray(choice.name)
            : isSelected
              ? chalk.cyan(choice.name)
              : choice.name;
          const description = choice.description
            ? `\n  ${choice.disabled ? chalk.gray(choice.description) : chalk.gray(choice.description)}`
            : "";
          return `${pointer} ${nameDisplay}${isSelected ? description : ""}`;
        })
        .join("\n");

      return `${prefix} ${message}\n${choicesDisplay}`;
    },
  );

  const choices: Array<SelectChoice<string>> = worktrees.map((w, index) => {
    const isInvalid = w.isAccessible === false;
    return {
      name: isInvalid
        ? `${index + 1}. ✗ ${w.branch}`
        : `${index + 1}. ${w.branch}`,
      value: w.branch,
      description: isInvalid
        ? `${w.path} (${w.invalidReason || "Inaccessible"})`
        : w.path,
      disabled: isInvalid,
    };
  });

  const result = await customSelect({
    message: "Select worktree to manage (q to go back):",
    choices,
  });

  return result === null ? "back" : result;
}

export async function selectWorktreeAction(): Promise<
  "open" | "remove" | "remove-branch" | "back"
> {
  const result = await createQuitableSelect({
    message: "What would you like to do (q to go back)?",
    choices: [
      {
        name: "📂 Open in AI tool",
        value: "open" as const,
        description: "Launch the selected AI tool in this worktree",
      },
      {
        name: "🗑️  Remove worktree",
        value: "remove" as const,
        description: "Delete this worktree only",
      },
      {
        name: "🔥 Remove worktree and branch",
        value: "remove-branch" as const,
        description: "Delete both worktree and branch",
      },
    ],
  });

  return result === null ? "back" : result;
}

export async function confirmBranchRemoval(
  branchName: string,
): Promise<boolean> {
  return await confirm({
    message: `Are you sure you want to delete the branch "${branchName}"? This cannot be undone.`,
    default: false,
  });
}

export async function selectChangesAction(): Promise<
  "status" | "commit" | "stash" | "discard" | "continue"
> {
  return await select({
    message: "Changes detected in worktree. What would you like to do?",
    choices: [
      {
        name: "📋 View changes (git status)",
        value: "status",
        description: "Show modified files",
      },
      {
        name: "💾 Commit changes",
        value: "commit",
        description: "Create a new commit",
      },
      {
        name: "📦 Stash changes",
        value: "stash",
        description: "Save changes for later",
      },
      {
        name: "🗑️  Discard changes",
        value: "discard",
        description: "Discard all changes (careful!)",
      },
      {
        name: "➡️  Continue without action",
        value: "continue",
        description: "Return to main menu",
      },
    ],
  });
}

export async function inputCommitMessage(): Promise<string> {
  return await input({
    message: "Enter commit message:",
    validate: (value: string) => {
      if (!value.trim()) {
        return "Commit message cannot be empty";
      }
      return true;
    },
  });
}

export async function confirmDiscardChanges(): Promise<boolean> {
  return await confirm({
    message:
      "Are you sure you want to discard all changes? This cannot be undone.",
    default: false,
  });
}

export async function confirmContinue(message = "Continue?"): Promise<boolean> {
  return await confirm({
    message,
    default: true,
  });
}

export async function selectCleanupTargets(
  targets: CleanupTarget[],
): Promise<CleanupTarget[] | null> {
  if (targets.length === 0) {
    return [];
  }

  const {
    createPrompt,
    useState,
    useKeypress,
    isEnterKey,
    usePrefix,
    isUpKey,
    isDownKey,
    isSpaceKey,
  } = await import("@inquirer/core");

  // Custom checkbox prompt with q key support
  const customCheckbox = createPrompt<CleanupTarget[] | null, {
    message: string;
    choices: Array<{
      name: string;
      value: CleanupTarget;
      disabled?: boolean | string;
      checked: boolean;
    }>;
  }>((config, done) => {
    const [selectedIndices, setSelectedIndices] = useState<Set<number>>(
      new Set(
        config.choices
          .map((c, i) => (!c.disabled && c.checked ? i : -1))
          .filter((i) => i >= 0)
      )
    );
    const [cursorIndex, setCursorIndex] = useState<number>(0);
    const [status, setStatus] = useState<"idle" | "done">("idle");
    const prefix = usePrefix({});

    useKeypress((key) => {
      if (status === "done") return;

      // Handle q key to go back
      if (key.name === "q" || (key.name === "c" && key.ctrl)) {
        setStatus("done");
        done(null);
        return;
      }

      // Handle Enter key to confirm
      if (isEnterKey(key)) {
        const selected = Array.from(selectedIndices)
          .map((i) => config.choices[i])
          .filter((choice): choice is NonNullable<typeof choice> => choice !== undefined)
          .map((choice) => choice.value);
        setStatus("done");
        done(selected);
        return;
      }

      // Handle Space key to toggle selection
      if (isSpaceKey(key)) {
        const choice = config.choices[cursorIndex];
        if (choice && !choice.disabled) {
          const newSelection = new Set(selectedIndices);
          if (newSelection.has(cursorIndex)) {
            newSelection.delete(cursorIndex);
          } else {
            newSelection.add(cursorIndex);
          }
          setSelectedIndices(newSelection);
        }
        return;
      }

      // Handle up/down navigation
      if (isUpKey(key)) {
        const newIndex = cursorIndex > 0 ? cursorIndex - 1 : config.choices.length - 1;
        setCursorIndex(newIndex);
      } else if (isDownKey(key)) {
        const newIndex = cursorIndex < config.choices.length - 1 ? cursorIndex + 1 : 0;
        setCursorIndex(newIndex);
      }
    });

    if (status === "done") {
      return "";
    }

    const message = config.message;
    const instructions = "Space to select, Enter to confirm, q to go back";

    const choicesDisplay = config.choices
      .map((choice, index) => {
        const isCursor = index === cursorIndex;
        const isSelected = selectedIndices.has(index);
        const pointer = isCursor ? "❯" : " ";
        const checkbox = isSelected ? "◉" : "◯";

        const nameDisplay = choice.disabled
          ? chalk.gray(choice.name)
          : isCursor
            ? chalk.cyan(choice.name)
            : choice.name;

        const disabledText = choice.disabled && typeof choice.disabled === "string"
          ? chalk.gray(` (${choice.disabled})`)
          : "";

        return `${pointer}${checkbox} ${nameDisplay}${disabledText}`;
      })
      .join("\n");

    return `${prefix} ${message}\n${chalk.gray(instructions)}\n${choicesDisplay}`;
  });

  const choices = targets.map((target) => ({
    name: `${target.branch} (PR #${target.pullRequest.number}: ${target.pullRequest.title})`,
    value: target,
    disabled: target.hasUncommittedChanges ? "Has uncommitted changes" : false,
    checked: !target.hasUncommittedChanges,
  }));

  return await customCheckbox({
    message: "Select worktrees to clean up (merged PRs):",
    choices,
  });
}

export async function confirmCleanup(
  targets: CleanupTarget[],
): Promise<boolean> {
  const message =
    targets.length === 1 && targets[0]
      ? `Delete worktree and branch "${targets[0].branch}"?`
      : `Delete ${targets.length} worktrees and their branches?`;

  return await confirm({
    message,
    default: false,
  });
}

export async function confirmRemoteBranchDeletion(
  targets: CleanupTarget[],
): Promise<boolean> {
  const message =
    targets.length === 1 && targets[0]
      ? `Also delete remote branch "${targets[0].branch}"?`
      : `Also delete ${targets.length} remote branches?`;

  return await confirm({
    message,
    default: false,
  });
}

export async function confirmPushUnpushedCommits(
  targets: CleanupTarget[],
): Promise<boolean> {
  const branchesWithUnpushed = targets.filter((t) => t.hasUnpushedCommits);

  if (branchesWithUnpushed.length === 0) {
    return false;
  }

  const message =
    branchesWithUnpushed.length === 1 && branchesWithUnpushed[0]
      ? `Push unpushed commits in "${branchesWithUnpushed[0].branch}" before deletion?`
      : `Push unpushed commits in ${branchesWithUnpushed.length} branches before deletion?`;

  return await confirm({
    message,
    default: true,
  });
}

export async function confirmProceedWithoutPush(
  branchName: string,
): Promise<boolean> {
  return await confirm({
    message: `Failed to push "${branchName}". Proceed with deletion anyway?`,
    default: false,
  });
}

export async function selectReleaseAction(): Promise<
  "complete" | "continue" | "nothing"
> {
  return await select({
    message: "What would you like to do with this release branch?",
    choices: [
      {
        name: "🚀 Complete release - Push and create PR to main",
        value: "complete",
        description: "Start the release process",
      },
      {
        name: "⏸️  Save and continue later",
        value: "continue",
        description: "Keep the branch for future work",
      },
      {
        name: "❌ Exit without action",
        value: "nothing",
        description: "Just exit",
      },
    ],
  });
}

export async function selectSession(
  sessions: SessionData[],
): Promise<SessionData | null> {
  if (sessions.length === 0) {
    return null;
  }

  console.log("\n" + chalk.bold.cyan("Recent Claude Code Sessions"));
  console.log(chalk.gray("Select a session to resume:\n"));

  // Collect enhanced session information with categorization
  const categorizedSessions: CategorizedSession[] = [];

  for (let index = 0; index < sessions.length; index++) {
    const session = sessions[index];
    if (!session) continue;

    if (!session.lastWorktreePath || !session.lastBranch) {
      // Create a fallback category for incomplete sessions
      const fallbackInfo: import("../../git.js").EnhancedSessionInfo = {
        hasUncommittedChanges: false,
        uncommittedChangesCount: 0,
        hasUnpushedCommits: false,
        unpushedCommitsCount: 0,
        latestCommitMessage: null,
        branchType: "other",
      };

      categorizedSessions.push({
        session,
        sessionInfo: fallbackInfo,
        category: categorizeSession(fallbackInfo),
        index,
      });
      continue;
    }

    try {
      const { getEnhancedSessionInfo } = await import("../../git.js");
      const sessionInfo = await getEnhancedSessionInfo(
        session.lastWorktreePath,
        session.lastBranch,
      );
      const category = categorizeSession(sessionInfo);

      categorizedSessions.push({
        session,
        sessionInfo,
        category,
        index,
      });
    } catch {
      // Fallback for sessions where enhanced info is not available
      const fallbackInfo: import("../../git.js").EnhancedSessionInfo = {
        hasUncommittedChanges: false,
        uncommittedChangesCount: 0,
        hasUnpushedCommits: false,
        unpushedCommitsCount: 0,
        latestCommitMessage: null,
        branchType: "other",
      };

      categorizedSessions.push({
        session,
        sessionInfo: fallbackInfo,
        category: categorizeSession(fallbackInfo),
        index,
      });
    }
  }

  // Group and sort sessions
  const groupedSessions = groupAndSortSessions(categorizedSessions);

  // Create choices with grouping
  const groupedChoices = createGroupedChoices(groupedSessions);

  // No cancel option - use q key to go back

  const selectedIndex = await createQuitableSelect({
    message: "Select session (q to go back):",
    choices: groupedChoices.map((choice) => {
      const result: {
        name: string;
        value: string;
        description?: string;
        disabled?: boolean | string;
      } = {
        name: choice.name,
        value: choice.value,
      };
      if (choice.description) result.description = choice.description;
      if (choice.disabled) result.disabled = choice.disabled;
      return result;
    }),
    pageSize: 12,
  });

  if (selectedIndex === null) {
    // User pressed q - user wants to go back
    return null;
  }

  const index = parseInt(selectedIndex);
  return sessions[index] ?? null;
}

/**
 * Select Claude Code conversation from history
 */
export async function selectClaudeConversation(
  worktreePath: string,
): Promise<import("../../claude-history.js").ClaudeConversation | null> {
  try {
    const { getConversationsForProject, isClaudeHistoryAvailable } =
      await import("../../claude-history.js");

    // Check if Claude Code history is available
    if (!(await isClaudeHistoryAvailable())) {
      console.log(
        chalk.yellow("⚠️  Claude Code history not found on this system"),
      );
      console.log(
        chalk.gray(
          "   Using standard Claude Code resume functionality instead...",
        ),
      );
      return null;
    }

    console.log("\n" + chalk.bold.cyan("🔄 Resume Claude Code Conversation"));
    console.log(chalk.gray("Select a conversation to resume:\n"));

    // Get conversations for the current project
    const conversations = await getConversationsForProject(worktreePath);

    if (conversations.length === 0) {
      console.log(chalk.yellow("📝 No conversations found for this project"));
      console.log(chalk.gray("   Starting a new conversation instead..."));
      return null;
    }

    // Categorize conversations by recency
    const categorizedConversations =
      categorizeConversationsByActivity(conversations);

    // Create grouped choices
    const choices = createConversationChoices(categorizedConversations);

    // No cancel option - use q key to go back

    // Single selection prompt
    const selectedValue = await createQuitableSelect({
      message: "Choose conversation to resume (q to go back):",
      choices: choices.map((choice) => {
        const result: {
          name: string;
          value: string;
          description?: string;
          disabled?: boolean | string;
        } = {
          name: choice.name,
          value: choice.value,
        };
        if (choice.description) result.description = choice.description;
        if (choice.disabled) result.disabled = choice.disabled;
        return result;
      }),
      pageSize: 15,
    });

    if (selectedValue === null) {
      // Handle q key - user wants to go back
      return null;
    }

    const selectedIndex = parseInt(selectedValue);
    const selectedConversation = conversations[selectedIndex] || null;

    if (!selectedConversation) {
      return null;
    }

    // Clear screen before showing preview
    console.clear();

    // Show enhanced preview
    console.log(chalk.bold.cyan("📖 Conversation Preview"));
    console.log(
      chalk.gray("─".repeat(Math.min(80, process.stdout.columns || 80))),
    );
    console.log();

    const { getDetailedConversation } = await import("../../claude-history.js");
    const detailed = await getDetailedConversation(selectedConversation);
    if (detailed) {
      displayConversationPreview(detailed.messages);
    }

    console.log();
    console.log(
      chalk.gray("─".repeat(Math.min(80, process.stdout.columns || 80))),
    );

    // Simple confirmation - use q to go back
    try {
      const shouldResume = await confirm({
        message: `Resume "${selectedConversation.title}"?`,
        default: true,
      });

      if (shouldResume) {
        return selectedConversation;
      } else {
        // User chose not to resume, go back to conversation selection
        console.clear();
        return await selectClaudeConversation(worktreePath);
      }
    } catch {
      // Handle q key - go back to conversation selection
      console.clear();
      return await selectClaudeConversation(worktreePath);
    }
  } catch {
    console.error(chalk.red("Failed to load Claude Code conversations:"));
    console.log(
      chalk.gray("Using standard Claude Code resume functionality instead..."),
    );
    return null;
  }
}

/**
 * Display conversation messages with scrollable interface
 */
export async function displayConversationMessages(
  conversation: import("../../claude-history.js").ClaudeConversation,
): Promise<boolean> {
  try {
    const { getDetailedConversation } = await import("../../claude-history.js");
    const detailedConversation = await getDetailedConversation(conversation);

    if (!detailedConversation || !detailedConversation.messages) {
      console.log(chalk.red("Unable to load conversation messages"));
      return false;
    }

    console.clear();
    console.log(chalk.bold.cyan(`📖 ${conversation.title}`));
    console.log(
      chalk.gray(
        `${conversation.messageCount} messages • ${formatTimeAgo(conversation.lastActivity)}`,
      ),
    );
    console.log(chalk.gray("─".repeat(80)));
    console.log();

    // Create scrollable message viewer
    return await createMessageViewer(detailedConversation.messages);
  } catch {
    console.error(chalk.red("Failed to display conversation messages:"));
    return false;
  }
}

/**
 * Create scrollable message viewer component
 */
async function createMessageViewer(
  messages: import("../../claude-history.js").ClaudeMessage[],
): Promise<boolean> {
  console.clear();
  console.log(
    chalk.bold.cyan(`📖 Conversation History (${messages.length} messages)`),
  );
  console.log(chalk.gray("─".repeat(80)));
  console.log();

  // Show recent messages (last 10)
  const recentMessages = messages.slice(-10);

  recentMessages.forEach((message) => {
    const isUser = message.role === "user";
    const roleSymbol = isUser ? ">" : "⏺";
    const roleColor = isUser ? chalk.blue : chalk.cyan;

    // Format message content
    let content = "";
    if (typeof message.content === "string") {
      content = message.content;
    } else if (Array.isArray(message.content)) {
      content = message.content.map((item) => item.text || "").join(" ");
    }

    // Handle special content types
    let displayContent = content;
    let toolInfo = "";

    if (content.startsWith("🔧 Used tool:")) {
      const toolName = content.replace("🔧 Used tool: ", "");
      toolInfo = chalk.yellow(`[Tool: ${toolName}]`);
      displayContent = ""; // Don't show content for tool calls
    } else if (content.length > 60) {
      // Truncate long messages
      displayContent = content.substring(0, 57) + "...";
    }

    // Format like Claude Code
    const roleDisplay = roleColor(roleSymbol);

    // Display the message with Claude Code formatting
    if (toolInfo) {
      console.log(`${roleDisplay} ${toolInfo}`);
    } else if (displayContent.trim()) {
      console.log(`${roleDisplay} ${displayContent}`);
    }

    // Add spacing between messages like Claude Code
    console.log();
  });

  if (messages.length > 10) {
    console.log();
    console.log(
      chalk.gray(`... and ${messages.length - 10} more messages above`),
    );
  }

  console.log();
  console.log(chalk.gray("─".repeat(80)));
  console.log();

  // Simple confirmation
  return await confirm({
    message: "Resume this conversation?",
    default: true,
  });
}

/**
 * Display conversation preview (ccresume style)
 */
function displayConversationPreview(
  messages: import("../../claude-history.js").ClaudeMessage[],
): void {
  // Get terminal height and calculate available space for messages
  const terminalHeight = process.stdout.rows || 24; // Default to 24 if unavailable
  const headerLines = 3; // Title + separator + empty line
  const footerLines = 3; // Empty line + separator + confirmation prompt
  const availableLines = Math.max(
    6,
    terminalHeight - headerLines - footerLines,
  );

  // Be very conservative with message count to ensure newest messages are always visible
  const messagesToShow = Math.min(
    messages.length,
    Math.floor(availableLines / 4),
  ); // Very conservative estimate

  // Always start from the most recent messages and display in normal order (oldest to newest)
  const recentMessages = messages.slice(-messagesToShow);

  recentMessages.forEach((message) => {
    const isUser = message.role === "user";
    const roleSymbol = isUser ? ">" : "⏺";
    const roleColor = isUser ? chalk.blue : chalk.cyan;

    // Format message content
    let content = "";
    if (typeof message.content === "string") {
      content = message.content;
    } else if (Array.isArray(message.content)) {
      content = message.content.map((item) => item.text || "").join(" ");
    }

    // Handle special content types
    let displayContent = content;

    if (content.startsWith("🔧 Used tool:")) {
      const toolName = content.replace("🔧 Used tool: ", "");
      displayContent = chalk.yellow(`[Tool: ${toolName}]`);
    } else {
      // Aggressive truncation to ensure all messages fit
      const terminalWidth = process.stdout.columns || 80;
      const maxContentWidth = terminalWidth - 15; // Account for role label and spacing

      if (content.length > maxContentWidth) {
        displayContent = content.substring(0, maxContentWidth - 3) + "...";
      } else {
        displayContent = content;
      }

      // Limit multi-line content strictly
      const lines = displayContent.split("\n");
      if (lines.length > 1) {
        displayContent =
          lines[0] +
          (lines.length > 1
            ? "\n" + chalk.gray(`... (${lines.length - 1} more lines)`)
            : "");
      }
    }

    // Format like Claude Code
    const roleDisplay = roleColor(roleSymbol);

    // Handle multi-line display
    const contentLines = displayContent.split("\n");
    contentLines.forEach((line, index) => {
      if (index === 0) {
        console.log(`${roleDisplay} ${line}`);
      } else {
        // Indent continuation lines
        console.log(`${" ".repeat(roleDisplay.length - 8)} ${line}`); // Account for ANSI color codes
      }
    });
  });

  if (messages.length > messagesToShow) {
    console.log(
      chalk.gray(
        `... and ${messages.length - messagesToShow} more messages above`,
      ),
    );
  }

  console.log(); // Add spacing before footer
}

/**
 * Conversation category for grouping
 */
interface ConversationCategory {
  type: "recent" | "this-week" | "older";
  title: string;
  emoji: string;
}

/**
 * Categorized conversation with metadata
 */
interface CategorizedConversation {
  conversation: import("../../claude-history.js").ClaudeConversation;
  category: ConversationCategory;
  index: number;
}

/**
 * Categorize conversations by activity recency
 */
function categorizeConversationsByActivity(
  conversations: import("../../claude-history.js").ClaudeConversation[],
): CategorizedConversation[] {
  const now = Date.now();
  const oneHour = 60 * 60 * 1000;
  const oneDay = 24 * oneHour;
  const oneWeek = 7 * oneDay;

  return conversations
    .map((conversation, index) => {
      const age = now - conversation.lastActivity;

      let category: ConversationCategory;
      if (age < oneHour) {
        category = {
          type: "recent",
          title: "🔥 Very Recent (within 1 hour)",
          emoji: "🔥",
        };
      } else if (age < oneDay) {
        category = {
          type: "recent",
          title: "⚡ Recent (within 24 hours)",
          emoji: "⚡",
        };
      } else if (age < oneWeek) {
        category = {
          type: "this-week",
          title: "📅 This week",
          emoji: "📅",
        };
      } else {
        category = {
          type: "older",
          title: "📚 Older conversations",
          emoji: "📚",
        };
      }

      return {
        conversation,
        category,
        index,
      };
    })
    .sort((a, b) => {
      // First sort by category priority (recent -> this-week -> older)
      const categoryOrder = { recent: 0, "this-week": 1, older: 2 };
      const categoryDiff =
        categoryOrder[a.category.type] - categoryOrder[b.category.type];
      if (categoryDiff !== 0) return categoryDiff;

      // Within each category, sort by most recent first
      return b.conversation.lastActivity - a.conversation.lastActivity;
    });
}

/**
 * Create conversation choices with grouping
 */
function createConversationChoices(
  categorizedConversations: CategorizedConversation[],
): Array<{
  name: string;
  value: string;
  description?: string;
  disabled?: boolean;
}> {
  const choices: Array<{
    name: string;
    value: string;
    description?: string;
    disabled?: boolean;
  }> = [];

  // Group conversations by category
  const groups = new Map<string, CategorizedConversation[]>();
  groups.set("recent", []);
  groups.set("this-week", []);
  groups.set("older", []);

  for (const item of categorizedConversations) {
    const group = groups.get(item.category.type) || [];
    group.push(item);
    groups.set(item.category.type, group);
  }

  // Add groups in order
  const groupOrder = ["recent", "this-week", "older"] as const;

  for (const groupType of groupOrder) {
    const group = groups.get(groupType as string) || [];

    if (group.length === 0) continue;

    // Add group header
    const category = group[0]?.category;
    if (!category) continue;

    choices.push({
      name: `\n${category.title}`,
      value: `__header_${groupType}__`,
      disabled: true,
    });

    // Add conversations in this group
    for (const { conversation, index } of group) {
      const formatted = formatConversationDisplay(conversation, index);
      choices.push(formatted);
    }
  }

  // Add separator before cancel option
  if (choices.length > 0) {
    choices.push({
      name: "",
      value: "__separator__",
      disabled: true,
    });
  }

  return choices;
}

/**
 * Format conversation display
 */
function formatConversationDisplay(
  conversation: import("../../claude-history.js").ClaudeConversation,
  index: number,
): { name: string; value: string; description?: string } {
  const timeAgo = formatTimeAgo(conversation.lastActivity);
  const messageCount = conversation.messageCount;

  // Icon based on conversation content/title
  let icon = "💬";
  const lowerTitle = conversation.title.toLowerCase();
  if (
    lowerTitle.includes("bug") ||
    lowerTitle.includes("fix") ||
    lowerTitle.includes("error")
  ) {
    icon = "🐛";
  } else if (
    lowerTitle.includes("feature") ||
    lowerTitle.includes("implement") ||
    lowerTitle.includes("add")
  ) {
    icon = "🚀";
  } else if (
    lowerTitle.includes("doc") ||
    lowerTitle.includes("readme") ||
    lowerTitle.includes("comment")
  ) {
    icon = "📝";
  } else if (lowerTitle.includes("test") || lowerTitle.includes("spec")) {
    icon = "🧪";
  }

  // Format: "  💬 Conversation title (X messages, time ago)"
  const title =
    conversation.title.length > 40
      ? conversation.title.substring(0, 37) + "..."
      : conversation.title;

  const metadata = `(${messageCount} message${messageCount !== 1 ? "s" : ""}, ${chalk.gray(timeAgo)})`;

  // Create main display line
  const display = `  ${icon} ${chalk.cyan(title)} ${metadata}`;

  // Enhanced description with summary if available
  let description = "";
  if (conversation.summary && conversation.summary.trim()) {
    description =
      conversation.summary.length > 80
        ? conversation.summary.substring(0, 77) + "..."
        : conversation.summary;
  } else {
    // Fallback description based on title analysis
    if (lowerTitle.includes("bug") || lowerTitle.includes("fix")) {
      description = "Bug fix or error resolution";
    } else if (
      lowerTitle.includes("feature") ||
      lowerTitle.includes("implement")
    ) {
      description = "Feature development or implementation";
    } else if (lowerTitle.includes("doc") || lowerTitle.includes("readme")) {
      description = "Documentation or README updates";
    } else if (lowerTitle.includes("test")) {
      description = "Testing and test improvements";
    } else {
      description = `${messageCount} messages exchanged ${timeAgo}`;
    }
  }

  return {
    name: display,
    value: index.toString(),
    description: description,
  };
}

export async function selectClaudeExecutionMode(
  toolLabel: string = "Claude Code",
): Promise<{
  mode: "normal" | "continue" | "resume";
  skipPermissions: boolean;
} | null> {
  const {
    createPrompt,
    useState,
    useKeypress,
    isEnterKey,
    usePrefix,
    isUpKey,
    isDownKey,
  } = await import("@inquirer/core");

  // Custom prompt that handles 'q' key
  const customSelect = createPrompt<
    "normal" | "continue" | "resume" | null,
    SelectPromptConfig<"normal" | "continue" | "resume">
  >((config, done) => {
    const [selectedIndex, setSelectedIndex] = useState<number>(0);
    const [status, setStatus] = useState<"idle" | "done">("idle");
    const prefix = usePrefix({});

    useKeypress((key) => {
      if (status === "done") return;
      if (config.choices.length === 0) {
        setStatus("done");
        done(null);
        return;
      }

      if (key.name === "q" || (key.name === "c" && key.ctrl)) {
        setStatus("done");
        done(null); // Return null for cancelled
        return;
      }

      if (isEnterKey(key)) {
        const selectedChoice = config.choices[selectedIndex];
        if (!selectedChoice) {
          return;
        }
        setStatus("done");
        done(selectedChoice.value);
        return;
      }

      if (isUpKey(key)) {
        const newIndex =
          selectedIndex > 0 ? selectedIndex - 1 : config.choices.length - 1;
        setSelectedIndex(newIndex);
      } else if (isDownKey(key)) {
        const newIndex =
          selectedIndex < config.choices.length - 1 ? selectedIndex + 1 : 0;
        setSelectedIndex(newIndex);
      }
    });

    if (status === "done") {
      return "";
    }

    const message = config.message;
    const choicesDisplay = config.choices
      .map((choice, index) => {
        const isSelected = index === selectedIndex;
        const pointer = isSelected ? "❯" : " ";
        const nameColor = isSelected ? chalk.cyan : chalk.reset;
        return `${pointer} ${nameColor(choice.name)}${isSelected && choice.description ? "\n  " + chalk.gray(choice.description) : ""}`;
      })
      .join("\n");

    return `${prefix} ${message}\n${choicesDisplay}`;
  });

  const isCodexTool = toolLabel.toLowerCase().includes("codex");
  const choices: Array<SelectChoice<"normal" | "continue" | "resume">> = [
    {
      name: "🚀 Normal - Start a new session",
      value: "normal",
      description: `Launch ${toolLabel} normally`,
    },
    {
      name: isCodexTool
        ? "⏭️  Resume last session (codex resume --last)"
        : "⏭️  Continue - Continue most recent conversation (-c)",
      value: "continue",
      description: isCodexTool
        ? "Run Codex resume --last to continue the most recent session"
        : "Continue from the most recent conversation",
    },
    {
      name: isCodexTool
        ? "🔄 Resume - Choose a session to resume (codex resume)"
        : "🔄 Resume - Select conversation to resume (-r)",
      value: "resume",
      description: isCodexTool
        ? "Launch Codex resume and pick a session from the list"
        : "Interactively select a conversation to resume",
    },
  ];

  const mode = await customSelect({
    message: `Select ${toolLabel} execution mode (q to go back):`,
    choices,
  });

  if (mode === null) {
    // User pressed 'q' or Ctrl+C
    return null;
  }

  // Show appropriate flag hint per tool
  const flagHint = isCodexTool ? "--yolo" : "--dangerously-skip-permissions";
  const skipPermissions = await confirm({
    message: `Skip permission checks? (${flagHint})`,
    default: false,
  });

  return { mode: mode as "normal" | "continue" | "resume", skipPermissions };
}

type AIToolChoiceValue = "claude" | "codex";

export async function selectAITool(
  options: {
    claudeAvailable?: boolean;
    codexAvailable?: boolean;
  } = {},
): Promise<AIToolChoiceValue | null> {
  const claudeAvailable = options.claudeAvailable ?? true;
  const codexAvailable = options.codexAvailable ?? true;
  const {
    createPrompt,
    useState,
    useKeypress,
    isEnterKey,
    usePrefix,
    isUpKey,
    isDownKey,
  } = await import("@inquirer/core");

  const customSelect = createPrompt<
    AIToolChoiceValue | null,
    {
      message: string;
      choices: Array<{
        name: string;
        value: AIToolChoiceValue;
        description?: string;
      }>;
    }
  >((config, done) => {
    const [selectedIndex, setSelectedIndex] = useState<number>(0);
    const [status, setStatus] = useState<"idle" | "done">("idle");
    const prefix = usePrefix({});

    useKeypress((key) => {
      if (status === "done") return;

      if (key.name === "q" || (key.name === "c" && key.ctrl)) {
        setStatus("done");
        done(null);
        return;
      }

      if (isEnterKey(key)) {
        const selectedChoice = config.choices[selectedIndex];
        if (!selectedChoice) {
          setStatus("done");
          done(null);
          return;
        }
        setStatus("done");
        done(selectedChoice.value);
        return;
      }

      if (isUpKey(key)) {
        const newIndex =
          selectedIndex > 0 ? selectedIndex - 1 : config.choices.length - 1;
        setSelectedIndex(newIndex);
      } else if (isDownKey(key)) {
        const newIndex =
          selectedIndex < config.choices.length - 1 ? selectedIndex + 1 : 0;
        setSelectedIndex(newIndex);
      }
    });

    if (status === "done") {
      return "";
    }

    const message = config.message;
    const choicesDisplay = config.choices
      .map((choice, index) => {
        const isSelected = index === selectedIndex;
        const pointer = isSelected ? "❯" : " ";
        const nameDisplay = isSelected ? chalk.cyan(choice.name) : choice.name;
        const description =
          choice.description && isSelected
            ? `\n  ${chalk.gray(choice.description)}`
            : "";
        return `${pointer} ${nameDisplay}${description}`;
      })
      .join("\n");

    return `${prefix} ${message}\n${choicesDisplay}`;
  });

  const claudeDescription = claudeAvailable
    ? "Run via bunx @anthropic-ai/claude-code@latest"
    : "bunx 経由で実行（事前チェックなし）";
  const codexDescription = codexAvailable
    ? "Run via bunx @openai/codex@latest"
    : "bunx 経由で実行（事前チェックなし）";

  const choices: Array<{
    name: string;
    value: AIToolChoiceValue;
    description: string;
  }> = [
    {
      name: "Claude Code - Anthropic's AI coding assistant",
      value: "claude",
      description: claudeDescription,
    },
    {
      name: "Codex CLI - OpenAI's code generation tool",
      value: "codex",
      description: codexDescription,
    },
  ];

  const value = await customSelect({
    message: "Which AI tool would you like to use? (q to cancel)",
    choices,
  });

  return value;
}

function formatTimeAgo(timestamp: number): string {
  const now = Date.now();
  const diff = now - timestamp;

  const minutes = Math.floor(diff / (1000 * 60));
  const hours = Math.floor(diff / (1000 * 60 * 60));
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));
  const weeks = Math.floor(days / 7);
  const months = Math.floor(days / 30);

  if (minutes < 1) {
    return "just now";
  } else if (minutes < 60) {
    return `${minutes}m ago`;
  } else if (hours < 24) {
    return `${hours}h ago`;
  } else if (days === 1) {
    return "1 day ago";
  } else if (days < 7) {
    return `${days} days ago`;
  } else if (weeks === 1) {
    return "1 week ago";
  } else if (weeks < 4) {
    return `${weeks} weeks ago`;
  } else if (months === 1) {
    return "1 month ago";
  } else {
    return `${months} months ago`;
  }
}

/**
 * Get project icon based on repository name
 */
function getProjectIcon(repoName: string): string {
  const lowerName = repoName.toLowerCase();

  if (lowerName.includes("app") || lowerName.includes("mobile")) return "📱";
  if (lowerName.includes("api") || lowerName.includes("backend")) return "⚡";
  if (lowerName.includes("frontend") || lowerName.includes("ui")) return "🎨";
  if (lowerName.includes("cli") || lowerName.includes("tool")) return "🛠️";
  if (lowerName.includes("bot") || lowerName.includes("ai")) return "🤖";
  if (lowerName.includes("web")) return "🌐";
  if (lowerName.includes("doc") || lowerName.includes("guide")) return "📚";

  return "🚀";
}

/**
 * Session status categories for grouping
 */
interface SessionCategory {
  type: "active" | "ready" | "needs-attention";
  title: string;
  emoji: string;
  description: string;
}

/**
 * Enhanced session with category information
 */
interface CategorizedSession {
  session: SessionData;
  sessionInfo: import("../../git.js").EnhancedSessionInfo;
  category: SessionCategory;
  index: number;
}

/**
 * Determine session category based on git status
 */
function categorizeSession(
  sessionInfo: import("../../git.js").EnhancedSessionInfo,
): SessionCategory {
  if (sessionInfo.hasUncommittedChanges) {
    return {
      type: "active",
      title: "🔥 Active (uncommitted changes)",
      emoji: "🔥",
      description: "Sessions with ongoing work",
    };
  }

  if (sessionInfo.hasUnpushedCommits) {
    return {
      type: "needs-attention",
      title: "⚠️ Needs attention",
      emoji: "⚠️",
      description: "Sessions with unpushed commits",
    };
  }

  return {
    type: "ready",
    title: "✅ Ready to continue",
    emoji: "✅",
    description: "Clean sessions ready to resume",
  };
}

/**
 * Format session in new compact style
 */
function formatCompactSessionDisplay(
  session: SessionData,
  sessionInfo: import("../../git.js").EnhancedSessionInfo,
  index: number,
): { name: string; value: string; description?: string } {
  const repo = session.repositoryRoot.split("/").pop() || "unknown";
  const timeAgo = formatTimeAgo(session.timestamp);
  const branch = session.lastBranch || "unknown";

  const projectIcon = getProjectIcon(repo);

  // Create status info in parentheses
  let statusInfo = "";
  if (sessionInfo.hasUncommittedChanges) {
    const count = sessionInfo.uncommittedChangesCount;
    statusInfo = `📝 ${count} file${count !== 1 ? "s" : ""}`;
  } else if (sessionInfo.hasUnpushedCommits) {
    const count = sessionInfo.unpushedCommitsCount;
    statusInfo = `🔄 ${count} commit${count !== 1 ? "s" : ""}`;
  }

  // Format: "  🚀 project-name → branch-name     (status, time)"
  const projectBranch = `${chalk.cyan(repo)} → ${chalk.green(branch)}`;
  const padding = Math.max(0, 35 - repo.length - branch.length);
  const timeAndStatus = statusInfo
    ? `(${statusInfo}, ${chalk.gray(timeAgo)})`
    : `(${chalk.gray(timeAgo)})`;

  const display = `  ${projectIcon} ${projectBranch}${" ".repeat(padding)} ${timeAndStatus}`;

  return {
    name: display,
    value: index.toString(),
    description: session.lastWorktreePath || "",
  };
}

/**
 * Group and sort sessions by category and priority
 */
function groupAndSortSessions(
  categorizedSessions: CategorizedSession[],
): Map<string, CategorizedSession[]> {
  const groups = new Map<string, CategorizedSession[]>();

  // Initialize groups
  groups.set("active", []);
  groups.set("ready", []);
  groups.set("needs-attention", []);

  // Group sessions by category
  for (const session of categorizedSessions) {
    const group = groups.get(session.category.type) || [];
    group.push(session);
    groups.set(session.category.type, group);
  }

  // Sort within each group by timestamp (most recent first)
  for (const [key, sessions] of groups.entries()) {
    sessions.sort((a, b) => b.session.timestamp - a.session.timestamp);
    groups.set(key, sessions);
  }

  return groups;
}

/**
 * Create grouped choices for the prompt
 */
function createGroupedChoices(
  groupedSessions: Map<string, CategorizedSession[]>,
): Array<{
  name: string;
  value: string;
  description?: string;
  disabled?: boolean;
}> {
  const choices: Array<{
    name: string;
    value: string;
    description?: string;
    disabled?: boolean;
  }> = [];

  // Define group order for display
  const groupOrder = ["active", "needs-attention", "ready"] as const;

  for (const groupType of groupOrder) {
    const sessions = groupedSessions.get(groupType) || [];

    if (sessions.length === 0) continue;

    // Add group header
    const category = sessions[0]?.category;
    if (!category) continue;

    choices.push({
      name: `
${category.title}`,
      value: `__header_${groupType}__`,
      disabled: true,
    });

    // Add sessions in this group
    for (const { session, sessionInfo, index } of sessions) {
      const formatted = formatCompactSessionDisplay(
        session,
        sessionInfo,
        index,
      );
      choices.push(formatted);
    }
  }

  // Add a separator before cancel option
  if (choices.length > 0) {
    choices.push({
      name: "",
      value: "__separator__",
      disabled: true,
    });
  }

  return choices;
}

/**
 * worktreeパス衝突時の対処方法を選択
 * @param targetBranch - 作成しようとしているブランチ名
 * @param targetPath - 作成しようとしているパス
 * @param existingBranch - 既に存在するworktreeのブランチ名
 * @returns 選択されたアクション（'remove-and-create' | 'use-different-path' | 'cancel'）
 */
export async function selectWorktreePathConflictResolution(
  targetBranch: string,
  targetPath: string,
  existingBranch: string,
): Promise<"remove-and-create" | "use-different-path" | "cancel"> {
  console.log(chalk.yellow(`\n⚠️  Worktree path conflict detected:`));
  console.log(chalk.dim(`  Target path: ${targetPath}`));
  console.log(chalk.dim(`  Target branch: ${targetBranch}`));
  console.log(chalk.dim(`  Existing branch at this path: ${existingBranch}\n`));

  const action = await select({
    message: "How would you like to proceed?",
    choices: [
      {
        name: `Remove existing worktree and create new one for "${targetBranch}"`,
        value: "remove-and-create" as const,
        description: `Delete the worktree for "${existingBranch}" and create a new one`,
      },
      {
        name: "Use a different path (add suffix)",
        value: "use-different-path" as const,
        description:
          "Create the worktree at an alternative path (e.g., path-2)",
      },
      {
        name: "Cancel",
        value: "cancel" as const,
        description: "Return to main menu",
      },
    ],
  });

  return action;
}
