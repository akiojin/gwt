import type { BranchInfo, PrStatusLite, WorktreeInfo } from "../types";

export type SidebarFilterType = "Local" | "Remote" | "All";
export type SidebarSortMode = "name" | "updated";
export type RefreshFailureAction = "ignore" | "clear-loading" | "show-error";

export type SidebarEventListen = <T = unknown>(
  event: string,
  handler: (event: { payload: T }) => void,
) => Promise<() => void>;

export function normalizeTabBranch(name: string): string {
  const trimmed = name.trim();
  return trimmed.startsWith("origin/") ? trimmed.slice("origin/".length) : trimmed;
}

export function stripRemotePrefix(name: string): string {
  const trimmed = name.trim();
  const slash = trimmed.indexOf("/");
  if (slash <= 0) return trimmed;
  return trimmed.slice(slash + 1);
}

export function branchPriority(name: string): number {
  const normalized = name.trim().toLowerCase();
  const baseName = normalized.startsWith("origin/")
    ? normalized.slice("origin/".length)
    : normalized;
  if (baseName === "main") return 2;
  if (baseName === "develop") return 1;
  return 0;
}

export function branchSortTimestamp(branch: BranchInfo): number | null {
  const value = (branch as { commit_timestamp?: unknown }).commit_timestamp;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

export function compareBranches(
  a: BranchInfo,
  b: BranchInfo,
  filter: SidebarFilterType,
  remoteBranchNames: ReadonlySet<string>,
  sortMode: SidebarSortMode,
): number {
  if (filter === "All") {
    const aRemote = remoteBranchNames.has(a.name) ? 1 : 0;
    const bRemote = remoteBranchNames.has(b.name) ? 1 : 0;
    if (aRemote !== bRemote) {
      return aRemote - bRemote;
    }
  }

  const aPriority = branchPriority(a.name);
  const bPriority = branchPriority(b.name);
  if (aPriority !== bPriority) {
    return bPriority - aPriority;
  }

  const byName = a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  if (sortMode === "name") {
    return byName;
  }

  const aTs = branchSortTimestamp(a);
  const bTs = branchSortTimestamp(b);
  if (aTs === null && bTs === null) {
    return byName;
  }
  if (aTs === null) return 1;
  if (bTs === null) return -1;
  if (aTs !== bTs) {
    return bTs - aTs;
  }
  return byName;
}

export function sortBranches(
  list: BranchInfo[],
  filter: SidebarFilterType,
  remoteBranchNames: ReadonlySet<string>,
  sortMode: SidebarSortMode,
): BranchInfo[] {
  return [...list].sort((a, b) => compareBranches(a, b, filter, remoteBranchNames, sortMode));
}

export function normalizeBranchForPrLookup(
  branchName: string,
  remoteBranchNames: ReadonlySet<string>,
): string {
  const trimmed = branchName.trim();
  return remoteBranchNames.has(trimmed) ? stripRemotePrefix(trimmed) : trimmed;
}

export function buildFilterCacheKey(
  filter: SidebarFilterType,
  path: string,
  refreshKey: number,
  localRefreshKey: number,
): string {
  if (filter === "Remote") {
    return `${path}::${refreshKey}`;
  }
  return `${path}::${refreshKey}::${localRefreshKey}`;
}

export function buildWorktreeMap(worktrees: WorktreeInfo[]): Map<string, WorktreeInfo> {
  const map = new Map<string, WorktreeInfo>();
  for (const wt of worktrees) {
    if (wt.branch) map.set(wt.branch, wt);
  }
  return map;
}

export function toErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    return String((err as { message?: unknown }).message);
  }
  return String(err);
}

export function getSafetyLevel(
  branch: BranchInfo,
  worktreeMap: ReadonlyMap<string, WorktreeInfo>,
): string {
  const wt = worktreeMap.get(branch.name);
  if (!wt) return "";
  return wt.safety_level || "";
}

export function safetyTitleForLevel(level: string): string {
  switch (level) {
    case "safe":
      return "Safe to delete";
    case "warning":
      return "Has uncommitted changes or unpushed commits";
    case "danger":
      return "Has uncommitted changes and unpushed commits";
    case "disabled":
      return "Protected or current branch";
    default:
      return "";
  }
}

export function getSafetyTitle(
  branch: BranchInfo,
  worktreeMap: ReadonlyMap<string, WorktreeInfo>,
): string {
  return safetyTitleForLevel(getSafetyLevel(branch, worktreeMap));
}

export function isBranchProtected(
  branch: BranchInfo,
  worktreeMap: ReadonlyMap<string, WorktreeInfo>,
): boolean {
  const wt = worktreeMap.get(branch.name);
  return wt ? wt.is_protected || wt.is_current : false;
}

export function divergenceIndicator(branch: BranchInfo): string {
  switch (branch.divergence_status) {
    case "Ahead":
      return `+${branch.ahead}`;
    case "Behind":
      return `-${branch.behind}`;
    case "Diverged":
      return `+${branch.ahead} -${branch.behind}`;
    default:
      return "";
  }
}

export function divergenceClass(status: string): string {
  switch (status) {
    case "Ahead":
      return "ahead";
    case "Behind":
      return "behind";
    case "Diverged":
      return "diverged";
    default:
      return "";
  }
}

export function toolUsageClass(usage: string | null | undefined): string {
  const key = (usage ?? "").toLowerCase();
  if (key.startsWith("claude@")) return "claude";
  if (key.startsWith("codex@")) return "codex";
  if (key.startsWith("gemini@")) return "gemini";
  if (key.startsWith("opencode@") || key.startsWith("open-code@"))
    return "opencode";
  return "";
}

export function clampSidebarWidth(
  width: number,
  minWidthPx: number,
  maxWidthPx: number,
): number {
  const min = Number.isFinite(minWidthPx) ? minWidthPx : 220;
  const maxCandidate = Number.isFinite(maxWidthPx) ? maxWidthPx : 520;
  const max = Math.max(min, maxCandidate);
  if (!Number.isFinite(width)) return min;
  return Math.max(min, Math.min(max, Math.round(width)));
}

export function decideRefreshFailureAction(
  background: boolean,
  hadFallbackCache: boolean,
  applyToActiveView: boolean,
  isActiveFilter: boolean,
): RefreshFailureAction {
  if (background && hadFallbackCache) {
    return applyToActiveView && isActiveFilter ? "clear-loading" : "ignore";
  }
  return applyToActiveView && isActiveFilter ? "show-error" : "ignore";
}

export function applyPrStatusUpdate(
  pollingStatuses: Record<string, PrStatusLite | null>,
  eventBranch: string,
  status: PrStatusLite,
): Record<string, PrStatusLite | null> | null {
  const next = { ...pollingStatuses };
  let updated = false;
  for (const key of Object.keys(next)) {
    const existing = next[key];
    if (existing && existing.headBranch === eventBranch) {
      next[key] = status;
      updated = true;
    }
  }
  return updated ? next : null;
}

export function resolveEventListen(
  mod: { listen?: SidebarEventListen; default?: { listen?: SidebarEventListen } },
): SidebarEventListen {
  const listenFn = mod.listen ?? mod.default?.listen;
  if (!listenFn) {
    throw new Error("Tauri event listen API is unavailable");
  }
  return listenFn;
}
