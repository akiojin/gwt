import { branchInventoryKey } from "./branchInventory";
import type {
  BranchBrowserPanelConfig,
  BranchBrowserPanelState,
  BranchInfo,
  BranchInventoryEntry,
} from "./types";
import {
  sortBranches,
  type SidebarFilterType,
} from "./components/sidebarHelpers";

export function matchesBranchFilterRuntime(
  entry: BranchInventoryEntry,
  filter: SidebarFilterType,
): boolean {
  if (filter === "Local") return entry.has_local;
  if (filter === "Remote") return entry.has_remote;
  return true;
}

export function buildFilteredBranchEntriesRuntime(args: {
  branchEntries: BranchInventoryEntry[];
  activeFilter: SidebarFilterType;
  searchQuery: string;
  remotePrimaryNames: Set<string>;
}) {
  const q = args.searchQuery.trim().toLowerCase();
  const matchingEntries = args.branchEntries.filter((entry) => {
    if (!matchesBranchFilterRuntime(entry, args.activeFilter)) return false;
    if (!q) return true;
    const branch = entry.primary_branch;
    const haystack =
      `${branch.display_name ?? ""} ${branch.name} ${entry.canonical_name}`.toLowerCase();
    return haystack.includes(q);
  });
  const sortedBranches = sortBranches(
    matchingEntries.map((entry) => entry.primary_branch),
    args.activeFilter,
    args.remotePrimaryNames,
    "name",
  );
  const orderedNames = new Map(
    sortedBranches.map((branch, index) => [branch.name, index]),
  );
  return [...matchingEntries].sort(
    (a, b) =>
      (orderedNames.get(a.primary_branch.name) ?? Number.MAX_SAFE_INTEGER) -
      (orderedNames.get(b.primary_branch.name) ?? Number.MAX_SAFE_INTEGER),
  );
}

export function resolveSelectedEntryRuntime(args: {
  config: BranchBrowserPanelConfig;
  branchEntries: BranchInventoryEntry[];
}) {
  const selectedBranchName =
    args.config.selectedBranch?.name?.trim() ??
    args.config.selectedBranchName?.trim() ??
    "";
  const key = branchInventoryKey(selectedBranchName);
  return args.branchEntries.find((entry) => entry.canonical_name === key) ?? null;
}

export function actionLabelRuntime(entry: BranchInventoryEntry): string {
  switch (entry.resolution_action) {
    case "focusExisting":
      return "Focus Worktree";
    case "resolveAmbiguity":
      return "Resolve Ambiguity";
    default:
      return "Create Worktree";
  }
}

export function buildBranchBrowserStateRuntime(args: {
  activeFilter: SidebarFilterType;
  searchQuery: string;
  config: BranchBrowserPanelConfig;
}): BranchBrowserPanelState {
  return {
    filter: args.activeFilter,
    query: args.searchQuery,
    selectedBranchName:
      args.config.selectedBranch?.name?.trim() ??
      args.config.selectedBranchName?.trim() ??
      null,
  };
}

export function buildFetchRequestKeyRuntime(
  projectPath: string,
  refreshKey: number,
): string {
  return `${projectPath}::${refreshKey}`;
}

export function buildHydrationKeyRuntime(config: BranchBrowserPanelConfig): string {
  return JSON.stringify([
    config.initialFilter ?? "Local",
    config.initialQuery ?? "",
  ]);
}

export function buildRemotePrimaryNamesRuntime(entries: BranchInventoryEntry[]) {
  return new Set(
    entries
      .filter((entry) => !entry.has_local && entry.has_remote)
      .map((entry) => entry.primary_branch.name),
  );
}

export function resolveSelectedBranchRuntime(args: {
  config: BranchBrowserPanelConfig;
  selectedEntry: BranchInventoryEntry | null;
}): BranchInfo | null {
  return args.config.selectedBranch ?? args.selectedEntry?.primary_branch ?? null;
}
