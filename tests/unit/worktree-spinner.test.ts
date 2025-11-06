import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { PassThrough } from "node:stream";

describe("worktree spinner integration", () => {
  beforeEach(() => {
    vi.resetModules();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("should start and stop spinner during worktree creation", async () => {
    const stopSpinner = vi.fn();
    const startSpinner = vi.fn(() => stopSpinner);

    vi.doMock("../../src/utils/spinner.js", () => ({
      startSpinner: (message: string) => {
        startSpinner(message);
        return stopSpinner;
      },
    }));

    const ensureGitignoreEntry = vi.fn().mockResolvedValue(undefined);
    vi.doMock("../../src/git.js", async () => {
      const actual =
        await vi.importActual<typeof import("../../src/git.js")>(
          "../../src/git.js",
        );
      return {
        ...actual,
        ensureGitignoreEntry,
      };
    });

    const execaMock = vi.fn(() => {
      let resolvePromise: (value: unknown) => void;
      const stdout = new PassThrough();
      const stderr = new PassThrough();

      const promise = new Promise((resolve) => {
        resolvePromise = resolve;
      });

      const child: any = promise;
      child.stdout = stdout;
      child.stderr = stderr;
      child.then = promise.then.bind(promise);
      child.catch = promise.catch.bind(promise);
      child.finally = promise.finally.bind(promise);

      setTimeout(() => {
        stdout.emit("data", Buffer.from("progress"));
        stdout.end();
        resolvePromise({ stdout: "", stderr: "", exitCode: 0 });
      }, 0);

      return child;
    });

    vi.doMock("execa", () => ({ execa: execaMock }));

    const worktree = await import("../../src/worktree");

    await worktree.createWorktree({
      branchName: "feature/test",
      worktreePath: "/tmp/worktrees/feature-test",
      repoRoot: "/repo",
      isNewBranch: false,
      baseBranch: "main",
    });

    expect(startSpinner).toHaveBeenCalledTimes(1);
    expect(startSpinner.mock.calls[0]?.[0]).toMatch(/worktree/i);
    expect(stopSpinner).toHaveBeenCalled();
  });
});
