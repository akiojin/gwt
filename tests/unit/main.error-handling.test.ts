import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

describe("main error handling", () => {
  beforeEach(() => {
    vi.resetModules();
  });

  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
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
      },
      undefined,
    ];

    const waitForUserAcknowledgement = vi.fn(async () => {});
    const stdinMock = {
      isTTY: true,
      resume: vi.fn(),
      pause: vi.fn(),
      on: vi.fn(),
      removeAllListeners: vi.fn(),
      setRawMode: vi.fn(),
    } as unknown as NodeJS.ReadStream;
    const stdoutMock = {
      write: vi.fn(),
    } as unknown as NodeJS.WriteStream;
    const stderrMock = {
      write: vi.fn(),
    } as unknown as NodeJS.WriteStream;

    vi.doMock("../../src/utils/terminal.js", () => {
      return {
        getTerminalStreams: vi.fn(() => ({
          stdin: stdinMock,
          stdout: stdoutMock,
          stderr: stderrMock,
          usingFallback: false,
          exitRawMode: vi.fn(),
        })),
        waitForUserAcknowledgement,
        createChildStdio: vi.fn(() => ({
          stdin: "inherit" as const,
          stdout: "inherit" as const,
          stderr: "inherit" as const,
          cleanup: vi.fn(),
        })),
      };
    });

    const renderSpy = vi.fn((element: any) => {
      const next = selectionQueue.shift();
      if (element?.props?.onExit) {
        element.props.onExit(next);
      }
      return {
        unmount: vi.fn(),
        waitUntilExit: () => Promise.resolve(),
      };
    });

    vi.doMock("ink", () => ({
      render: renderSpy,
    }));

    vi.doMock("react", () => ({
      createElement: (type: any, props: any) => ({ type, props }),
    }));

    vi.doMock("../../src/ui/components/App.js", () => ({
      App: (props: unknown) => props,
    }));

    vi.doMock("../../src/git.js", () => ({
      isGitRepository: vi.fn(async () => true),
      getRepositoryRoot: vi.fn(async () => "/repo"),
      branchExists: vi.fn(async () => false),
      getCurrentBranch: vi.fn(async () => "main"),
    }));

    vi.doMock("../../src/worktree.js", () => ({
      worktreeExists: vi.fn(async () => null),
      generateWorktreePath: vi.fn(async (_repo: string, branch: string) => `/worktrees/${branch}`),
      createWorktree: vi.fn(async () => {}),
    }));

    const ensureWorktreeMock = vi.fn(async () => "/tmp/worktree");
    vi.doMock("../../src/services/WorktreeOrchestrator.js", () => ({
      WorktreeOrchestrator: class {
        ensureWorktree = ensureWorktreeMock;
      },
    }));

    vi.doMock("../../src/config/tools.js", () => ({
      getToolById: vi.fn(async () => ({
        id: "codex-cli",
        displayName: "Codex CLI",
        type: "command",
        command: "codex",
        modeArgs: { normal: [] },
      })),
    }));

    vi.doMock("../../src/config/index.js", () => ({
      saveSession: vi.fn(async () => {}),
    }));

    vi.doMock("../../src/claude.js", () => ({
      launchClaudeCode: vi.fn(async () => {}),
    }));

    const codexError = new Error("Codex failed");
    vi.doMock("../../src/codex.js", () => ({
      launchCodexCLI: vi.fn(async () => {
        throw codexError;
      }),
    }));

    vi.doMock("../../src/launcher.js", () => ({
      launchCustomAITool: vi.fn(async () => {}),
    }));

    const processExitSpy = vi
      .spyOn(process, "exit")
      .mockImplementation((() => undefined) as unknown as (code?: number) => never);

    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    const { main } = await import("../../src/index.js");

    await expect(main()).resolves.toBeUndefined();

    expect(waitForUserAcknowledgement).toHaveBeenCalled();
    expect(processExitSpy).not.toHaveBeenCalledWith(1);
    expect(consoleErrorSpy).toHaveBeenCalled();
    const errorMessages = consoleErrorSpy.mock.calls.map(([msg]) => String(msg ?? ""));
    expect(
      errorMessages.some((msg) => msg.includes("Workflow error, returning to main menu")),
    ).toBe(false);
    expect(renderSpy).toHaveBeenCalledTimes(2);

    processExitSpy.mockRestore();
    consoleErrorSpy.mockRestore();
  });
});
