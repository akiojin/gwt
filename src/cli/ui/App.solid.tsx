/** @jsxImportSource @opentui/solid */
import { createEffect, createMemo, createSignal } from "solid-js";
import type { BranchItem, Statistics } from "./types.js";
import type { ToolStatus } from "./hooks/useToolStatus.js";
import type { FormattedLogEntry } from "../../logging/formatter.js";
import { calculateStatistics } from "./utils/statisticsCalculator.js";
import { BranchListScreen } from "./screens/solid/BranchListScreen.js";
import { LogScreen } from "./screens/solid/LogScreen.js";
import { LogDetailScreen } from "./screens/solid/LogDetailScreen.js";
import {
  SelectorScreen,
  type SelectorItem,
} from "./screens/solid/SelectorScreen.js";
import {
  EnvironmentScreen,
  type EnvironmentVariable,
} from "./screens/solid/EnvironmentScreen.js";
import {
  ProfileScreen,
  type ProfileItem,
} from "./screens/solid/ProfileScreen.js";
import {
  SettingsScreen,
  type SettingsItem,
} from "./screens/solid/SettingsScreen.js";
import { WorktreeCreateScreen } from "./screens/solid/WorktreeCreateScreen.js";
import { WorktreeDeleteScreen } from "./screens/solid/WorktreeDeleteScreen.js";
import { LoadingIndicatorScreen } from "./screens/solid/LoadingIndicator.js";
import { ConfirmScreen } from "./screens/solid/ConfirmScreen.js";
import { InputScreen } from "./screens/solid/InputScreen.js";
import { ErrorScreen } from "./screens/solid/ErrorScreen.js";

export type AppScreen =
  | "branch-list"
  | "log-list"
  | "log-detail"
  | "environment"
  | "profiles"
  | "settings"
  | "selector"
  | "worktree-create"
  | "worktree-delete"
  | "loading"
  | "confirm"
  | "input"
  | "error";

export interface AppSolidProps {
  onExit?: () => void;
  loadingIndicatorDelay?: number;
  initialScreen?: AppScreen;
  version?: string | null;
  workingDirectory?: string;
  branches?: BranchItem[];
  stats?: Statistics;
  toolStatuses?: ToolStatus[];
  logEntries?: FormattedLogEntry[];
  logSelectedDate?: string | null;
  environmentVariables?: EnvironmentVariable[];
  profiles?: ProfileItem[];
  settings?: SettingsItem[];
  selectorTitle?: string;
  selectorDescription?: string;
  selectorItems?: SelectorItem[];
  worktreeBranchName?: string;
  worktreeBaseBranch?: string;
  worktreePath?: string | null;
  confirmMessage?: string;
  inputMessage?: string;
  inputValue?: string;
  errorMessage?: string;
}

const DEFAULT_SCREEN: AppScreen = "branch-list";

const buildStats = (branches: BranchItem[]): Statistics => {
  const changedBranches = new Set(
    branches.filter((branch) => branch.hasChanges).map((branch) => branch.name),
  );
  return calculateStatistics(branches, changedBranches);
};

export function AppSolid(props: AppSolidProps) {
  const [currentScreen, setCurrentScreen] = createSignal<AppScreen>(
    props.initialScreen ?? DEFAULT_SCREEN,
  );
  const [screenStack, setScreenStack] = createSignal<AppScreen[]>([]);
  const [selectedLogEntry, setSelectedLogEntry] =
    createSignal<FormattedLogEntry | null>(null);
  const [worktreeBranchName, setWorktreeBranchName] = createSignal(
    props.worktreeBranchName ?? "",
  );
  const [inputValue, setInputValue] = createSignal(props.inputValue ?? "");

  createEffect(() => {
    if (props.initialScreen) {
      setScreenStack([]);
      setCurrentScreen(props.initialScreen);
    }
  });

  createEffect(() => {
    if (props.worktreeBranchName !== undefined) {
      setWorktreeBranchName(props.worktreeBranchName);
    }
  });

  createEffect(() => {
    if (props.inputValue !== undefined) {
      setInputValue(props.inputValue);
    }
  });

  const branches = createMemo(() => props.branches ?? []);
  const stats = createMemo(() => props.stats ?? buildStats(branches()));
  const logEntries = createMemo(() => props.logEntries ?? []);
  const selectorItems = createMemo(() => props.selectorItems ?? []);
  const environmentVariables = createMemo(
    () => props.environmentVariables ?? [],
  );
  const profiles = createMemo(() => props.profiles ?? []);
  const settings = createMemo(() => props.settings ?? []);

  const navigateTo = (screen: AppScreen) => {
    setScreenStack((prev) => [...prev, currentScreen()]);
    setCurrentScreen(screen);
  };

  const goBack = () => {
    const stack = screenStack();
    if (stack.length === 0) {
      return;
    }
    const previous = stack[stack.length - 1] ?? DEFAULT_SCREEN;
    setScreenStack(stack.slice(0, -1));
    setCurrentScreen(previous);
  };

  const renderCurrentScreen = () => {
    const screen = currentScreen();
    if (screen === "branch-list") {
      return (
        <BranchListScreen
          branches={branches()}
          stats={stats()}
          onSelect={() => {
            props.onExit?.();
          }}
          onQuit={props.onExit}
          onOpenLogs={() => navigateTo("log-list")}
          onOpenProfiles={() => navigateTo("profiles")}
          onRefresh={() => {
            // no-op placeholder
          }}
          loading={false}
          error={null}
          loadingIndicatorDelay={props.loadingIndicatorDelay}
          version={props.version}
          workingDirectory={props.workingDirectory}
          activeProfile={profiles().find((profile) => profile.isActive)?.name}
          toolStatuses={props.toolStatuses}
        />
      );
    }

    if (screen === "log-list") {
      return (
        <LogScreen
          entries={logEntries()}
          onBack={goBack}
          onSelect={(entry) => {
            setSelectedLogEntry(entry);
            navigateTo("log-detail");
          }}
          onCopy={() => {
            // no-op placeholder
          }}
          selectedDate={props.logSelectedDate ?? null}
          version={props.version}
        />
      );
    }

    if (screen === "log-detail") {
      return (
        <LogDetailScreen
          entry={selectedLogEntry()}
          onBack={goBack}
          onCopy={() => {
            // no-op placeholder
          }}
          version={props.version}
        />
      );
    }

    if (screen === "environment") {
      return (
        <EnvironmentScreen
          variables={environmentVariables()}
          onBack={goBack}
          version={props.version}
        />
      );
    }

    if (screen === "profiles") {
      return (
        <ProfileScreen
          profiles={profiles()}
          onBack={goBack}
          version={props.version}
        />
      );
    }

    if (screen === "settings") {
      return (
        <SettingsScreen
          settings={settings()}
          onBack={goBack}
          version={props.version}
        />
      );
    }

    if (screen === "selector") {
      return (
        <SelectorScreen
          title={props.selectorTitle ?? "Select item"}
          description={props.selectorDescription}
          items={selectorItems()}
          onSelect={() => goBack()}
          onBack={goBack}
          version={props.version}
        />
      );
    }

    if (screen === "worktree-create") {
      return (
        <WorktreeCreateScreen
          branchName={worktreeBranchName()}
          baseBranch={props.worktreeBaseBranch}
          onChange={setWorktreeBranchName}
          onSubmit={() => goBack()}
          onCancel={goBack}
          version={props.version}
        />
      );
    }

    if (screen === "worktree-delete") {
      return (
        <WorktreeDeleteScreen
          branchName={worktreeBranchName()}
          worktreePath={props.worktreePath}
          onConfirm={() => goBack()}
          version={props.version}
        />
      );
    }

    if (screen === "loading") {
      return (
        <LoadingIndicatorScreen
          message={props.inputMessage ?? "Loading..."}
          delay={props.loadingIndicatorDelay ?? 0}
        />
      );
    }

    if (screen === "confirm") {
      return (
        <ConfirmScreen
          message={props.confirmMessage ?? "Proceed?"}
          onConfirm={() => goBack()}
        />
      );
    }

    if (screen === "input") {
      return (
        <InputScreen
          message={props.inputMessage ?? "Enter value"}
          value={inputValue()}
          onChange={setInputValue}
          onSubmit={() => goBack()}
          label="Value"
        />
      );
    }

    if (screen === "error") {
      return <ErrorScreen error={props.errorMessage ?? "Unknown error"} />;
    }

    return <ErrorScreen error={`Unknown screen: ${screen}`} />;
  };

  return <>{renderCurrentScreen()}</>;
}
