import { describe, it, expect, mock, spyOn } from "bun:test";
import type { SelectionResult } from "../../src/cli/ui/App.solid.js";
import type { CliRendererConfig } from "@opentui/core";

const waitForUserAcknowledgement = mock(async () => {});
const writeTerminalLineMock = mock();
const renderSolidAppMock = mock(
  async (
    props: { onExit?: (value?: SelectionResult | undefined) => void },
    _config?: CliRendererConfig,
  ) => {
    props.onExit?.(undefined);
  },
);

const stdinMock = {
  isTTY: true,
  resume: mock(),
  pause: mock(),
  on: mock(),
  removeAllListeners: mock(),
  setRawMode: mock(),
} as unknown as NodeJS.ReadStream;

const stdoutMock = {
  write: mock(),
  isTTY: true,
} as unknown as NodeJS.WriteStream;

const stderrMock = {
  write: mock(),
  isTTY: true,
} as unknown as NodeJS.WriteStream;

mock.module("../../src/utils/terminal.js", () => ({
  getTerminalStreams: mock(() => ({
    stdin: stdinMock,
    stdout: stdoutMock,
    stderr: stderrMock,
    usingFallback: false,
    exitRawMode: mock(),
  })),
  resetTerminalModes: mock(),
  waitForUserAcknowledgement,
  writeTerminalLine: writeTerminalLineMock,
  createChildStdio: mock(() => ({
    stdin: "inherit" as const,
    stdout: "inherit" as const,
    stderr: "inherit" as const,
    cleanup: mock(),
  })),
}));

mock.module("../../src/opentui/index.solid.js", () => ({
  renderSolidApp: renderSolidAppMock,
}));

mock.module("../../src/git.js", () => ({
  isGitRepository: mock(async () => true),
  getRepositoryRoot: mock(async () => "/repo"),
  branchExists: mock(async () => false),
  fetchAllRemotes: mock(async () => undefined),
  pullFastForward: mock(async () => undefined),
  getBranchDivergenceStatuses: mock(async () => []),
  hasUncommittedChanges: mock(async () => false),
  hasUnpushedCommits: mock(async () => false),
  getUncommittedChangesCount: mock(async () => 0),
  getUnpushedCommitsCount: mock(async () => 0),
}));

mock.module("../../src/worktree.js", () => ({
  isProtectedBranchName: mock(() => false),
  switchToProtectedBranch: mock(async () => undefined),
  resolveWorktreePathForBranch: mock(async () => ({ path: null })),
  listAllWorktrees: mock(async () => []),
  repairWorktreePath: mock(async () => ({ repaired: false, removed: false })),
}));

mock.module("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = mock(async () => "/tmp/worktree");
  },
}));

mock.module("../../src/config/tools.js", () => ({
  getCodingAgentById: mock(async () => ({
    id: "codex-cli",
    displayName: "Codex CLI",
    type: "command",
    command: "codex",
    modeArgs: { normal: [] },
  })),
  getSharedEnvironment: mock(async () => ({})),
}));

mock.module("../../src/config/index.js", () => ({
  saveSession: mock(async () => {}),
  loadSession: mock(async () => null),
}));

mock.module("../../src/claude.js", () => ({
  launchClaudeCode: mock(async () => undefined),
  ClaudeError: class ClaudeError extends Error {},
}));

mock.module("../../src/codex.js", () => ({
  launchCodexCLI: mock(async () => undefined),
  CodexError: class CodexError extends Error {},
}));

mock.module("../../src/gemini.js", () => ({
  launchGeminiCLI: mock(async () => undefined),
  GeminiError: class GeminiError extends Error {},
}));

mock.module("../../src/launcher.js", () => ({
  launchCodingAgent: mock(async () => undefined),
}));

mock.module("../../src/utils/session.js", () => ({
  findLatestCodexSession: mock(async () => null),
  findLatestClaudeSession: mock(async () => null),
  findLatestGeminiSession: mock(async () => null),
  findLatestClaudeSessionId: mock(async () => null),
}));

mock.module("../../src/cli/ui/utils/continueSession.js", () => ({
  resolveContinueSessionId: mock(async () => null),
}));

mock.module("../../src/cli/ui/utils/modelOptions.js", () => ({
  normalizeModelId: mock(() => null),
}));

mock.module("../../src/services/dependency-installer.js", () => ({
  installDependenciesForWorktree: mock(async () => ({
    skipped: true,
    manager: null,
    lockfile: null,
    reason: "missing-lockfile",
  })),
  DependencyInstallError: class DependencyInstallError extends Error {},
}));

mock.module("../../src/utils/error-utils.js", () => ({
  isGitRelatedError: mock(() => false),
  isRecoverableError: mock(() => false),
}));

describe("OpenTUI console config", () => {
  it("disables the built-in OpenTUI console", async () => {
    const processExitSpy = spyOn(process, "exit").mockImplementation(
      (() => undefined) as unknown as (code?: number) => never,
    );

    let capturedConfig: CliRendererConfig | undefined;
    renderSolidAppMock.mockImplementation(
      async (
        props: { onExit?: (value?: SelectionResult | undefined) => void },
        config?: CliRendererConfig,
      ) => {
        capturedConfig = config;
        props.onExit?.(undefined);
      },
    );

    const originalArgv = [...process.argv];
    process.argv = ["node", "index.js"];

    try {
      const { main } = await import("../../src/index.js");
      await expect(main()).resolves.toBeUndefined();

      expect(capturedConfig?.useConsole).toBe(false);
      expect(capturedConfig?.openConsoleOnError).toBe(false);
    } finally {
      process.argv = originalArgv;
      processExitSpy.mockRestore();
    }
  });
});
