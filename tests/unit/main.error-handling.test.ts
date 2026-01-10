import {
  describe,
  it,
  expect,
  mock,
  beforeEach,
  afterEach,
  spyOn,
} from "bun:test";
import type { SelectionResult } from "../../src/cli/ui/App.solid.js";

describe("main error handling", () => {
  beforeEach(() => {
    // resetModules not needed in bun;
  });

  afterEach(() => {
    // Module mocks are preserved across tests
  });

  it("AIツールの起動失敗時でもCLIが継続する", async () => {
    const selectionQueue = [
      {
        branch: "feature/test",
        displayName: "feature/test",
        branchType: "local" as const,
        tool: "codex-cli",
        mode: "normal" as const,
        skipPermissions: false,
        model: "gpt-5.2-codex",
      },
      undefined,
    ];

    const waitForUserAcknowledgement = mock(async () => {});
    const writeTerminalLineMock = mock();
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
    } as unknown as NodeJS.WriteStream;
    const stderrMock = {
      write: mock(),
    } as unknown as NodeJS.WriteStream;

    mock.module("../../src/utils/terminal.js", () => {
      return {
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
      };
    });

    const renderSpy = mock(
      async (props: {
        onExit?: (value?: SelectionResult | undefined) => void;
      }) => {
        const next = selectionQueue.shift();
        props.onExit?.(next);
      },
    );

    mock.module("../../src/opentui/index.solid.js", () => ({
      renderSolidApp: renderSpy,
    }));

    mock.module("../../src/git.js", () => ({
      isGitRepository: mock(async () => true),
      getRepositoryRoot: mock(async () => "/repo"),
      branchExists: mock(async () => false),
      getCurrentBranch: mock(async () => "main"),
      fetchAllRemotes: mock(async () => undefined),
      pullFastForward: mock(async () => undefined),
      getBranchDivergenceStatuses: mock(async () => []),
      hasUncommittedChanges: mock(async () => false),
      hasUnpushedCommits: mock(async () => false),
      getUncommittedChangesCount: mock(async () => 0),
      getUnpushedCommitsCount: mock(async () => 0),
      GitError: class GitError extends Error {
        constructor(
          message: string,
          public cause?: unknown,
        ) {
          super(message);
          this.name = "GitError";
        }
      },
    }));

    mock.module("../../src/worktree.js", () => ({
      worktreeExists: mock(async () => null),
      resolveWorktreePathForBranch: mock(async () => ({ path: null })),
      generateWorktreePath: mock(
        async (_repo: string, branch: string) => `/worktrees/${branch}`,
      ),
      createWorktree: mock(async () => {}),
      listAllWorktrees: mock(async () => []),
      listAdditionalWorktrees: mock(async () => []),
      isProtectedBranchName: mock(() => false),
      switchToProtectedBranch: mock(async () => undefined),
      WorktreeError: class WorktreeError extends Error {
        constructor(
          message: string,
          public cause?: unknown,
        ) {
          super(message);
          this.name = "WorktreeError";
        }
      },
    }));

    const ensureWorktreeMock = mock(async () => "/tmp/worktree");
    mock.module("../../src/services/WorktreeOrchestrator.js", () => ({
      WorktreeOrchestrator: class {
        ensureWorktree = ensureWorktreeMock;
      },
    }));

    mock.module("../../src/config/tools.js", () => ({
      CONFIG_DIR: "/tmp/gwt-test",
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
      launchClaudeCode: mock(async () => {}),
      ClaudeError: class ClaudeError extends Error {
        constructor(
          message: string,
          public cause?: unknown,
        ) {
          super(message);
          this.name = "ClaudeError";
        }
      },
    }));

    const codexError = new Error("Codex failed");
    mock.module("../../src/codex.js", () => ({
      launchCodexCLI: mock(async () => {
        throw codexError;
      }),
      CodexError: class CodexError extends Error {
        constructor(
          message: string,
          public cause?: unknown,
        ) {
          super(message);
          this.name = "CodexError";
        }
      },
    }));

    mock.module("../../src/launcher.js", () => ({
      launchCodingAgent: mock(async () => {}),
    }));

    const processExitSpy = spyOn(process, "exit").mockImplementation(
      (() => undefined) as unknown as (code?: number) => never,
    );

    const { main } = await import("../../src/index.js");

    await expect(main()).resolves.toBeUndefined();

    expect(waitForUserAcknowledgement).toHaveBeenCalled();
    expect(processExitSpy).not.toHaveBeenCalledWith(1);
    expect(writeTerminalLineMock).toHaveBeenCalled();
    expect(renderSpy).toHaveBeenCalledTimes(2);

    processExitSpy.mockRestore();
  }, 30000);
});
