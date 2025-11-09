import { describe, it, expect } from "vitest";
import { execa } from "execa";
import { join } from "path";

const hookScriptPath = join(
  process.cwd(),
  ".claude/hooks/block-git-branch-ops.sh",
);

describe("block-git-branch-ops.sh hook", () => {
  /**
   * Helper function to execute the hook script with a given tool input
   */
  async function runHook(
    toolName: string,
    command: string,
  ): Promise<{
    exitCode: number;
    stdout: string;
    stderr: string;
  }> {
    const input = JSON.stringify({
      tool_name: toolName,
      tool_input: { command },
    });

    try {
      const result = await execa(hookScriptPath, {
        input,
        reject: false,
      });
      return {
        exitCode: result.exitCode,
        stdout: result.stdout,
        stderr: result.stderr,
      };
    } catch (error: any) {
      return {
        exitCode: error.exitCode || 1,
        stdout: error.stdout || "",
        stderr: error.stderr || "",
      };
    }
  }

  describe("Interactive rebase blocking", () => {
    it("should block 'git rebase -i origin/main'", async () => {
      const result = await runHook("Bash", "git rebase -i origin/main");

      expect(result.exitCode).toBe(2);
      const jsonOutput = JSON.parse(result.stdout);
      expect(jsonOutput.decision).toBe("block");
      expect(jsonOutput.reason).toContain(
        "Interactive rebase against origin/main is not allowed",
      );
      expect(jsonOutput.stopReason).toContain(
        "Interactive rebase against origin/main initiated by LLMs is blocked",
      );
    });

    it("should block 'git rebase --interactive origin/main'", async () => {
      const result = await runHook(
        "Bash",
        "git rebase --interactive origin/main",
      );

      expect(result.exitCode).toBe(2);
      const jsonOutput = JSON.parse(result.stdout);
      expect(jsonOutput.decision).toBe("block");
    });

    it("should allow non-interactive rebase 'git rebase origin/main'", async () => {
      const result = await runHook("Bash", "git rebase origin/main");

      expect(result.exitCode).toBe(0);
    });

    it("should allow interactive rebase with different base", async () => {
      const result = await runHook("Bash", "git rebase -i develop");

      expect(result.exitCode).toBe(0);
    });
  });

  describe("Branch switching blocking", () => {
    it("should block 'git checkout main'", async () => {
      const result = await runHook("Bash", "git checkout main");

      expect(result.exitCode).toBe(2);
      const jsonOutput = JSON.parse(result.stdout);
      expect(jsonOutput.decision).toBe("block");
      expect(jsonOutput.reason).toContain(
        "Branch switching, creation, and worktree commands are not allowed",
      );
    });

    it("should block 'git switch develop'", async () => {
      const result = await runHook("Bash", "git switch develop");

      expect(result.exitCode).toBe(2);
      const jsonOutput = JSON.parse(result.stdout);
      expect(jsonOutput.decision).toBe("block");
    });

    it("should block 'git checkout -b new-branch'", async () => {
      const result = await runHook("Bash", "git checkout -b new-branch");

      expect(result.exitCode).toBe(2);
    });
  });

  describe("Branch operations blocking", () => {
    it("should block 'git branch -d test-branch'", async () => {
      const result = await runHook("Bash", "git branch -d test-branch");

      expect(result.exitCode).toBe(2);
      const jsonOutput = JSON.parse(result.stdout);
      expect(jsonOutput.decision).toBe("block");
    });

    it("should block 'git branch -D test-branch'", async () => {
      const result = await runHook("Bash", "git branch -D test-branch");

      expect(result.exitCode).toBe(2);
    });

    it("should block 'git branch -m old-name new-name'", async () => {
      const result = await runHook("Bash", "git branch -m old-name new-name");

      expect(result.exitCode).toBe(2);
    });

    it("should allow 'git branch' (list branches)", async () => {
      const result = await runHook("Bash", "git branch");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git branch --list'", async () => {
      const result = await runHook("Bash", "git branch --list");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git branch -a' (list all branches)", async () => {
      const result = await runHook("Bash", "git branch -a");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git branch --merged'", async () => {
      const result = await runHook("Bash", "git branch --merged");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git branch -r' (list remote branches)", async () => {
      const result = await runHook("Bash", "git branch -r");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git branch -v' (verbose list)", async () => {
      const result = await runHook("Bash", "git branch -v");

      expect(result.exitCode).toBe(0);
    });
  });

  describe("Worktree operations blocking", () => {
    it("should block 'git worktree add /tmp/test main'", async () => {
      const result = await runHook("Bash", "git worktree add /tmp/test main");

      expect(result.exitCode).toBe(2);
      const jsonOutput = JSON.parse(result.stdout);
      expect(jsonOutput.decision).toBe("block");
    });

    it("should block 'git worktree remove test'", async () => {
      const result = await runHook("Bash", "git worktree remove test");

      expect(result.exitCode).toBe(2);
    });
  });

  describe("Non-Bash tools", () => {
    it("should allow non-Bash tools", async () => {
      const input = JSON.stringify({
        tool_name: "Read",
        tool_input: { file_path: "/some/file.txt" },
      });

      const result = await execa(hookScriptPath, {
        input,
        reject: false,
      });

      expect(result.exitCode).toBe(0);
    });
  });

  describe("Safe git commands", () => {
    it("should allow 'git status'", async () => {
      const result = await runHook("Bash", "git status");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git log'", async () => {
      const result = await runHook("Bash", "git log");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git diff'", async () => {
      const result = await runHook("Bash", "git diff");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git add .'", async () => {
      const result = await runHook("Bash", "git add .");

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git commit -m \"message\"'", async () => {
      const result = await runHook("Bash", 'git commit -m "message"');

      expect(result.exitCode).toBe(0);
    });

    it("should allow 'git push'", async () => {
      const result = await runHook("Bash", "git push");

      expect(result.exitCode).toBe(0);
    });
  });

  describe("Compound commands", () => {
    it("should block when dangerous command is in chain", async () => {
      const result = await runHook("Bash", "git add . && git checkout main");

      expect(result.exitCode).toBe(2);
    });

    it("should allow safe command chains", async () => {
      const result = await runHook(
        "Bash",
        "git add . && git commit -m 'test' && git push",
      );

      expect(result.exitCode).toBe(0);
    });
  });
});
