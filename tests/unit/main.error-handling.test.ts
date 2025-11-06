import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { SelectionResult } from "../../src/ui/components/App.js";

type ViWithDoMock = typeof vi & { doMock?: typeof vi.mock };
const viWithDoMock = vi as unknown as ViWithDoMock;
if (!viWithDoMock.doMock) {
  viWithDoMock.doMock = vi.mock.bind(vi);
}

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

    viWithDoMock.doMock?.("../../src/utils/terminal.js", () => {
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

    const renderSpy = vi.fn(
      (
        element:
          | {
              props?: {
                onExit?: (value?: SelectionResult | undefined) => void;
              };
            }
          | undefined,
      ) => {
        const next = selectionQueue.shift();
        element?.props?.onExit?.(next);
        return {
          unmount: vi.fn(),
          waitUntilExit: () => Promise.resolve(),
        };
      },
    );

    viWithDoMock.doMock?.("ink", () => ({
      render: renderSpy,
    }));

    viWithDoMock.doMock?.("react", () => ({
      createElement: <P>(type: unknown, props: P) => ({ type, props }),
    }));

    viWithDoMock.doMock?.("../../src/ui/components/App.js", () => ({
      App: (props: unknown) => props,
    }));

    viWithDoMock.doMock?.("../../src/git.js", () => ({
      isGitRepository: vi.fn(async () => true),
      getRepositoryRoot: vi.fn(async () => "/repo"),
      branchExists: vi.fn(async () => false),
      getCurrentBranch: vi.fn(async () => "main"),
    }));

    viWithDoMock.doMock?.("../../src/worktree.js", async () => {
      const actual = await vi.importActual<
        typeof import("../../src/worktree.js")
      >("../../src/worktree.js");
      return {
        ...actual,
        worktreeExists: vi.fn(async () => null),
        generateWorktreePath: vi.fn(
          async (_repo: string, branch: string) => `/worktrees/${branch}`,
        ),
        createWorktree: vi.fn(async () => {}),
      };
    });

    const ensureWorktreeMock = vi.fn(async () => "/tmp/worktree");
    viWithDoMock.doMock?.("../../src/services/WorktreeOrchestrator.js", () => ({
      WorktreeOrchestrator: class {
        ensureWorktree = ensureWorktreeMock;
      },
    }));

    viWithDoMock.doMock?.("../../src/config/tools.js", () => ({
      getToolById: vi.fn(async () => ({
        id: "codex-cli",
        displayName: "Codex CLI",
        type: "command",
        command: "codex",
        modeArgs: { normal: [] },
      })),
    }));

    viWithDoMock.doMock?.("../../src/config/index.js", () => ({
      saveSession: vi.fn(async () => {}),
    }));

    viWithDoMock.doMock?.("../../src/claude.js", () => ({
      launchClaudeCode: vi.fn(async () => {}),
    }));

    const codexError = new Error("Codex failed");
    viWithDoMock.doMock?.("../../src/codex.js", () => ({
      launchCodexCLI: vi.fn(async () => {
        throw codexError;
      }),
    }));

    viWithDoMock.doMock?.("../../src/launcher.js", () => ({
      launchCustomAITool: vi.fn(async () => {}),
    }));

    const processExitSpy = vi
      .spyOn(process, "exit")
      .mockImplementation(
        (() => undefined) as unknown as (code?: number) => never,
      );

    const consoleErrorSpy = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});

    const { main } = await import("../../src/index.js");

    await expect(main()).resolves.toBeUndefined();

    expect(waitForUserAcknowledgement).toHaveBeenCalled();
    expect(processExitSpy).not.toHaveBeenCalledWith(1);
    expect(consoleErrorSpy).toHaveBeenCalled();
    const errorMessages = consoleErrorSpy.mock.calls.map(([msg]) =>
      String(msg ?? ""),
    );
    expect(
      errorMessages.some((msg) =>
        msg.includes("Workflow error, returning to main menu"),
      ),
    ).toBe(false);
    expect(renderSpy).toHaveBeenCalledTimes(2);

    processExitSpy.mockRestore();
    consoleErrorSpy.mockRestore();
  });
});
