import { createEffect, createMemo, createSignal } from "solid-js";
import { useKeyboard, useTerminalDimensions } from "@opentui/solid";
import { TextAttributes } from "@opentui/core";
import type { BranchItem, BranchViewMode, Statistics } from "../../types.js";
import type { ToolStatus } from "../../hooks/useToolStatus.js";
import { getLatestActivityTimestamp } from "../../utils/branchFormatter.js";
import stringWidth from "string-width";

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

const WIDTH_OVERRIDES: Record<string, number> = {
  "☁": 1,
  "⚡": 1,
  "✨": 1,
  "🐛": 1,
  "🔥": 1,
  "🚀": 1,
  "📌": 1,
  "🟢": 2,
  "⚪": 2,
  "🔴": 2,
  "👉": 1,
  "💾": 1,
  "📤": 1,
  "🔃": 1,
  "✅": 2,
  "⚠": 2,
  "⚠️": 2,
  "🛡": 2,
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

const truncateToWidth = (value: string, maxWidth: number): string => {
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
};

const padLine = (value: string, width: number): string => {
  let line = value;
  if (measureDisplayWidth(line) > width) {
    line = truncateToWidth(line, width);
  }
  const padding = Math.max(0, width - measureDisplayWidth(line));
  return line + " ".repeat(padding);
};

const formatRelativeTime = (date: Date): string => {
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSec = Math.floor(diffMs / 1000);

  if (diffSec < 60) {
    return `${diffSec}s ago`;
  }

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) {
    return `${diffMin}m ago`;
  }

  const diffHour = Math.floor(diffMin / 60);
  return `${diffHour}h ago`;
};

const formatViewModeLabel = (mode: BranchViewMode): string => {
  switch (mode) {
    case "all":
      return "All";
    case "local":
      return "Local";
    case "remote":
      return "Remote";
  }
};

interface LoadingIndicatorProps {
  isLoading: boolean;
  delay?: number;
  message?: string;
  width: number;
}

function LoadingIndicator(props: LoadingIndicatorProps) {
  const frames = ["|", "/", "-", "\\"];
  const delay = props.delay ?? 300;
  const [visible, setVisible] = createSignal(props.isLoading && delay <= 0);
  const [frameIndex, setFrameIndex] = createSignal(0);

  createEffect(() => {
    let delayTimer: ReturnType<typeof setTimeout> | undefined;
    let intervalTimer: ReturnType<typeof setInterval> | undefined;

    if (props.isLoading) {
      if (delay <= 0) {
        setVisible(true);
      } else {
        delayTimer = setTimeout(() => {
          setVisible(true);
        }, delay);
      }
    } else {
      setVisible(false);
      setFrameIndex(0);
    }

    if (visible() && props.isLoading) {
      intervalTimer = setInterval(() => {
        setFrameIndex((current) => (current + 1) % frames.length);
      }, 80);
    }

    return () => {
      if (delayTimer) {
        clearTimeout(delayTimer);
      }
      if (intervalTimer) {
        clearInterval(intervalTimer);
      }
    };
  });

  if (!props.isLoading || !visible()) {
    return null;
  }

  return (
    <text fg="yellow">
      {padLine(
        `${frames[frameIndex()]} ${props.message ?? "Loading... please wait"}`,
        props.width,
      )}
    </text>
  );
}

export function BranchListScreen(props: BranchListScreenProps) {
  const terminal = useTerminalDimensions();

  const [filterQuery, setFilterQuery] = createSignal("");
  const [filterMode, setFilterMode] = createSignal(false);
  const [viewMode, setViewMode] = createSignal<BranchViewMode>("all");
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [scrollOffset, setScrollOffset] = createSignal(0);

  const layoutWidth = createMemo(() => Math.max(20, terminal().width || 80));
  const listWidth = createMemo(() => Math.max(20, layoutWidth() - 1));

  const headerTitle = createMemo(() => {
    let title = "gwt - Branch Selection";
    if (props.version) {
      title = `${title} v${props.version}`;
    }
    if (props.activeProfile !== undefined) {
      title = `${title} | Profile: ${props.activeProfile ?? "(none)"}`;
    }
    return title;
  });

  const dividerLine = createMemo(() => "─".repeat(layoutWidth()));

  const fixedLines = createMemo(() => {
    const headerLines = 2 + (props.workingDirectory ? 1 : 0);
    const filterLines = 1;
    const toolLines =
      props.toolStatuses && props.toolStatuses.length > 0 ? 1 : 0;
    const statsLines = 1;
    const footerMessageLines = props.cleanupUI?.footerMessage ? 1 : 0;
    const footerLines = 1;
    return (
      headerLines +
      filterLines +
      toolLines +
      statsLines +
      footerMessageLines +
      footerLines
    );
  });

  const contentHeight = createMemo(() => {
    const height = terminal().height || 24;
    return Math.max(3, height - fixedLines());
  });

  const selectedSet = createMemo(() => new Set(props.selectedBranches ?? []));

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

  const formatLatestCommit = (timestamp?: number) => {
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
  };

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

    if (key.ctrl && key.name === "c") {
      props.onQuit?.();
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
      setSelectedIndex(0);
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
      return;
    }

    if (key.sequence === " " || key.name === "space") {
      const selected = filteredBranches()[selectedIndex()];
      if (selected) {
        props.onToggleSelect?.(selected.name);
      }
      return;
    }

    if (key.name === "c" || key.sequence === "c") {
      props.onCleanupCommand?.();
    } else if (key.name === "r" || key.sequence === "r") {
      props.onRefresh?.();
    } else if (key.name === "p" || key.sequence === "p") {
      props.onOpenProfiles?.();
    } else if (key.name === "l" || key.sequence === "l") {
      props.onOpenLogs?.();
    }
  });

  const visibleBranches = createMemo(() => {
    const start = scrollOffset();
    const limit = contentHeight();
    return filteredBranches().slice(start, start + limit);
  });

  const renderBranchLine = (branch: BranchItem, index: number) => {
    const absoluteIndex = scrollOffset() + index;
    const isSelected = absoluteIndex === selectedIndex();
    const columns = listWidth();

    const indicatorInfo = props.cleanupUI?.indicators?.[branch.name];
    let leadingIndicator = isSelected ? ">" : " ";
    if (indicatorInfo) {
      leadingIndicator = indicatorInfo.isSpinning ? "⠋" : indicatorInfo.icon;
    }

    const isChecked = selectedSet().has(branch.name);
    const selectionIcon = isChecked ? "[*]" : "[ ]";
    let worktreeIcon = "⚪";
    if (branch.worktreeStatus === "active") {
      worktreeIcon = "🟢";
    } else if (branch.worktreeStatus === "inaccessible") {
      worktreeIcon = "🔴";
    }
    const safeIcon = branch.safeToCleanup === true ? "🛡" : "⚠";

    const stateCluster = `${selectionIcon} ${worktreeIcon} ${safeIcon}`;

    let commitText = "---";
    const latestActivitySec = getLatestActivityTimestamp(branch);
    if (latestActivitySec > 0) {
      commitText = formatLatestCommit(latestActivitySec);
    }

    const toolLabelRaw =
      branch.lastToolUsageLabel?.split("|")?.[0]?.trim() ??
      branch.lastToolUsage?.toolId ??
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
    const DATE_WIDTH = 16;
    const paddedTool = formatFixedWidth(toolLabelRaw, TOOL_WIDTH);
    const paddedDate =
      commitText === "---"
        ? " ".repeat(DATE_WIDTH)
        : commitText.padStart(DATE_WIDTH, " ");
    const timestampText = `${paddedTool} | ${paddedDate}`;
    const timestampWidth = measureDisplayWidth(timestampText);

    const displayLabel =
      branch.type === "remote" && branch.remoteName
        ? branch.remoteName
        : branch.name;

    const staticPrefix = `${leadingIndicator} ${stateCluster} `;
    const staticPrefixWidth = measureDisplayWidth(staticPrefix);
    const maxLeftDisplayWidth = Math.max(0, columns - timestampWidth - 1);
    const maxLabelWidth = Math.max(0, maxLeftDisplayWidth - staticPrefixWidth);
    let truncatedLabel = truncateToWidth(displayLabel, maxLabelWidth);
    let leftText = `${staticPrefix}${truncatedLabel}`;
    let leftDisplayWidth = measureDisplayWidth(leftText);
    let gapWidth = Math.max(1, columns - leftDisplayWidth - timestampWidth);

    const buildLine = () =>
      `${leftText}${" ".repeat(gapWidth)}${timestampText}`;
    let line = buildLine();

    if (measureDisplayWidth(line) > columns) {
      const overflow = measureDisplayWidth(line) - columns;
      gapWidth = Math.max(1, gapWidth - overflow);
      line = buildLine();
      if (measureDisplayWidth(line) > columns) {
        const extra = measureDisplayWidth(line) - columns;
        const newLabelWidth = Math.max(
          0,
          measureDisplayWidth(truncatedLabel) - extra,
        );
        truncatedLabel = truncateToWidth(displayLabel, newLabelWidth);
        leftText = `${staticPrefix}${truncatedLabel}`;
        leftDisplayWidth = measureDisplayWidth(leftText);
        gapWidth = Math.max(1, columns - leftDisplayWidth - timestampWidth);
        line = buildLine();
      }
    }

    return {
      line: padLine(line, columns),
      isSelected,
    };
  };

  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "f", description: "Filter" },
    { key: "tab", description: "Mode" },
    { key: "r", description: "Refresh" },
    { key: "c", description: "Cleanup" },
    { key: "p", description: "Profiles" },
    { key: "l", description: "Logs" },
  ];

  const filterLine = createMemo(() => {
    const query = filterQuery();
    const display = filterMode()
      ? query || "Type to search..."
      : query || "(press f to filter)";
    let line = `Filter: ${display}`;
    if (query) {
      line += ` (Showing ${filteredBranches().length} of ${props.branches.length})`;
    }
    return line;
  });

  const toolStatusLine = createMemo(() => {
    if (!props.toolStatuses || props.toolStatuses.length === 0) {
      return null;
    }
    const parts = props.toolStatuses.map((tool) => {
      const statusLabel =
        tool.status === "installed" && tool.version
          ? tool.version
          : tool.status;
      return `${tool.name}: ${statusLabel}`;
    });
    return `Tools: ${parts.join(" | ")}`;
  });

  const statsLine = createMemo(() => {
    const parts = [
      `Mode: ${formatViewModeLabel(viewMode())}`,
      `Local: ${props.stats.localCount}`,
      `Remote: ${props.stats.remoteCount}`,
      `Worktrees: ${props.stats.worktreeCount}`,
      `Changes: ${props.stats.changesCount}`,
    ];
    if (props.lastUpdated) {
      parts.push(`Updated: ${formatRelativeTime(props.lastUpdated)}`);
    }
    return parts.join("  ");
  });

  const footerLine = createMemo(() => {
    const parts = footerActions.map(
      (action) => `[${action.key}] ${action.description}`,
    );
    return parts.join("  ");
  });

  return (
    <box flexDirection="column" height={terminal().height || 24}>
      <box flexDirection="column">
        <text fg="cyan" attributes={TextAttributes.BOLD}>
          {padLine(headerTitle(), layoutWidth())}
        </text>
        <text attributes={TextAttributes.DIM}>
          {padLine(dividerLine(), layoutWidth())}
        </text>
        {props.workingDirectory && (
          <text attributes={TextAttributes.DIM}>
            {padLine(
              `Working Directory: ${props.workingDirectory}`,
              layoutWidth(),
            )}
          </text>
        )}
      </box>

      <text attributes={TextAttributes.DIM}>
        {padLine(filterLine(), layoutWidth())}
      </text>

      {toolStatusLine() && (
        <text attributes={TextAttributes.DIM}>
          {padLine(toolStatusLine() ?? "", layoutWidth())}
        </text>
      )}

      <text attributes={TextAttributes.DIM}>
        {padLine(statsLine(), layoutWidth())}
      </text>

      <box flexDirection="column" flexGrow={1}>
        <LoadingIndicator
          isLoading={Boolean(props.loading)}
          delay={props.loadingIndicatorDelay}
          message="Loading Git information..."
          width={layoutWidth()}
        />

        {props.error && (
          <text fg="red" attributes={TextAttributes.BOLD}>
            {padLine(`Error: ${props.error.message}`, layoutWidth())}
          </text>
        )}

        {!props.loading && !props.error && props.branches.length === 0 && (
          <text attributes={TextAttributes.DIM}>
            {padLine("No branches found", layoutWidth())}
          </text>
        )}

        {!props.loading &&
          !props.error &&
          props.branches.length > 0 &&
          filteredBranches().length === 0 &&
          filterQuery() && (
            <text attributes={TextAttributes.DIM}>
              {padLine("No branches match your filter", layoutWidth())}
            </text>
          )}

        {!props.loading && !props.error && filteredBranches().length > 0 && (
          <>
            {visibleBranches().map((branch, index) => {
              const row = renderBranchLine(branch, index);
              return (
                <text
                  fg={row.isSelected ? "black" : undefined}
                  bg={row.isSelected ? "cyan" : undefined}
                >
                  {row.line}
                </text>
              );
            })}
            {Array.from({
              length: Math.max(0, contentHeight() - visibleBranches().length),
            }).map(() => (
              <text>{padLine("", listWidth())}</text>
            ))}
          </>
        )}
      </box>

      {props.cleanupUI?.footerMessage && (
        <text fg={props.cleanupUI.footerMessage.color}>
          {padLine(
            props.cleanupUI.footerMessage.isSpinning
              ? `⠋ ${props.cleanupUI.footerMessage.text}`
              : props.cleanupUI.footerMessage.text,
            layoutWidth(),
          )}
        </text>
      )}

      <text>{padLine(footerLine(), layoutWidth())}</text>
    </box>
  );
}
