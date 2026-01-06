/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import type { JSX } from "solid-js";
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

const renderScreen = async (render: () => JSX.Element) => {
  const testSetup = await testRender(() => render(), {
    width: 80,
    height: 20,
  });

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    cleanup,
  };
};

describe("Solid all screens integration", () => {
  it("renders every screen", async () => {
    const branches = [createBranch("feature/one")];
    const stats = buildStats(branches);
    const logEntry = createLogEntry("1", "log entry");

    const cases: Array<{
      id: ScreenId;
      render: () => JSX.Element;
      value: string;
    }> = [
      {
        id: "branch-list",
        render: () => (
          <BranchListScreen
            branches={branches}
            stats={stats}
            onSelect={() => {}}
            version="1.2.3"
            workingDirectory="/tmp/repo"
          />
        ),
        value: "gwt - Branch Selection",
      },
      {
        id: "log-list",
        render: () => (
          <LogScreen
            entries={[logEntry]}
            onBack={() => {}}
            onSelect={() => {}}
            onCopy={() => {}}
            selectedDate="2026-01-05"
          />
        ),
        value: "gwt - Log Viewer",
      },
      {
        id: "log-detail",
        render: () => (
          <LogDetailScreen
            entry={logEntry}
            onBack={() => {}}
            onCopy={() => {}}
          />
        ),
        value: "gwt - Log Detail",
      },
      {
        id: "selector",
        render: () => (
          <SelectorScreen
            title="Select item"
            items={[
              { label: "Option A", value: "a" },
              { label: "Option B", value: "b" },
            ]}
            onSelect={() => {}}
          />
        ),
        value: "Option A",
      },
      {
        id: "environment",
        render: () => (
          <EnvironmentScreen
            variables={[
              { key: "API_KEY", value: "secret" },
              { key: "REGION", value: "us-east-1" },
            ]}
          />
        ),
        value: "API_KEY=secret",
      },
      {
        id: "profiles",
        render: () => (
          <ProfileScreen
            profiles={[
              { name: "default", displayName: "Default", isActive: true },
              { name: "dev", displayName: "Dev" },
            ]}
          />
        ),
        value: "Default (active)",
      },
      {
        id: "settings",
        render: () => (
          <SettingsScreen
            settings={[
              { label: "Theme", value: "theme" },
              { label: "Telemetry", value: "telemetry" },
            ]}
          />
        ),
        value: "Telemetry",
      },
      {
        id: "worktree-create",
        render: () => (
          <WorktreeCreateScreen
            branchName="feature/one"
            baseBranch="main"
            onChange={() => {}}
            onSubmit={() => {}}
          />
        ),
        value: "gwt - Worktree Create",
      },
      {
        id: "worktree-delete",
        render: () => (
          <WorktreeDeleteScreen
            branchName="feature/one"
            worktreePath="/tmp/worktree"
            onConfirm={() => {}}
          />
        ),
        value: "/tmp/worktree",
      },
      {
        id: "loading",
        render: () => (
          <LoadingIndicatorScreen message="Loading data" delay={0} />
        ),
        value: "Loading data",
      },
      {
        id: "confirm",
        render: () => <ConfirmScreen message="Proceed?" onConfirm={() => {}} />,
        value: "Proceed?",
      },
      {
        id: "input",
        render: () => (
          <InputScreen
            message="Enter value"
            value="hello"
            onChange={() => {}}
            onSubmit={() => {}}
            label="Value"
          />
        ),
        value: "Enter value",
      },
      {
        id: "error",
        render: () => <ErrorScreen error="Something went wrong" />,
        value: "Error: Something went wrong",
      },
    ];

    try {
      for (const testCase of cases) {
        const { renderOnce, captureCharFrame, cleanup } = await renderScreen(
          testCase.render,
        );
        try {
          await renderOnce();
          const frame = captureCharFrame();
          expect(frame).toContain(testCase.value);
        } finally {
          cleanup();
        }
      }
    } finally {
      // no-op; per-case cleanup above
    }
  });
});
