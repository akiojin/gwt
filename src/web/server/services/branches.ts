/**
 * Branch Service
 *
 * ブランチ一覧取得とマージステータス判定のビジネスロジック。
 * 既存のgit.tsとgithub.tsの機能を活用します。
 */

import { execa } from "execa";
import { getAllBranches, getRepositoryRoot } from "../../../git.js";
import { getPullRequestByBranch } from "../../../github.js";
import { listAdditionalWorktrees } from "../../../worktree.js";
import type { Branch } from "../../../types/api.js";
import type { BranchInfo, PullRequest } from "../../../cli/ui/types.js";

const DEFAULT_BASE_BRANCHES = ["main", "master", "develop", "dev"];

/**
 * すべてのブランチ一覧を取得（マージステータスとWorktree情報付き）
 */
export async function listBranches(): Promise<Branch[]> {
  const [branches, worktrees, repoRoot] = await Promise.all([
    getAllBranches(),
    listAdditionalWorktrees(),
    getRepositoryRoot(),
  ]);
  const upstreamMap = await collectUpstreamMap(repoRoot);
  const baseCandidates = buildBaseBranchCandidates(branches);
  const mainBranch = "main"; // TODO: 動的に取得
  const refCache = new Map<string, boolean>();
  const ancestorCache = new Map<string, boolean>();

  const branchList: Branch[] = await Promise.all(
    branches.map(async (branchInfo: BranchInfo) => {
      // Worktree情報
      const worktree = worktrees.find((wt) => wt.branch === branchInfo.name);

      // マージステータス判定（GitHub PRベース）
      let mergeStatus: "unmerged" | "merged" | "unknown" = "unknown";
      const pr = await getPullRequestByBranch(branchInfo.name);
      if (pr) {
        mergeStatus = pr.state === "MERGED" ? "merged" : "unmerged";
      } else if (branchInfo.name === mainBranch) {
        // メインブランチは常にmerged扱い
        mergeStatus = "merged";
      }

      const baseBranch = await resolveBaseBranch(
        branchInfo,
        pr,
        upstreamMap,
        baseCandidates,
        repoRoot,
        refCache,
        ancestorCache,
      );

      return {
        name: branchInfo.name,
        type: branchInfo.type,
        commitHash: "unknown", // BranchInfoには含まれていない
        commitMessage: null,
        author: null,
        commitDate: null,
        mergeStatus,
        worktreePath: worktree?.path || null,
        baseBranch: baseBranch ?? null,
        divergence: null, // TODO: divergence情報を取得
      };
    }),
  );

  return branchList;
}

/**
 * 特定のブランチ情報を取得
 */
export async function getBranchByName(
  branchName: string,
): Promise<Branch | null> {
  const branches = await listBranches();
  return branches.find((b) => b.name === branchName) || null;
}

async function collectUpstreamMap(
  repoRoot: string,
): Promise<Map<string, string>> {
  try {
    const { stdout } = await execa(
      "git",
      [
        "for-each-ref",
        "--format=%(refname:short)|%(upstream:short)",
        "refs/heads",
      ],
      { cwd: repoRoot },
    );

    return stdout
      .split("\n")
      .filter((line) => line.includes("|"))
      .reduce((map, line) => {
        const [branch, upstream] = line.split("|");
        if (branch && upstream) {
          map.set(branch.trim(), upstream.trim());
        }
        return map;
      }, new Map<string, string>());
  } catch {
    return new Map();
  }
}

function buildBaseBranchCandidates(branches: BranchInfo[]): string[] {
  const candidates = new Set<string>();
  for (const base of DEFAULT_BASE_BRANCHES) {
    candidates.add(base);
    candidates.add(`origin/${base}`);
  }

  for (const branch of branches) {
    if (branch.branchType === "main" || branch.branchType === "develop") {
      candidates.add(branch.name);
      if (branch.name.startsWith("origin/")) {
        const localName = branch.name.replace(/^origin\//, "");
        if (localName) {
          candidates.add(localName);
        }
      } else {
        candidates.add(`origin/${branch.name}`);
      }
    }
  }

  return Array.from(candidates);
}

async function resolveBaseBranch(
  branchInfo: BranchInfo,
  pr: PullRequest | null,
  upstreamMap: Map<string, string>,
  baseCandidates: string[],
  repoRoot: string,
  refCache: Map<string, boolean>,
  ancestorCache: Map<string, boolean>,
): Promise<string | null> {
  if (pr?.baseRefName) {
    return pr.baseRefName;
  }

  if (branchInfo.type === "local") {
    const upstream = upstreamMap.get(branchInfo.name);
    if (upstream) {
      return upstream;
    }
  }

  return inferBaseBranchFromCandidates(
    branchInfo.name,
    baseCandidates,
    repoRoot,
    refCache,
    ancestorCache,
  );
}

async function inferBaseBranchFromCandidates(
  branchName: string,
  baseCandidates: string[],
  repoRoot: string,
  refCache: Map<string, boolean>,
  ancestorCache: Map<string, boolean>,
): Promise<string | null> {
  for (const candidate of baseCandidates) {
    if (!candidate || candidate === branchName) {
      continue;
    }

    const exists = await refExists(candidate, repoRoot, refCache);
    if (!exists) {
      continue;
    }

    const isAncestor = await isAncestorRef(
      candidate,
      branchName,
      repoRoot,
      ancestorCache,
    );
    if (isAncestor) {
      return candidate;
    }
  }

  return null;
}

async function refExists(
  refName: string,
  repoRoot: string,
  cache: Map<string, boolean>,
): Promise<boolean> {
  if (cache.has(refName)) {
    return cache.get(refName)!;
  }

  try {
    await execa("git", ["rev-parse", "--verify", refName], { cwd: repoRoot });
    cache.set(refName, true);
    return true;
  } catch {
    cache.set(refName, false);
    return false;
  }
}

async function isAncestorRef(
  ancestor: string,
  branch: string,
  repoRoot: string,
  cache: Map<string, boolean>,
): Promise<boolean> {
  const cacheKey = `${ancestor}->${branch}`;
  if (cache.has(cacheKey)) {
    return cache.get(cacheKey)!;
  }

  try {
    await execa("git", ["merge-base", "--is-ancestor", ancestor, branch], {
      cwd: repoRoot,
    });
    cache.set(cacheKey, true);
    return true;
  } catch {
    cache.set(cacheKey, false);
    return false;
  }
}
