/**
 * Branch Store - Branch list state management for SolidJS
 *
 * This store manages branch data, selection, filtering, and statistics.
 * Uses SolidJS's fine-grained reactivity with signals and stores.
 *
 * @see specs/SPEC-d27be71b/spec.md - OpenTUI migration spec
 */

import { createMemo, createRoot } from "solid-js";
import { createStore, produce } from "solid-js/store";
import type {
  BranchCategory,
  BranchViewMode,
  FilterState,
  SelectionState,
  BranchStatistics,
} from "../core/types.js";
import type { BranchItem } from "../types.js";

// ========================================
// Types
// ========================================

export interface BranchStoreState {
  /** All branches (unfiltered) */
  branches: BranchItem[];
  /** Current filter state */
  filter: FilterState;
  /** Current selection state */
  selection: SelectionState;
  /** Current view mode */
  viewMode: BranchViewMode;
  /** Statistics */
  statistics: BranchStatistics;
  /** Loading state */
  isLoading: boolean;
  /** Error message */
  error: string | null;
}

// ========================================
// Initial State
// ========================================

const initialFilter: FilterState = {
  searchQuery: "",
  showWithWorktree: true,
  showWithoutWorktree: true,
  showLocal: true,
  showRemote: true,
  branchTypes: [
    "feature",
    "bugfix",
    "hotfix",
    "release",
    "main",
    "develop",
    "other",
  ],
};

const initialSelection: SelectionState = {
  selectedIndex: 0,
  highlightedIndex: 0,
  scrollOffset: 0,
};

const initialStatistics: BranchStatistics = {
  localCount: 0,
  remoteCount: 0,
  worktreeCount: 0,
  changesCount: 0,
  lastUpdated: new Date(),
};

const initialState: BranchStoreState = {
  branches: [],
  filter: initialFilter,
  selection: initialSelection,
  viewMode: "all",
  statistics: initialStatistics,
  isLoading: false,
  error: null,
};

// ========================================
// Store Creation
// ========================================

function createBranchStore() {
  const [state, setState] = createStore<BranchStoreState>(initialState);

  // Derived: filtered branches
  const filteredBranches = createMemo(() => {
    let result = state.branches;

    // Filter by view mode
    if (state.viewMode === "local") {
      result = result.filter((b) => b.type === "local");
    } else if (state.viewMode === "remote") {
      result = result.filter((b) => b.type === "remote");
    }

    // Filter by search query
    const query = state.filter.searchQuery.toLowerCase();
    if (query) {
      result = result.filter(
        (b) =>
          b.name.toLowerCase().includes(query) ||
          b.description?.toLowerCase().includes(query),
      );
    }

    // Filter by worktree status
    if (!state.filter.showWithWorktree) {
      result = result.filter((b) => !b.worktree);
    }
    if (!state.filter.showWithoutWorktree) {
      result = result.filter((b) => b.worktree);
    }

    // Filter by local/remote
    if (!state.filter.showLocal) {
      result = result.filter((b) => b.type !== "local");
    }
    if (!state.filter.showRemote) {
      result = result.filter((b) => b.type !== "remote");
    }

    // Filter by branch types
    if (state.filter.branchTypes.length < 7) {
      result = result.filter((b) =>
        state.filter.branchTypes.includes(b.branchType),
      );
    }

    return result;
  });

  // Derived: selected branch
  const selectedBranch = createMemo(() => {
    const branches = filteredBranches();
    const index = state.selection.selectedIndex;
    return branches[index] ?? null;
  });

  const actions = {
    /**
     * Set branches data
     */
    setBranches(branches: BranchItem[]): void {
      setState("branches", branches);
      // Update statistics
      setState("statistics", {
        localCount: branches.filter((b) => b.type === "local").length,
        remoteCount: branches.filter((b) => b.type === "remote").length,
        worktreeCount: branches.filter((b) => b.worktree).length,
        changesCount: branches.filter((b) => b.hasChanges).length,
        lastUpdated: new Date(),
      });
      // Reset selection if out of bounds
      if (state.selection.selectedIndex >= branches.length) {
        setState("selection", "selectedIndex", 0);
      }
    },

    /**
     * Set loading state
     */
    setLoading(isLoading: boolean): void {
      setState("isLoading", isLoading);
    },

    /**
     * Set error
     */
    setError(error: string | null): void {
      setState("error", error);
    },

    /**
     * Update search query
     */
    setSearchQuery(query: string): void {
      setState("filter", "searchQuery", query);
      // Reset selection when filter changes
      setState("selection", "selectedIndex", 0);
      setState("selection", "scrollOffset", 0);
    },

    /**
     * Clear search query
     */
    clearSearch(): void {
      setState("filter", "searchQuery", "");
    },

    /**
     * Toggle view mode
     */
    cycleViewMode(): void {
      const modes: BranchViewMode[] = ["all", "local", "remote"];
      const currentIndex = modes.indexOf(state.viewMode);
      const nextIndex = (currentIndex + 1) % modes.length;
      const nextMode = modes[nextIndex];
      if (nextMode) setState("viewMode", nextMode);
      // Reset selection
      setState("selection", "selectedIndex", 0);
      setState("selection", "scrollOffset", 0);
    },

    /**
     * Set view mode
     */
    setViewMode(mode: BranchViewMode): void {
      setState("viewMode", mode);
    },

    /**
     * Toggle branch type filter
     */
    toggleBranchType(type: BranchCategory): void {
      setState(
        produce((s) => {
          const types = s.filter.branchTypes;
          const index = types.indexOf(type);
          if (index >= 0) {
            types.splice(index, 1);
          } else {
            types.push(type);
          }
        }),
      );
    },

    /**
     * Toggle worktree filter
     */
    toggleWorktreeFilter(): void {
      setState(
        produce((s) => {
          // Cycle through: both -> with only -> without only -> both
          if (s.filter.showWithWorktree && s.filter.showWithoutWorktree) {
            s.filter.showWithoutWorktree = false;
          } else if (s.filter.showWithWorktree) {
            s.filter.showWithWorktree = false;
            s.filter.showWithoutWorktree = true;
          } else {
            s.filter.showWithWorktree = true;
            s.filter.showWithoutWorktree = true;
          }
        }),
      );
    },

    /**
     * Move selection up
     */
    moveUp(amount = 1): void {
      setState(
        produce((s) => {
          const newIndex = Math.max(0, s.selection.selectedIndex - amount);
          s.selection.selectedIndex = newIndex;
          s.selection.highlightedIndex = newIndex;
        }),
      );
    },

    /**
     * Move selection down
     */
    moveDown(amount = 1): void {
      const maxIndex = filteredBranches().length - 1;
      setState(
        produce((s) => {
          const newIndex = Math.min(
            maxIndex,
            s.selection.selectedIndex + amount,
          );
          s.selection.selectedIndex = newIndex;
          s.selection.highlightedIndex = newIndex;
        }),
      );
    },

    /**
     * Go to top of list
     */
    goToTop(): void {
      setState("selection", "selectedIndex", 0);
      setState("selection", "scrollOffset", 0);
    },

    /**
     * Go to bottom of list
     */
    goToBottom(): void {
      const maxIndex = Math.max(0, filteredBranches().length - 1);
      setState("selection", "selectedIndex", maxIndex);
    },

    /**
     * Update scroll offset for virtualization
     */
    setScrollOffset(offset: number): void {
      setState("selection", "scrollOffset", offset);
    },

    /**
     * Reset all filters
     */
    resetFilters(): void {
      setState("filter", initialFilter);
      setState("selection", initialSelection);
    },
  };

  return {
    state,
    actions,
    // Derived state (memos)
    filteredBranches,
    selectedBranch,
  };
}

// ========================================
// Singleton Export
// ========================================

let _store: ReturnType<typeof createBranchStore> | null = null;

export function getBranchStore() {
  if (!_store) {
    createRoot(() => {
      _store = createBranchStore();
    });
  }
  // Store is guaranteed to be initialized by createRoot
  return _store as ReturnType<typeof createBranchStore>;
}

// Convenience exports
export const branchStore = new Proxy({} as BranchStoreState, {
  get: (_, prop) => getBranchStore().state[prop as keyof BranchStoreState],
});

export const branchActions = new Proxy(
  {} as ReturnType<typeof createBranchStore>["actions"],
  {
    get: (_, prop) =>
      getBranchStore().actions[
        prop as keyof ReturnType<typeof createBranchStore>["actions"]
      ],
  },
);
