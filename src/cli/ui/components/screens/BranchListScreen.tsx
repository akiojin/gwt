import React, { useCallback, useState, useMemo, useEffect } from "react";
import { Box, Text } from "ink";
import { Header } from "../parts/Header.js";
import { Stats } from "../parts/Stats.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { Input } from "../common/Input.js";
import { LoadingIndicator } from "../common/LoadingIndicator.js";
import { useSpinnerFrame } from "../common/SpinnerIcon.js";
import { useAppInput } from "../../hooks/useAppInput.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { BranchItem, Statistics, BranchViewMode } from "../../types.js";
import type { ToolStatus } from "../../hooks/useToolStatus.js";
import { getLatestActivityTimestamp } from "../../utils/branchFormatter.js";
import stringWidth from "string-width";
import stripAnsi from "strip-ansi";
import chalk from "chalk";

// Emoji 幅は端末によって 1 または 2 になることがあるため、最小幅を上書きして
// 実測より小さくならないようにする（過小評価＝折り返しの原因を防ぐ）
const WIDTH_OVERRIDES: Record<string, number> = {
  // Remote icon
  "☁": 1,
  // Branch type icons
  "⚡": 1,
  "✨": 1,
  "🐛": 1,
  "🔥": 1,
  "🚀": 1,
  "📌": 1,
  // Worktree status icons
  "🟢": 2,
  "⚪": 2,
  "🔴": 2,
  // Change status icons
  "👉": 1,
  "💾": 1,
  "📤": 1,
  "🔃": 1,
  "✅": 2,
  "⚠": 2,
  "⚠️": 2,
  "🛡": 2,
  // Remote markers
  "🔗": 2,
  "💻": 2,
  "☁️": 2,
  "☑": 2,
  "☐": 2,
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
  isSpinning?: boolean;
  color?: IndicatorColor;
}

interface CleanupFooterMessage {
  text: string;
  isSpinning?: boolean;
  color?: IndicatorColor;
}

interface CleanupUIState {
  indicators: Record<string, CleanupIndicator>;
  footerMessage: CleanupFooterMessage | null;
  inputLocked: boolean;
}

/**
 * Props for `BranchListScreen`.
 */
export interface BranchListScreenProps {
  branches: BranchItem[];
  stats: Statistics;
  onSelect: (branch: BranchItem) => void;
  onQuit?: () => void;
  onCleanupCommand?: () => void;
  onRepairCommand?: () => void;
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
  // Test support: allow external control of filter mode and query
  testFilterMode?: boolean;
  testOnFilterModeChange?: (mode: boolean) => void;
  testFilterQuery?: string;
  testOnFilterQueryChange?: (query: string) => void;
  testViewMode?: BranchViewMode;
  testOnViewModeChange?: (mode: BranchViewMode) => void;
  selectedBranches?: string[];
  onToggleSelect?: (branchName: string) => void;
  /**
   * AIツールのインストール状態配列
   * @see specs/SPEC-3b0ed29b/spec.md FR-019, FR-021
   */
  toolStatuses?: ToolStatus[] | undefined;
}

/**
 * BranchListScreen - Main screen for branch selection
 * Layout: Header + Stats + Branch List + Footer
 */
export const BranchListScreen = React.memo(function BranchListScreen({
  branches,
  stats,
  onSelect,
  onCleanupCommand,
  onRepairCommand,
  onRefresh,
  onOpenProfiles,
  onOpenLogs,
  loading = false,
  error = null,
  lastUpdated = null,
  loadingIndicatorDelay = 300,
  cleanupUI,
  version,
  workingDirectory,
  activeProfile,
  testFilterMode,
  testOnFilterModeChange,
  testFilterQuery,
  testOnFilterQueryChange,
  testViewMode,
  testOnViewModeChange,
  selectedBranches = [],
  onToggleSelect,
  toolStatuses,
}: BranchListScreenProps) {
  const { rows } = useTerminalSize();
  const selectedSet = useMemo(
    () => new Set(selectedBranches),
    [selectedBranches],
  );

  // Check if any indicator needs spinner animation
  const hasSpinningIndicator = useMemo(() => {
    if (!cleanupUI?.indicators) return false;
    return Object.values(cleanupUI.indicators).some((ind) => ind.isSpinning);
  }, [cleanupUI?.indicators]);

  // Also check footer message for spinner
  const hasSpinningFooter = cleanupUI?.footerMessage?.isSpinning ?? false;

  // Get spinner frame for all spinning elements
  const spinnerFrame = useSpinnerFrame(
    hasSpinningIndicator || hasSpinningFooter,
  );

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

  // View mode state for filtering by local/remote
  const [internalViewMode, setInternalViewMode] =
    useState<BranchViewMode>("all");
  const viewMode = testViewMode !== undefined ? testViewMode : internalViewMode;
  const setViewMode = useCallback(
    (mode: BranchViewMode) => {
      setInternalViewMode(mode);
      testOnViewModeChange?.(mode);
    },
    [testOnViewModeChange],
  );

  // Cycle view mode: all → local → remote → all
  const cycleViewMode = useCallback(() => {
    const modes: BranchViewMode[] = ["all", "local", "remote"];
    const currentIndex = modes.indexOf(viewMode);
    const nextIndex = (currentIndex + 1) % modes.length;
    const nextMode = modes[nextIndex];
    if (nextMode !== undefined) {
      setViewMode(nextMode);
    }
  }, [viewMode, setViewMode]);

  // Cursor position for Select (controlled to enable space toggle)
  const [selectedIndex, setSelectedIndex] = useState(0);

  // Handle keyboard input
  // Note: Input component blocks specific keys (c/r/f) using blockKeys prop
  // This prevents shortcuts from triggering while typing in the filter
  useAppInput((input, key) => {
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

    // Toggle selection with space (only in branch selection mode)
    if (input === " " && !filterMode) {
      const target = filteredBranches[selectedIndex];
      if (target) {
        onToggleSelect?.(target.name);
      }
      return;
    }

    // Tab key to cycle view mode (only in branch selection mode)
    if (key.tab && !filterMode) {
      cycleViewMode();
      setSelectedIndex(0); // Reset cursor position on mode change
      return;
    }

    // Disable global shortcuts while in filter mode
    if (filterMode) {
      return;
    }

    // Global shortcuts (blocked by Input component when typing in filter mode)
    if (input === "c") {
      onCleanupCommand?.();
    } else if (input === "x") {
      onRepairCommand?.();
    } else if (input === "r" && onRefresh) {
      onRefresh();
    } else if (input === "p" && onOpenProfiles) {
      onOpenProfiles();
    } else if (input === "l" && onOpenLogs) {
      onOpenLogs();
    }
  });

  // Filter branches based on view mode and query
  const filteredBranches = useMemo(() => {
    let result = branches;

    // Apply view mode filter
    if (viewMode === "local") {
      result = result.filter((branch) => branch.type === "local");
    } else if (viewMode === "remote") {
      // リモート専用ブランチ OR ローカルだがリモートにも存在するブランチ
      result = result.filter(
        (branch) =>
          branch.type === "remote" || branch.hasRemoteCounterpart === true,
      );
    }

    // Apply search filter
    if (filterQuery.trim()) {
      const query = filterQuery.toLowerCase();
      result = result.filter((branch) => {
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
    }

    return result;
  }, [branches, viewMode, filterQuery]);

  useEffect(() => {
    setSelectedIndex((prev) => {
      if (filteredBranches.length === 0) {
        return 0;
      }
      return Math.min(prev, filteredBranches.length - 1);
    });
  }, [filteredBranches.length]);

  // Calculate available space for branch list
  // Header: 2 lines (title + divider)
  // Filter input: 1 line
  // Stats: 1 line
  // Empty line: 1 line
  // Selected branch path: 1 line
  // Footer: 1 line
  // Total fixed: 7 lines
  const headerLines = 2;
  const filterLines = 1;
  const statsLines = 1;
  const emptyLine = 1;
  const branchPathLines = 1;
  const footerLines = 1;
  const fixedLines =
    headerLines +
    filterLines +
    statsLines +
    emptyLine +
    branchPathLines +
    footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight); // Minimum 5 items visible

  // Footer actions
  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "f", description: "Filter" },
    { key: "tab", description: "Mode" },
    { key: "r", description: "Refresh" },
    { key: "c", description: "Cleanup" },
    { key: "x", description: "Repair" },
    { key: "p", description: "Profiles" },
    { key: "l", description: "Logs" },
  ];

  const selectedBranchName =
    filteredBranches[selectedIndex]?.name ?? "(none)";
  const selectedBranchLabel = `Branch: ${selectedBranchName}`;

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

  const colorToolLabel = useCallback(
    (label: string, toolId?: string | null) => {
      switch (toolId) {
        case "claude-code":
          return chalk.hex("#ffaf00")(label); // orange-ish
        case "codex-cli":
          return chalk.cyan(label);
        case "gemini-cli":
          return chalk.magenta(label);
        default: {
          const trimmed = label.trim().toLowerCase();
          if (!toolId || trimmed === "unknown") {
            return chalk.gray(label);
          }
          return chalk.white(label);
        }
      }
    },
    [],
  );

  const renderBranchRow = useCallback(
    (item: BranchItem, isSelected: boolean, context: { columns: number }) => {
      // 端末幅ピッタリでの自動折返しを避けるため、1桁だけ余白を取る
      const columns = Math.max(20, context.columns - 1);
      const visibleWidth = (value: string) =>
        measureDisplayWidth(stripAnsi(value));
      // FR-041: Display latest activity time (max of git commit and tool usage)
      let commitText = "---";
      const latestActivitySec = getLatestActivityTimestamp(item);
      if (latestActivitySec > 0) {
        commitText = formatLatestCommit(latestActivitySec);
      }
      const toolLabelRaw =
        item.lastToolUsageLabel?.split("|")?.[0]?.trim() ??
        item.lastToolUsage?.toolId ??
        "Unknown";

      const formatFixedWidth = (value: string, targetWidth: number) => {
        let v = value;
        if (measureDisplayWidth(v) > targetWidth) {
          v = truncateToWidth(v, targetWidth);
        }
        const padding = Math.max(0, targetWidth - measureDisplayWidth(v));
        return v + " ".repeat(padding);
      };

      const TOOL_WIDTH = 7;
      const DATE_WIDTH = 16; // "YYYY-MM-DD HH:mm"
      const paddedTool = formatFixedWidth(toolLabelRaw, TOOL_WIDTH);
      const paddedDate =
        commitText === "---"
          ? " ".repeat(DATE_WIDTH)
          : commitText.padStart(DATE_WIDTH, " ");
      const timestampText = `${paddedTool} | ${paddedDate}`;
      const displayTimestampText = `${colorToolLabel(
        paddedTool,
        item.lastToolUsage?.toolId,
      )} | ${paddedDate}`;
      const timestampWidth = measureDisplayWidth(timestampText);

      // Determine the leading indicator (cursor or cleanup status)
      const indicatorInfo = cleanupUI?.indicators?.[item.name];
      let leadingIndicator: string;
      if (indicatorInfo) {
        // Use static spinner icon if isSpinning to avoid re-render dependency
        // The static "⠋" provides visual feedback without causing flicker
        let indicatorIcon = indicatorInfo.isSpinning ? "⠋" : indicatorInfo.icon;
        if (indicatorIcon && indicatorInfo.color && !isSelected) {
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
        leadingIndicator = indicatorIcon;
      } else {
        // Normal cursor
        leadingIndicator = isSelected ? ">" : " ";
      }

      const isChecked = selectedSet.has(item.name);
      const isWarning = Boolean(item.hasUnpushedCommits) || !item.mergedPR;
      const selectionIcon = isChecked
        ? isWarning
          ? chalk.red("[*]")
          : "[*]"
        : "[ ]";
      let worktreeIcon = chalk.gray("⚪");
      if (item.worktreeStatus === "active") {
        worktreeIcon = chalk.green("🟢");
      } else if (item.worktreeStatus === "inaccessible") {
        worktreeIcon = chalk.red("🔴");
      }
      const safeIcon =
        item.safeToCleanup === true ? chalk.green("🛡") : chalk.yellow("⚠");
      const stateCluster = `${selectionIcon} ${worktreeIcon} ${safeIcon}`;

      const staticPrefix = `${leadingIndicator} ${stateCluster} `;
      const staticPrefixWidth = visibleWidth(staticPrefix);
      const maxLeftDisplayWidth = Math.max(0, columns - timestampWidth - 1);
      const maxLabelWidth = Math.max(
        0,
        maxLeftDisplayWidth - staticPrefixWidth,
      );
      const displayLabel =
        item.type === "remote" && item.remoteName ? item.remoteName : item.name;
      let truncatedLabel = truncateToWidth(displayLabel, maxLabelWidth);
      let leftText = `${staticPrefix}${truncatedLabel}`;

      let leftDisplayWidth = visibleWidth(leftText);
      // Gap between labelとツール/日時。右端に寄せるため必要分だけ確保。
      let gapWidth = Math.max(1, columns - leftDisplayWidth - timestampWidth);

      // もしまだオーバーする場合、隙間→ラベルの順で削って収める
      let totalWidth = leftDisplayWidth + gapWidth + timestampWidth;
      if (totalWidth > columns) {
        const overflow = totalWidth - columns;
        const reducedGap = Math.max(1, gapWidth - overflow);
        gapWidth = reducedGap;
        totalWidth = leftDisplayWidth + gapWidth + timestampWidth;
      }
      if (leftDisplayWidth + gapWidth + timestampWidth > columns) {
        const extra = leftDisplayWidth + gapWidth + timestampWidth - columns;
        const newLabelWidth = Math.max(
          0,
          measureDisplayWidth(truncatedLabel) - extra,
        );
        truncatedLabel = truncateToWidth(displayLabel, newLabelWidth);
        leftText = `${staticPrefix}${truncatedLabel}`;
        leftDisplayWidth = visibleWidth(leftText);
        gapWidth = Math.max(1, columns - leftDisplayWidth - timestampWidth);
      }

      const buildLine = () =>
        `${leftText}${" ".repeat(gapWidth)}${timestampText}`;

      let line = buildLine();
      // Replace timestamp with colorized tool name (keep alignment from width calc)
      let lineWithColoredTimestamp = line.replace(
        timestampText,
        displayTimestampText,
      );

      // 端末幅を超えた場合は隙間→ラベルの順で詰めて収める
      const clampToWidth = () => {
        const finalWidth = measureDisplayWidth(
          stripAnsi(lineWithColoredTimestamp),
        );
        if (finalWidth <= columns) {
          return;
        }
        const overflow = finalWidth - columns;
        const reducedGap = Math.max(1, gapWidth - overflow);
        gapWidth = reducedGap;
        line = buildLine();
        lineWithColoredTimestamp = line.replace(
          timestampText,
          displayTimestampText,
        );
        const widthAfterGap = measureDisplayWidth(
          stripAnsi(lineWithColoredTimestamp),
        );
        if (widthAfterGap > columns) {
          const extra = widthAfterGap - columns;
          const newLabelWidth = Math.max(
            0,
            measureDisplayWidth(truncatedLabel) - extra,
          );
          truncatedLabel = truncateToWidth(displayLabel, newLabelWidth);
          leftText = `${staticPrefix}${truncatedLabel}`;
          leftDisplayWidth = visibleWidth(leftText);
          gapWidth = Math.max(1, columns - leftDisplayWidth - timestampWidth);
          line = buildLine();
          lineWithColoredTimestamp = line.replace(
            timestampText,
            displayTimestampText,
          );
        }
      };

      clampToWidth();

      const output = isSelected
        ? `\u001b[46m\u001b[30m${lineWithColoredTimestamp}\u001b[0m`
        : lineWithColoredTimestamp;
      return <Text>{output}</Text>;
    },
    [
      cleanupUI,
      formatLatestCommit,
      truncateToWidth,
      selectedSet,
      colorToolLabel,
    ],
  );

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header
        title="gwt - Branch Selection"
        titleColor="cyan"
        version={version}
        {...(workingDirectory !== undefined && { workingDirectory })}
        activeProfile={activeProfile}
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
            blockKeys={["c", "r", "f", "l"]} // Block shortcuts while typing
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

      {/* Tool Status - FR-019, FR-021, FR-022 */}
      {toolStatuses && toolStatuses.length > 0 && (
        <Box>
          <Text dimColor>Tools: </Text>
          {toolStatuses.map((tool, index) => (
            <React.Fragment key={tool.id}>
              <Text>{tool.name}: </Text>
              <Text color={tool.status === "installed" ? "green" : "yellow"}>
                {tool.status === "installed" && tool.version
                  ? tool.version
                  : tool.status}
              </Text>
              {index < toolStatuses.length - 1 && <Text dimColor> | </Text>}
            </React.Fragment>
          ))}
        </Box>
      )}

      {/* Stats */}
      <Box>
        <Stats stats={stats} lastUpdated={lastUpdated} viewMode={viewMode} />
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
            <>
              <Select
                items={filteredBranches}
                onSelect={onSelect}
                limit={limit}
                disabled={Boolean(cleanupUI?.inputLocked)}
                renderIndicator={() => null}
                renderItem={renderBranchRow}
                selectedIndex={selectedIndex}
                onSelectedIndexChange={setSelectedIndex}
              />
            </>
          )}
      </Box>

      {cleanupUI?.footerMessage && (
        <Box marginBottom={1}>
          {cleanupUI.footerMessage.color ? (
            <Text color={cleanupUI.footerMessage.color}>
              {cleanupUI.footerMessage.isSpinning && spinnerFrame
                ? `${spinnerFrame} ${cleanupUI.footerMessage.text}`
                : cleanupUI.footerMessage.text}
            </Text>
          ) : (
            <Text>
              {cleanupUI.footerMessage.isSpinning && spinnerFrame
                ? `${spinnerFrame} ${cleanupUI.footerMessage.text}`
                : cleanupUI.footerMessage.text}
            </Text>
          )}
        </Box>
      )}

      <Box>
        <Text dimColor>{selectedBranchLabel}</Text>
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
});
