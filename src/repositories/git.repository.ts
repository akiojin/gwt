import { execa } from "execa";
import { GitError } from "../git.js";

/**
 * Git操作のための低レベルRepository
 * execaの直接呼び出しをカプセル化
 */
export class GitRepository {
  async execute(args: string[], options?: { cwd?: string }): Promise<string> {
    try {
      const { stdout } = await execa("git", args, options);
      return stdout;
    } catch (error) {
      throw new GitError(`Git command failed: git ${args.join(" ")}`, error);
    }
  }

  async isRepository(): Promise<boolean> {
    try {
      await this.execute(["rev-parse", "--git-dir"]);
      return true;
    } catch {
      return false;
    }
  }

  async getRepositoryRoot(): Promise<string> {
    const stdout = await this.execute(["rev-parse", "--show-toplevel"]);
    return stdout.trim();
  }

  async getCurrentBranch(): Promise<string | null> {
    try {
      const stdout = await this.execute(["branch", "--show-current"]);
      return stdout.trim() || null;
    } catch {
      return null;
    }
  }

  async getBranches(options: { remote?: boolean }): Promise<string[]> {
    const args = ["branch"];
    if (options.remote) {
      args.push("-r");
    }
    args.push("--format=%(refname:short)");

    const stdout = await this.execute(args);
    return stdout
      .split("\n")
      .filter((line) => line.trim())
      .filter((line) => !line.includes("HEAD"));
  }

  async createBranch(branchName: string, baseBranch?: string): Promise<void> {
    const args = ["checkout", "-b", branchName];
    if (baseBranch) {
      args.push(baseBranch);
    }
    await this.execute(args);
  }

  async deleteBranch(branchName: string, force = false): Promise<void> {
    const args = ["branch", force ? "-D" : "-d", branchName];
    await this.execute(args);
  }

  async deleteRemoteBranch(branchName: string): Promise<void> {
    await this.execute(["push", "origin", "--delete", branchName]);
  }

  async getStatus(options?: { cwd?: string }): Promise<string> {
    return await this.execute(
      ["status", "--porcelain"],
      options?.cwd ? options : undefined,
    );
  }

  async hasChanges(workdir?: string): Promise<boolean> {
    const status = await this.getStatus(workdir ? { cwd: workdir } : undefined);
    return status.trim().length > 0;
  }

  async fetch(options?: { all?: boolean; prune?: boolean }): Promise<void> {
    const args = ["fetch"];
    if (options?.all) args.push("--all");
    if (options?.prune) args.push("--prune");
    await this.execute(args);
  }

  async push(options?: { upstream?: boolean; branch?: string }): Promise<void> {
    const args = ["push"];
    if (options?.upstream && options?.branch) {
      args.push("--set-upstream", "origin", options.branch);
    }
    await this.execute(args);
  }

  async commit(message: string, options?: { all?: boolean }): Promise<void> {
    const args = ["commit", "-m", message];
    if (options?.all) args.push("-a");
    await this.execute(args);
  }

  async add(files: string[] | "."): Promise<void> {
    const args = ["add"];
    if (Array.isArray(files)) {
      args.push(...files);
    } else {
      args.push(files);
    }
    await this.execute(args);
  }

  async stash(message?: string): Promise<void> {
    const args = ["stash", "push"];
    if (message) args.push("-m", message);
    await this.execute(args);
  }

  async checkout(target: string): Promise<void> {
    await this.execute(["checkout", target]);
  }

  async getChangedFilesCount(workdir?: string): Promise<number> {
    const status = await this.getStatus(workdir ? { cwd: workdir } : undefined);
    return status.split("\n").filter((line) => line.trim()).length;
  }
}
