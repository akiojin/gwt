import React, { useCallback, useState, useMemo } from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Stats } from "../parts/Stats.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { Input } from "../common/Input.js";
import { LoadingIndicator } from "../common/LoadingIndicator.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { BranchItem, Statistics } from "../../types.js";
import stringWidth from "string-width";
import chalk from "chalk";

const WIDTH_OVERRIDES: Record<string, number> = {
  // Remote icon
  "☁": 1,
  "☁️": 1,
  "☁︎": 1,
  // Unpushed icon
  "⬆": 1,
  "⬆️": 1,
  "⬆︎": 1,
  // Branch type icons
  "⚡": 1,
  "✨": 1,
  "🐛": 1,
  "🔥": 1,
  "🚀": 1,
  "📌": 1,
  // Worktree status icons
  "🟢": 1,
  "🟠": 1,
  // Change status icons
  "⭐": 1,
  "✏️": 1,
  "🔀": 1,
  "✅": 1,
  "⚠️": 1,
};

const getCharWidth = (char: string): number => {
  const baseWidth = stringWidth(char);
  const override = WIDTH_OVERRIDES[char];
  return override !== undefined ? Math.max(baseWidth, override) : baseWidth;
};

const measureDisplayWidth = (value: string): number => {
  let width = 0;
  for (const char of Array.from(value)) {
    width += getCharWidth(char);
  }
  return width;
};

type IndicatorColor = "cyan" | "green" | "yellow" | "red";

interface CleanupIndicator {
  icon: string;
  color?: IndicatorColor;
}

interface CleanupFooterMessage {
  text: string;
  color?: IndicatorColor;
}

interface CleanupUIState {
  indicators: Record<string, CleanupIndicator>;
  footerMessage: CleanupFooterMessage | null;
  inputLocked: boolean;
}

export interface BranchListScreenProps {
  branches: BranchItem[];
  stats: Statistics;
  onSelect: (branch: BranchItem) => void;
  onNavigate?: (screen: string) => void;
  onQuit?: () => void;
  onCleanupCommand?: () => void;
  onRefresh?: () => void;
  loading?: boolean;
  error?: Error | null;
  lastUpdated?: Date | null;
  loadingIndicatorDelay?: number;
  cleanupUI?: CleanupUIState;
  version?: string | null;
  workingDirectory?: string;
  // Test support: allow external control of filter mode and query
  testFilterMode?: boolean;
  testOnFilterModeChange?: (mode: boolean) => void;
  testFilterQuery?: string;
  testOnFilterQueryChange?: (query: string) => void;
}

/**
 * BranchListScreen - Main screen for branch selection
 * Layout: Header + Stats + Branch List + Footer
 */
export function BranchListScreen({
  branches,
  stats,
  onSelect,
  onNavigate,
  onCleanupCommand,
  onRefresh,
  loading = false,
  error = null,
  lastUpdated = null,
  loadingIndicatorDelay = 300,
  cleanupUI,
  version,
  workingDirectory,
  testFilterMode,
  testOnFilterModeChange,
  testFilterQuery,
  testOnFilterQueryChange,
}: BranchListScreenProps) {
  const { rows } = useTerminalSize();

  // Filter state - allow test control via props
  const [internalFilterQuery, setInternalFilterQuery] = useState("");
  const filterQuery =
    testFilterQuery !== undefined ? testFilterQuery : internalFilterQuery;
  const setFilterQuery = useCallback(
    (query: string) => {
      setInternalFilterQuery(query);
      testOnFilterQueryChange?.(query);
    },
    [testOnFilterQueryChange],
  );

  // Focus management: true = filter mode, false = branch selection mode
  // Allow test control via props
  const [internalFilterMode, setInternalFilterMode] = useState(false);
  const filterMode =
    testFilterMode !== undefined ? testFilterMode : internalFilterMode;
  const setFilterMode = useCallback(
    (mode: boolean) => {
      setInternalFilterMode(mode);
      testOnFilterModeChange?.(mode);
    },
    [testOnFilterModeChange],
  );

  // Handle keyboard input
  // Note: Input component blocks specific keys (c/r/m/f) using blockKeys prop
  // This prevents shortcuts from triggering while typing in the filter
  useInput((input, key) => {
    if (cleanupUI?.inputLocked) {
      return;
    }

    // Escape key handling
    if (key.escape) {
      if (filterQuery) {
        // Clear filter query first
        setFilterQuery("");
        return;
      }
      if (filterMode) {
        // Exit filter mode if query is empty
        setFilterMode(false);
        return;
      }
    }

    // Enter filter mode with 'f' key (only in branch selection mode)
    if (input === "f" && !filterMode) {
      setFilterMode(true);
      return;
    }

    // Disable global shortcuts while in filter mode
    if (filterMode) {
      return;
    }

    // Global shortcuts (blocked by Input component when typing in filter mode)
    if (input === "m" && onNavigate) {
      onNavigate("worktree-manager");
    } else if (input === "c") {
      onCleanupCommand?.();
    } else if (input === "r" && onRefresh) {
      onRefresh();
    }
  });

  // Filter branches based on query
  const filteredBranches = useMemo(() => {
    if (!filterQuery.trim()) {
      return branches;
    }

    const query = filterQuery.toLowerCase();
    return branches.filter((branch) => {
      // Search in branch name
      if (branch.name.toLowerCase().includes(query)) {
        return true;
      }

      // Search in PR title if available (only openPR has title)
      if (branch.openPR?.title?.toLowerCase().includes(query)) {
        return true;
      }

      return false;
    });
  }, [branches, filterQuery]);

  // Calculate available space for branch list
  // Header: 2 lines (title + divider)
  // Filter input: 1 line
  // Stats: 1 line
  // Empty line: 1 line
  // Footer: 1 line
  // Total fixed: 6 lines
  const headerLines = 2;
  const filterLines = 1;
  const statsLines = 1;
  const emptyLine = 1;
  const footerLines = 1;
  const fixedLines =
    headerLines + filterLines + statsLines + emptyLine + footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight); // Minimum 5 items visible

  // Footer actions
  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "f", description: "Filter" },
    { key: "r", description: "Refresh" },
    { key: "m", description: "Manage worktrees" },
    { key: "c", description: "Cleanup branches" },
  ];

  const formatLatestCommit = useCallback((timestamp?: number) => {
    if (!timestamp || Number.isNaN(timestamp)) {
      return "---";
    }

    const date = new Date(timestamp * 1000);
    const year = date.getFullYear();
    const month = String(date.getMonth() + 1).padStart(2, "0");
    const day = String(date.getDate()).padStart(2, "0");
    const hours = String(date.getHours()).padStart(2, "0");
    const minutes = String(date.getMinutes()).padStart(2, "0");

    return `${year}-${month}-${day} ${hours}:${minutes}`;
  }, []);

  const truncateToWidth = useCallback((value: string, maxWidth: number) => {
    if (maxWidth <= 0) {
      return "";
    }

    if (measureDisplayWidth(value) <= maxWidth) {
      return value;
    }

    const ellipsis = "…";
    const ellipsisWidth = measureDisplayWidth(ellipsis);
    if (ellipsisWidth >= maxWidth) {
      return ellipsis;
    }

    let currentWidth = 0;
    let result = "";

    for (const char of Array.from(value)) {
      const charWidth = getCharWidth(char);
      if (currentWidth + charWidth + ellipsisWidth > maxWidth) {
        break;
      }
      result += char;
      currentWidth += charWidth;
    }

    return result + ellipsis;
  }, []);

  const renderBranchRow = useCallback(
    (item: BranchItem, isSelected: boolean, context: { columns: number }) => {
      const columns = Math.max(20, context.columns);
      const arrow = isSelected ? ">" : " ";
      const timestampText = formatLatestCommit(item.latestCommitTimestamp);
      const timestampWidth = stringWidth(timestampText);

      const indicatorInfo = cleanupUI?.indicators?.[item.name];
      let indicatorIcon = indicatorInfo?.icon ?? "";
      if (indicatorIcon && indicatorInfo?.color && !isSelected) {
        switch (indicatorInfo.color) {
          case "cyan":
            indicatorIcon = chalk.cyan(indicatorIcon);
            break;
          case "green":
            indicatorIcon = chalk.green(indicatorIcon);
            break;
          case "yellow":
            indicatorIcon = chalk.yellow(indicatorIcon);
            break;
          case "red":
            indicatorIcon = chalk.red(indicatorIcon);
            break;
          default:
            break;
        }
      }
      const indicatorPrefix = indicatorIcon ? `${indicatorIcon} ` : "";
      const staticPrefix = `${arrow} ${indicatorPrefix}`;
      const staticPrefixWidth = measureDisplayWidth(staticPrefix);

      const availableLeftWidth = Math.max(
        staticPrefixWidth,
        columns - timestampWidth - 1,
      );
      const maxLabelWidth = Math.max(0, availableLeftWidth - staticPrefixWidth);
      const truncatedLabel = truncateToWidth(item.label, maxLabelWidth);
      const leftText = `${staticPrefix}${truncatedLabel}`;

      const leftMeasuredWidth = stringWidth(leftText);
      const leftDisplayWidth = measureDisplayWidth(leftText);
      const baseGapWidth = Math.max(
        1,
        columns - leftMeasuredWidth - timestampWidth,
      );
      const displayGapWidth = Math.max(
        1,
        columns - leftDisplayWidth - timestampWidth,
      );
      const cursorShift = Math.max(0, displayGapWidth - baseGapWidth);

      const gap = " ".repeat(baseGapWidth);
      const cursorAdjust = cursorShift > 0 ? `\u001b[${cursorShift}C` : "";

      let line = `${leftText}${gap}${cursorAdjust}${timestampText}`;
      const paddingWidth = Math.max(0, columns - stringWidth(line));
      if (paddingWidth > 0) {
        line += " ".repeat(paddingWidth);
      }

      const output = isSelected ? `\u001b[46m\u001b[30m${line}\u001b[0m` : line;
      return <Text>{output}</Text>;
    },
    [cleanupUI, formatLatestCommit, truncateToWidth],
  );

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header
        title="gwt - Branch Selection"
        titleColor="cyan"
        version={version}
        {...(workingDirectory !== undefined && { workingDirectory })}
      />

      {/* Filter Input - Always visible */}
      <Box>
        <Text dimColor>Filter: </Text>
        {filterMode ? (
          <Input
            value={filterQuery}
            onChange={setFilterQuery}
            onSubmit={() => {}} // No-op: filter is applied in real-time
            placeholder="Type to search..."
            blockKeys={["c", "r", "m", "f"]} // Block shortcuts while typing
          />
        ) : (
          <Text dimColor>{filterQuery || "(press f to filter)"}</Text>
        )}
        {filterQuery && (
          <Text dimColor>
            {" "}
            (Showing {filteredBranches.length} of {branches.length})
          </Text>
        )}
      </Box>

      {/* Stats */}
      <Box>
        <Stats stats={stats} lastUpdated={lastUpdated} />
      </Box>

      {/* Content */}
      <Box flexDirection="column" flexGrow={1}>
        <LoadingIndicator
          isLoading={Boolean(loading)}
          delay={loadingIndicatorDelay}
          message="Loading Git information..."
        />

        {error && (
          <Box flexDirection="column">
            <Text color="red" bold>
              Error: {error.message}
            </Text>
            {process.env.DEBUG && error.stack && (
              <Box marginTop={1}>
                <Text color="gray">{error.stack}</Text>
              </Box>
            )}
          </Box>
        )}

        {!loading && !error && branches.length === 0 && (
          <Box>
            <Text dimColor>No branches found</Text>
          </Box>
        )}

        {!loading &&
          !error &&
          branches.length > 0 &&
          filteredBranches.length === 0 &&
          filterQuery && (
            <Box>
              <Text dimColor>No branches match your filter</Text>
            </Box>
          )}

        {!loading &&
          !error &&
          branches.length > 0 &&
          filteredBranches.length > 0 && (
            <Select
              items={filteredBranches}
              onSelect={onSelect}
              limit={limit}
              disabled={Boolean(cleanupUI?.inputLocked)}
              renderIndicator={() => null}
              renderItem={renderBranchRow}
            />
          )}
      </Box>

      {cleanupUI?.footerMessage && (
        <Box marginBottom={1}>
          {cleanupUI.footerMessage.color ? (
            <Text color={cleanupUI.footerMessage.color}>
              {cleanupUI.footerMessage.text}
            </Text>
          ) : (
            <Text>{cleanupUI.footerMessage.text}</Text>
          )}
        </Box>
      )}

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
