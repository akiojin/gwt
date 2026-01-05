/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { Match, Switch, createMemo, createSignal } from "solid-js";
import { BranchListScreen } from "../../../screens/solid/BranchListScreen.js";
import { LogScreen } from "../../../screens/solid/LogScreen.js";
import { LogDetailScreen } from "../../../screens/solid/LogDetailScreen.js";
import { SelectorScreen } from "../../../screens/solid/SelectorScreen.js";
import { EnvironmentScreen } from "../../../screens/solid/EnvironmentScreen.js";
import { ProfileScreen } from "../../../screens/solid/ProfileScreen.js";
import { SettingsScreen } from "../../../screens/solid/SettingsScreen.js";
import { WorktreeCreateScreen } from "../../../screens/solid/WorktreeCreateScreen.js";
import { WorktreeDeleteScreen } from "../../../screens/solid/WorktreeDeleteScreen.js";
import { LoadingIndicatorScreen } from "../../../screens/solid/LoadingIndicator.js";
import { ConfirmScreen } from "../../../screens/solid/ConfirmScreen.js";
import { InputScreen } from "../../../screens/solid/InputScreen.js";
import { ErrorScreen } from "../../../screens/solid/ErrorScreen.js";
import type { BranchItem, Statistics } from "../../../types.js";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";

const createBranch = (name: string): BranchItem => ({
  name,
  type: "local",
  branchType: "feature",
  isCurrent: false,
  icons: [],
  hasChanges: false,
  label: name,
  value: name,
});

const buildStats = (branches: BranchItem[]): Statistics => ({
  localCount: branches.filter((branch) => branch.type === "local").length,
  remoteCount: branches.filter((branch) => branch.type === "remote").length,
  worktreeCount: branches.filter((branch) => branch.worktree).length,
  changesCount: branches.filter((branch) => branch.hasChanges).length,
  lastUpdated: new Date(),
});

const createLogEntry = (id: string, summary: string): FormattedLogEntry => ({
  id,
  raw: { message: summary },
  timestamp: Date.now(),
  timeLabel: "12:00:00",
  levelLabel: "INFO",
  category: "test",
  message: summary,
  summary,
  json: '{\n  "message": "hello"\n}',
});

type ScreenId =
  | "branch-list"
  | "log-list"
  | "log-detail"
  | "selector"
  | "environment"
  | "profiles"
  | "settings"
  | "worktree-create"
  | "worktree-delete"
  | "loading"
  | "confirm"
  | "input"
  | "error";

const renderScreens = async () => {
  let setIndex: (value: number) => void = () => {};

  const branches = [createBranch("feature/one")];
  const stats = buildStats(branches);
  const logEntry = createLogEntry("1", "log entry");

  const testSetup = await testRender(
    () => {
      const [index, setIndexSignal] = createSignal(0);
      setIndex = setIndexSignal;

      const screenId = createMemo(
        () =>
          ([
            "branch-list",
            "log-list",
            "log-detail",
            "selector",
            "environment",
            "profiles",
            "settings",
            "worktree-create",
            "worktree-delete",
            "loading",
            "confirm",
            "input",
            "error",
          ][index()] ?? "branch-list") as ScreenId,
      );

      return (
        <Switch fallback={<ErrorScreen error="Unknown screen" />}>
          <Match when={screenId() === "branch-list"}>
            <BranchListScreen
              branches={branches}
              stats={stats}
              onSelect={() => {}}
              version="1.2.3"
              workingDirectory="/tmp/repo"
            />
          </Match>
          <Match when={screenId() === "log-list"}>
            <LogScreen
              entries={[logEntry]}
              onBack={() => {}}
              onSelect={() => {}}
              onCopy={() => {}}
              selectedDate="2026-01-05"
            />
          </Match>
          <Match when={screenId() === "log-detail"}>
            <LogDetailScreen
              entry={logEntry}
              onBack={() => {}}
              onCopy={() => {}}
            />
          </Match>
          <Match when={screenId() === "selector"}>
            <SelectorScreen
              title="Select item"
              items={[
                { label: "Option A", value: "a" },
                { label: "Option B", value: "b" },
              ]}
              onSelect={() => {}}
            />
          </Match>
          <Match when={screenId() === "environment"}>
            <EnvironmentScreen
              variables={[
                { key: "API_KEY", value: "secret" },
                { key: "REGION", value: "us-east-1" },
              ]}
            />
          </Match>
          <Match when={screenId() === "profiles"}>
            <ProfileScreen
              profiles={[
                { name: "default", displayName: "Default", isActive: true },
                { name: "dev", displayName: "Dev" },
              ]}
            />
          </Match>
          <Match when={screenId() === "settings"}>
            <SettingsScreen
              settings={[
                { label: "Theme", value: "theme" },
                { label: "Telemetry", value: "telemetry" },
              ]}
            />
          </Match>
          <Match when={screenId() === "worktree-create"}>
            <WorktreeCreateScreen
              branchName="feature/one"
              baseBranch="main"
              onChange={() => {}}
              onSubmit={() => {}}
            />
          </Match>
          <Match when={screenId() === "worktree-delete"}>
            <WorktreeDeleteScreen
              branchName="feature/one"
              worktreePath="/tmp/worktree"
              onConfirm={() => {}}
            />
          </Match>
          <Match when={screenId() === "loading"}>
            <LoadingIndicatorScreen message="Loading data" delay={0} />
          </Match>
          <Match when={screenId() === "confirm"}>
            <ConfirmScreen message="Proceed?" onConfirm={() => {}} />
          </Match>
          <Match when={screenId() === "input"}>
            <InputScreen
              message="Enter value"
              value="hello"
              onChange={() => {}}
              onSubmit={() => {}}
              label="Value"
            />
          </Match>
          <Match when={screenId() === "error"}>
            <ErrorScreen error="Something went wrong" />
          </Match>
        </Switch>
      );
    },
    { width: 80, height: 20 },
  );

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    setIndex,
    cleanup,
  };
};

describe("Solid all screens integration", () => {
  it("renders every screen", async () => {
    const { setIndex, renderOnce, captureCharFrame, cleanup } =
      await renderScreens();

    const expectations: Array<{ index: number; value: string }> = [
      { index: 0, value: "gwt - Branch Selection" },
      { index: 1, value: "gwt - Log Viewer" },
      { index: 2, value: "gwt - Log Detail" },
      { index: 3, value: "Option A" },
      { index: 4, value: "API_KEY=secret" },
      { index: 5, value: "Default (active)" },
      { index: 6, value: "Telemetry" },
      { index: 7, value: "gwt - Worktree Create" },
      { index: 8, value: "/tmp/worktree" },
      { index: 9, value: "Loading data" },
      { index: 10, value: "Proceed?" },
      { index: 11, value: "Enter value" },
      { index: 12, value: "Error: Something went wrong" },
    ];

    try {
      for (const expectation of expectations) {
        setIndex(expectation.index);
        await renderOnce();
        const frame = captureCharFrame();
        expect(frame).toContain(expectation.value);
      }
    } finally {
      cleanup();
    }
  });
});
