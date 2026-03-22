import type { BranchInfo, BranchInventoryEntry, BranchInventoryResolutionAction, WorktreeInfo } from "./types";
import type { SidebarFilterType } from "./components/sidebarHelpers";
import { stripRemotePrefix } from "./components/sidebarHelpers";

export function branchInventoryKey(name: string): string {
  const trimmed = name.trim();
  return trimmed.startsWith("origin/") ? stripRemotePrefix(trimmed) : trimmed;
}

function buildInventoryEntry(
  key: string,
  localBranch: BranchInfo | null,
  remoteBranch: BranchInfo | null,
  worktrees: WorktreeInfo[],
): BranchInventoryEntry {
  const worktree = worktrees[0] ?? null;
  const worktreeCount = worktrees.length;
  let resolutionAction: BranchInventoryResolutionAction = "createWorktree";
  if (worktreeCount > 1) {
    resolutionAction = "resolveAmbiguity";
  } else if (worktree) {
    resolutionAction = "focusExisting";
  }

  return {
    id: key,
    canonical_name: key,
    primary_branch: localBranch ?? remoteBranch ?? {
      name: key,
      commit: "",
      is_current: false,
      is_agent_running: false,
      agent_status: "unknown",
      ahead: 0,
      behind: 0,
      divergence_status: "UpToDate",
      commit_timestamp: null,
      last_tool_usage: null,
    },
    local_branch: localBranch,
    remote_branch: remoteBranch,
    has_local: Boolean(localBranch),
    has_remote: Boolean(remoteBranch),
    worktree,
    worktree_count: worktreeCount,
    resolution_action: resolutionAction,
  };
}

export function buildBranchInventoryEntries(
  local: BranchInfo[],
  remote: BranchInfo[],
  worktrees: WorktreeInfo[],
  filter: SidebarFilterType,
): BranchInventoryEntry[] {
  const localByKey = new Map(local.map((branch) => [branchInventoryKey(branch.name), branch]));
  const remoteByKey = new Map(remote.map((branch) => [branchInventoryKey(branch.name), branch]));
  const worktreesByKey = new Map<string, WorktreeInfo[]>();
  for (const worktree of worktrees) {
    const key = branchInventoryKey(worktree.branch ?? "");
    if (!key) continue;
    const existing = worktreesByKey.get(key) ?? [];
    worktreesByKey.set(key, [...existing, worktree]);
  }

  if (filter === "Local") {
    return local.map((branch) =>
      buildInventoryEntry(
        branchInventoryKey(branch.name),
        branch,
        remoteByKey.get(branchInventoryKey(branch.name)) ?? null,
        worktreesByKey.get(branchInventoryKey(branch.name)) ?? [],
      ),
    );
  }

  if (filter === "Remote") {
    return remote.map((branch) =>
      buildInventoryEntry(
        branchInventoryKey(branch.name),
        localByKey.get(branchInventoryKey(branch.name)) ?? null,
        branch,
        worktreesByKey.get(branchInventoryKey(branch.name)) ?? [],
      ),
    );
  }

  const keys = new Set([
    ...local.map((branch) => branchInventoryKey(branch.name)),
    ...remote.map((branch) => branchInventoryKey(branch.name)),
  ]);

  return Array.from(keys).map((key) =>
    buildInventoryEntry(
      key,
      localByKey.get(key) ?? null,
      remoteByKey.get(key) ?? null,
      worktreesByKey.get(key) ?? [],
    ),
  );
}

export function resolveBranchInventoryAction(entry: BranchInventoryEntry): BranchInventoryResolutionAction {
  return entry.resolution_action;
}
