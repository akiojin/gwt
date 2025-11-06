import type { SelectedBranchState } from "../types.js";

function getBranchRef(branch: SelectedBranchState | null | undefined): string | null {
  if (!branch) {
    return null;
  }
  return branch.remoteBranch ?? branch.name;
}

export function resolveBaseBranchRef(
  creationSource: SelectedBranchState | null,
  selectedBranch: SelectedBranchState | null,
  resolveDefault: () => string,
): string {
  return (
    getBranchRef(creationSource) ??
    getBranchRef(selectedBranch) ??
    resolveDefault()
  );
}

export function resolveBaseBranchLabel(
  creationSource: SelectedBranchState | null,
  selectedBranch: SelectedBranchState | null,
  resolveDefault: () => string,
): string {
  return (
    creationSource?.displayName ??
    selectedBranch?.displayName ??
    resolveDefault()
  );
}
