import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { PassThrough } from "node:stream";

// execa を事前にモック
vi.mock("execa", () => ({
  execa: vi.fn(),
}));

describe("worktree spinner integration", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("should start and stop spinner during worktree creation", async () => {
    const stopSpinner = vi.fn();

    const spinnerModule = await import("../../src/utils/spinner.js");
    const startSpinnerSpy = vi
      .spyOn(spinnerModule, "startSpinner")
      .mockImplementation((message: string) => {
        stopSpinner.mockName(`stopSpinner:${message}`);
        return stopSpinner;
      });

    const gitModule = await import("../../src/git.js");
    vi.spyOn(gitModule, "ensureGitignoreEntry").mockResolvedValue(undefined);

    const execaModule = await import("execa");
    const execaMock = execaModule.execa as any as ReturnType<typeof vi.fn>;
    execaMock.mockImplementation(() => {
      let resolvePromise: (value: unknown) => void = () => {};
      const stdout = new PassThrough();
      const stderr = new PassThrough();

      const promise = new Promise<unknown>((resolve) => {
        resolvePromise = resolve;
      });

      const child = promise as Promise<unknown> & {
        stdout: PassThrough;
        stderr: PassThrough;
      };
      child.stdout = stdout;
      child.stderr = stderr;

      setTimeout(() => {
        stdout.emit("data", Buffer.from("progress"));
        stdout.end();
        resolvePromise({ stdout: "", stderr: "", exitCode: 0 });
      }, 0);

      return child;
    });

    const worktree = await import("../../src/worktree");

    await worktree.createWorktree({
      branchName: "feature/test",
      worktreePath: "/tmp/worktrees/feature-test",
      repoRoot: "/repo",
      isNewBranch: false,
      baseBranch: "main",
    });

    expect(startSpinnerSpy).toHaveBeenCalledTimes(1);
    expect(startSpinnerSpy.mock.calls[0]?.[0]).toMatch(/worktree/i);
    expect(stopSpinner).toHaveBeenCalled();
    expect(execaMock).toHaveBeenCalled();
  });
});
