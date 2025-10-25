import { execa } from "execa";
import chalk from "chalk";
import type { GitHubPRResponse } from "../ui/types.js";

/**
 * GitHub CLI操作のための低レベルRepository
 */
export class GitHubRepository {
  async execute(args: string[]): Promise<string> {
    try {
      const { stdout } = await execa("gh", args);
      return stdout;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new Error(
        `GitHub CLI command failed: gh ${args.join(" ")}\n${message}`,
      );
    }
  }

  async isAvailable(): Promise<boolean> {
    try {
      await this.execute(["--version"]);
      return true;
    } catch {
      return false;
    }
  }

  async isAuthenticated(): Promise<boolean> {
    try {
      await this.execute(["api", "user"]);
      return true;
    } catch {
      return false;
    }
  }

  async fetchPullRequests(options: {
    state?: "all" | "open" | "closed" | "merged";
    limit?: number;
    head?: string;
  }): Promise<GitHubPRResponse[]> {
    const args = ["pr", "list"];

    if (options.state) {
      args.push("--state", options.state);
    }

    if (options.head) {
      args.push("--head", options.head);
    }

    args.push(
      "--json",
      "number,title,state,headRefName,mergedAt,author",
      "--limit",
      String(options.limit || 100),
    );

    const stdout = await this.execute(args);

    if (!stdout || stdout.trim() === "") {
      return [];
    }

    return JSON.parse(stdout);
  }

  async fetchRemoteUpdates(): Promise<void> {
    try {
      await execa("git", ["fetch", "--all", "--prune"]);
    } catch {
      if (process.env.DEBUG_CLEANUP) {
        console.log(
          chalk.yellow(
            "Debug: Failed to fetch remote updates, continuing anyway",
          ),
        );
      }
    }
  }
}
