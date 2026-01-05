import { createEffect, createMemo, createSignal } from "solid-js";
import { useKeyboard, useTerminalDimensions } from "@opentui/solid";
import type { BranchItem, BranchViewMode, Statistics } from "../../types.js";
import type { ToolStatus } from "../../hooks/useToolStatus.js";

interface CleanupUIState {
  indicators: Record<string, unknown>;
  footerMessage: unknown | null;
  inputLocked: boolean;
}

export interface BranchListScreenProps {
  branches: BranchItem[];
  stats: Statistics;
  onSelect: (branch: BranchItem) => void;
  onQuit?: () => void;
  onCleanupCommand?: () => void;
  onRefresh?: () => void;
  onOpenProfiles?: () => void;
  onOpenLogs?: () => void;
  loading?: boolean;
  error?: Error | null;
  lastUpdated?: Date | null;
  loadingIndicatorDelay?: number;
  cleanupUI?: CleanupUIState;
  version?: string | null;
  workingDirectory?: string;
  activeProfile?: string | null;
  selectedBranches?: string[];
  onToggleSelect?: (branchName: string) => void;
  toolStatuses?: ToolStatus[] | undefined;
}

const VIEW_MODES: BranchViewMode[] = ["all", "local", "remote"];

export function BranchListScreen(props: BranchListScreenProps) {
  const terminal = useTerminalDimensions();

  const [filterQuery, setFilterQuery] = createSignal("");
  const [filterMode, setFilterMode] = createSignal(false);
  const [viewMode, setViewMode] = createSignal<BranchViewMode>("all");
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [scrollOffset, setScrollOffset] = createSignal(0);

  const contentHeight = createMemo(() => {
    const height = terminal().height || 24;
    const fixedLines = 2; // header + filter line
    return Math.max(1, height - fixedLines);
  });

  const filteredBranches = createMemo(() => {
    let result = props.branches;
    const mode = viewMode();

    if (mode === "local") {
      result = result.filter((branch) => branch.type === "local");
    } else if (mode === "remote") {
      result = result.filter(
        (branch) =>
          branch.type === "remote" || branch.hasRemoteCounterpart === true,
      );
    }

    const query = filterQuery().trim().toLowerCase();
    if (query) {
      result = result.filter((branch) => {
        if (branch.name.toLowerCase().includes(query)) {
          return true;
        }
        if (branch.openPR?.title?.toLowerCase().includes(query)) {
          return true;
        }
        return false;
      });
    }

    return result;
  });

  createEffect(() => {
    filterQuery();
    viewMode();
    setSelectedIndex(0);
    setScrollOffset(0);
  });

  createEffect(() => {
    const total = filteredBranches().length;
    if (total === 0) {
      setSelectedIndex(0);
      setScrollOffset(0);
      return;
    }
    setSelectedIndex((prev) => Math.min(prev, total - 1));
  });

  createEffect(() => {
    const total = filteredBranches().length;
    const limit = contentHeight();
    const index = selectedIndex();

    setScrollOffset((prev) => {
      if (total === 0) {
        return 0;
      }

      let next = prev;
      if (index < prev) {
        next = index;
      } else if (index >= prev + limit) {
        next = index - limit + 1;
      }

      const maxOffset = Math.max(0, total - limit);
      return Math.min(Math.max(0, next), maxOffset);
    });
  });

  useKeyboard((key) => {
    if (props.cleanupUI?.inputLocked) {
      return;
    }

    if (filterMode()) {
      if (key.name === "escape") {
        if (filterQuery()) {
          setFilterQuery("");
        } else {
          setFilterMode(false);
        }
        return;
      }

      if (key.name === "backspace") {
        setFilterQuery((prev) => prev.slice(0, -1));
        return;
      }

      if (key.name === "return" || key.name === "linefeed") {
        setFilterMode(false);
        return;
      }

      if (
        key.sequence &&
        key.sequence.length === 1 &&
        !key.ctrl &&
        !key.meta &&
        !key.super &&
        !key.hyper
      ) {
        setFilterQuery((prev) => prev + key.sequence);
      }
      return;
    }

    if (key.name === "escape" && filterQuery()) {
      setFilterQuery("");
      return;
    }

    if (key.name === "f" || key.sequence === "f") {
      setFilterMode(true);
      return;
    }

    if (key.name === "tab") {
      const currentIndex = VIEW_MODES.indexOf(viewMode());
      const nextIndex = (currentIndex + 1) % VIEW_MODES.length;
      setViewMode(VIEW_MODES[nextIndex] ?? "all");
      return;
    }

    if (key.name === "down") {
      const total = filteredBranches().length;
      if (total > 0) {
        setSelectedIndex((prev) => Math.min(prev + 1, total - 1));
      }
      return;
    }

    if (key.name === "up") {
      const total = filteredBranches().length;
      if (total > 0) {
        setSelectedIndex((prev) => Math.max(prev - 1, 0));
      }
      return;
    }

    if (key.name === "return" || key.name === "linefeed") {
      const selected = filteredBranches()[selectedIndex()];
      if (selected) {
        props.onSelect(selected);
      }
    }
  });

  const visibleBranches = createMemo(() => {
    const start = scrollOffset();
    const limit = contentHeight();
    return filteredBranches().slice(start, start + limit);
  });

  return (
    <box flexDirection="column">
      <text>gwt - Branch Selection</text>
      <text>Filter: {filterQuery() || "(press f to filter)"}</text>
      <box flexDirection="column">
        {filteredBranches().length === 0 ? (
          <text>No branches found</text>
        ) : (
          visibleBranches().map((branch, index) => {
            const absoluteIndex = scrollOffset() + index;
            const isSelected = absoluteIndex === selectedIndex();
            const label = branch.label ?? branch.name;
            return (
              <text>
                {isSelected ? ">" : " "} {label}
              </text>
            );
          })
        )}
      </box>
    </box>
  );
}
