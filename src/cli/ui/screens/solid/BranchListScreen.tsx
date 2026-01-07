/** @jsxImportSource @opentui/solid */
import { createEffect, createMemo, createSignal } from "solid-js";
import { useKeyboard, useTerminalDimensions } from "@opentui/solid";
import { TextAttributes } from "@opentui/core";
import type { BranchItem, BranchViewMode, Statistics } from "../../types.js";
import type { ToolStatus } from "../../../../utils/command.js";
import { getLatestActivityTimestamp } from "../../utils/branchFormatter.js";
import stringWidth from "string-width";
import { Header } from "../../components/solid/Header.js";
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
  onRepairWorktrees?: () => void;
  onCreateBranch?: (branch: BranchItem | null) => void;
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
  helpVisible?: boolean;
  /** ウィザードポップアップ表示中は入力を無効化 */
  wizardVisible?: boolean;
}

const VIEW_MODES: BranchViewMode[] = ["all", "local", "remote"];

const getCharWidth = (char: string): number => stringWidth(char);

const measureDisplayWidth = (value: string): number => stringWidth(value);

const truncateToWidth = (value: string, maxWidth: number): string => {
  if (maxWidth <= 0) {
    return "";
  }

  if (measureDisplayWidth(value) <= maxWidth) {
    return value;
  }

  const ellipsis = "...";
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

interface TextSegment {
  text: string;
  fg?: string | undefined;
  bg?: string | undefined;
  attributes?: number | undefined;
}

const segmentStyleKey = (segment: TextSegment): string =>
  `${segment.fg ?? ""}|${segment.bg ?? ""}|${segment.attributes ?? ""}`;

const appendSegment = (segments: TextSegment[], segment: TextSegment) => {
  if (!segment.text) {
    return;
  }
  const last = segments[segments.length - 1];
  if (last && segmentStyleKey(last) === segmentStyleKey(segment)) {
    last.text += segment.text;
    return;
  }
  segments.push({ ...segment });
};

const measureSegmentsWidth = (segments: TextSegment[]): number =>
  segments.reduce(
    (total, segment) => total + measureDisplayWidth(segment.text),
    0,
  );

const truncateSegmentsToWidth = (
  segments: TextSegment[],
  maxWidth: number,
): TextSegment[] => {
  if (maxWidth <= 0) {
    return [];
  }
  const ellipsis = "...";
  const ellipsisWidth = measureDisplayWidth(ellipsis);
  if (ellipsisWidth >= maxWidth) {
    return [{ text: ellipsis }];
  }

  const targetWidth = maxWidth - ellipsisWidth;
  const result: TextSegment[] = [];
  let currentWidth = 0;
  let truncated = false;

  for (const segment of segments) {
    for (const char of Array.from(segment.text)) {
      const charWidth = getCharWidth(char);
      if (currentWidth + charWidth > targetWidth) {
        truncated = true;
        break;
      }
      appendSegment(result, {
        text: char,
        fg: segment.fg,
        bg: segment.bg,
        attributes: segment.attributes,
      });
      currentWidth += charWidth;
    }
    if (truncated) {
      break;
    }
  }

  appendSegment(result, { text: ellipsis });
  return result;
};

const fitSegmentsToWidth = (
  segments: TextSegment[],
  width: number,
): TextSegment[] => {
  const totalWidth = measureSegmentsWidth(segments);
  if (totalWidth === width) {
    return segments;
  }
  if (totalWidth > width) {
    return truncateSegmentsToWidth(segments, width);
  }
  const padding = " ".repeat(Math.max(0, width - totalWidth));
  if (padding) {
    return [...segments, { text: padding }];
  }
  return segments;
};

const applySelectionStyle = (segments: TextSegment[]): TextSegment[] =>
  segments.map((segment) => ({
    text: segment.text,
    fg: "black",
    bg: "cyan",
  }));

const CLEANUP_SPINNER_FRAMES = ["-", "\\", "|", "/"];

const CURSOR_FRAMES = ["|", " "];

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

const getToolColor = (label: string, toolId?: string | null): string => {
  switch (toolId) {
    case "claude-code":
      return "yellow";
    case "codex-cli":
      return "cyan";
    case "gemini-cli":
      return "magenta";
    default: {
      const trimmed = label.trim().toLowerCase();
      if (!toolId || trimmed === "unknown") {
        return "gray";
      }
      return "white";
    }
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
  const [cleanupSpinnerIndex, setCleanupSpinnerIndex] = createSignal(0);
  const [cursorIndex, setCursorIndex] = createSignal(0);

  const layoutWidth = createMemo(() => Math.max(20, terminal().width || 80));
  const listWidth = createMemo(() => Math.max(20, layoutWidth()));

  const fixedLines = createMemo(() => {
    const headerLines = 2 + (props.workingDirectory ? 1 : 0);
    const filterLines = 1;
    const toolLines =
      props.toolStatuses && props.toolStatuses.length > 0 ? 1 : 0;
    const statsLines = 1;
    const loadingLines = 1;
    const footerMessageLines = props.cleanupUI?.footerMessage ? 1 : 0;
    const branchLines = 1;
    const footerLines = 1;
    return (
      headerLines +
      filterLines +
      toolLines +
      statsLines +
      loadingLines +
      footerMessageLines +
      branchLines +
      footerLines
    );
  });

  const contentHeight = createMemo(() => {
    const height = terminal().height || 24;
    return Math.max(3, height - fixedLines());
  });

  const selectedSet = createMemo(() => new Set(props.selectedBranches ?? []));

  const hasSpinningIndicator = createMemo(() => {
    if (!props.cleanupUI?.indicators) {
      return false;
    }
    return Object.values(props.cleanupUI.indicators).some(
      (indicator) => indicator.isSpinning,
    );
  });

  const hasSpinningFooter = createMemo(
    () => props.cleanupUI?.footerMessage?.isSpinning ?? false,
  );

  const cleanupSpinnerActive = createMemo(
    () => hasSpinningIndicator() || hasSpinningFooter(),
  );

  createEffect(() => {
    let intervalTimer: ReturnType<typeof setInterval> | undefined;
    if (cleanupSpinnerActive()) {
      intervalTimer = setInterval(() => {
        setCleanupSpinnerIndex(
          (current) => (current + 1) % CLEANUP_SPINNER_FRAMES.length,
        );
      }, 120);
    } else {
      setCleanupSpinnerIndex(0);
    }
    return () => {
      if (intervalTimer) {
        clearInterval(intervalTimer);
      }
    };
  });

  const cleanupSpinnerFrame = createMemo(() =>
    cleanupSpinnerActive()
      ? (CLEANUP_SPINNER_FRAMES[cleanupSpinnerIndex()] ??
        CLEANUP_SPINNER_FRAMES[0])
      : null,
  );

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

  createEffect(() => {
    let intervalTimer: ReturnType<typeof setInterval> | undefined;
    if (filterMode()) {
      intervalTimer = setInterval(() => {
        setCursorIndex((current) => (current + 1) % CURSOR_FRAMES.length);
      }, 500);
    } else {
      setCursorIndex(0);
    }
    return () => {
      if (intervalTimer) {
        clearInterval(intervalTimer);
      }
    };
  });

  const cursorFrame = createMemo(() =>
    filterMode()
      ? (CURSOR_FRAMES[cursorIndex()] ?? CURSOR_FRAMES[0] ?? "")
      : "",
  );

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
    // FR-044: ウィザードポップアップ表示中は入力を無効化
    if (props.wizardVisible) {
      return;
    }
    if (props.helpVisible) {
      return;
    }
    if (props.cleanupUI?.inputLocked) {
      return;
    }

    if (filterMode()) {
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
      key.preventDefault();
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

    if (key.name === "n" || key.sequence === "n") {
      key.preventDefault();
      const selected = filteredBranches()[selectedIndex()] ?? null;
      props.onCreateBranch?.(selected);
      return;
    }

    if (key.name === "c" || key.sequence === "c") {
      props.onCleanupCommand?.();
    } else if (key.name === "x" || key.sequence === "x") {
      props.onRepairWorktrees?.();
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
    let leadingIndicator = "";
    if (indicatorInfo) {
      leadingIndicator = indicatorInfo.isSpinning
        ? (CLEANUP_SPINNER_FRAMES[0] ?? "-")
        : indicatorInfo.icon;
    }
    const indicatorColor =
      !isSelected && indicatorInfo?.color ? indicatorInfo.color : undefined;

    const isChecked = selectedSet().has(branch.name);
    const isWarning = Boolean(branch.hasUnpushedCommits) || !branch.mergedPR;
    const selectionIcon = isChecked ? "[*]" : "[ ]";
    const selectionColor = isChecked && isWarning ? "red" : undefined;
    let worktreeIcon = ".";
    let worktreeColor: IndicatorColor | "gray" = "gray";
    if (branch.worktreeStatus === "active") {
      worktreeIcon = "w";
      worktreeColor = "green";
    } else if (branch.worktreeStatus === "inaccessible") {
      worktreeIcon = "x";
      worktreeColor = "red";
    }
    const safeIcon = branch.safeToCleanup === true ? " " : "!";
    const safeColor: IndicatorColor | undefined =
      branch.safeToCleanup === true ? undefined : "red";

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
    const toolColor = getToolColor(paddedTool, branch.lastToolUsage?.toolId);

    const displayLabel =
      branch.type === "remote" && branch.remoteName
        ? branch.remoteName
        : branch.name;

    const staticPrefix = `${leadingIndicator}${selectionIcon} ${worktreeIcon} ${safeIcon} `;
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
      }
    }

    const segments: TextSegment[] = [];
    if (leadingIndicator) {
      appendSegment(segments, { text: leadingIndicator, fg: indicatorColor });
    }
    appendSegment(segments, { text: selectionIcon, fg: selectionColor });
    appendSegment(segments, { text: " " });
    appendSegment(segments, { text: worktreeIcon, fg: worktreeColor });
    appendSegment(segments, { text: " " });
    appendSegment(segments, { text: safeIcon, fg: safeColor });
    appendSegment(segments, { text: " " });
    appendSegment(segments, { text: truncatedLabel });
    appendSegment(segments, { text: " ".repeat(gapWidth) });
    appendSegment(segments, { text: paddedTool, fg: toolColor });
    appendSegment(segments, { text: " | " });
    appendSegment(segments, { text: paddedDate });

    const lineSegments = fitSegmentsToWidth(segments, columns);
    return isSelected ? applySelectionStyle(lineSegments) : lineSegments;
  };

  const footerActions = [
    { key: "r", description: "Refresh" },
    { key: "c", description: "Cleanup" },
    { key: "x", description: "Repair" },
    { key: "l", description: "Logs" },
  ];

  const filterLineSegments = createMemo(() => {
    const segments: TextSegment[] = [];
    const query = filterQuery();

    appendSegment(segments, {
      text: "Filter(f): ",
      attributes: TextAttributes.DIM,
    });
    if (filterMode()) {
      if (query) {
        appendSegment(segments, { text: query });
      } else {
        appendSegment(segments, {
          text: "Type to search...",
          attributes: TextAttributes.DIM,
        });
      }
      appendSegment(segments, { text: cursorFrame() });
    } else {
      appendSegment(segments, {
        text: query || "(press f to filter)",
        attributes: TextAttributes.DIM,
      });
    }

    if (query) {
      appendSegment(segments, {
        text: ` (Showing ${filteredBranches().length} of ${props.branches.length})`,
        attributes: TextAttributes.DIM,
      });
    }

    return fitSegmentsToWidth(segments, layoutWidth());
  });

  const toolStatusSegments = createMemo(() => {
    const toolStatuses = props.toolStatuses ?? [];
    if (toolStatuses.length === 0) {
      return null;
    }

    const segments: TextSegment[] = [];
    appendSegment(segments, {
      text: "Tools: ",
      attributes: TextAttributes.DIM,
    });
    toolStatuses.forEach((tool, index) => {
      const statusLabel =
        tool.status === "installed" && tool.version
          ? tool.version
          : tool.status;
      appendSegment(segments, { text: `${tool.name}: ` });
      appendSegment(segments, {
        text: statusLabel,
        fg: tool.status === "installed" ? "green" : "yellow",
      });
      if (index < toolStatuses.length - 1) {
        appendSegment(segments, {
          text: " | ",
          attributes: TextAttributes.DIM,
        });
      }
    });

    return fitSegmentsToWidth(segments, layoutWidth());
  });

  const statsSegments = createMemo(() => {
    const segments: TextSegment[] = [];
    const separator = "  ";

    appendSegment(segments, {
      text: "Mode(tab): ",
      attributes: TextAttributes.DIM,
    });
    appendSegment(segments, {
      text: formatViewModeLabel(viewMode()),
      fg: "white",
      attributes: TextAttributes.BOLD,
    });
    appendSegment(segments, {
      text: separator,
      attributes: TextAttributes.DIM,
    });

    const statsItems = [
      { label: "Local", value: props.stats.localCount, color: "cyan" },
      { label: "Remote", value: props.stats.remoteCount, color: "green" },
      { label: "Worktrees", value: props.stats.worktreeCount, color: "yellow" },
      { label: "Changes", value: props.stats.changesCount, color: "magenta" },
    ];

    statsItems.forEach((item, index) => {
      appendSegment(segments, {
        text: `${item.label}: `,
        attributes: TextAttributes.DIM,
      });
      appendSegment(segments, {
        text: String(item.value),
        fg: item.color,
        attributes: TextAttributes.BOLD,
      });
      if (index < statsItems.length - 1 || props.lastUpdated) {
        appendSegment(segments, {
          text: separator,
          attributes: TextAttributes.DIM,
        });
      }
    });

    if (props.lastUpdated) {
      appendSegment(segments, {
        text: "Updated: ",
        attributes: TextAttributes.DIM,
      });
      appendSegment(segments, {
        text: formatRelativeTime(props.lastUpdated),
        fg: "gray",
      });
    }

    return fitSegmentsToWidth(segments, layoutWidth());
  });

  const footerSegments = createMemo(() => {
    const segments: TextSegment[] = [];
    const separator = "  ";
    footerActions.forEach((action, index) => {
      appendSegment(segments, { text: "[", attributes: TextAttributes.DIM });
      appendSegment(segments, {
        text: action.key,
        fg: "cyan",
        attributes: TextAttributes.BOLD,
      });
      appendSegment(segments, { text: "]", attributes: TextAttributes.DIM });
      appendSegment(segments, { text: ` ${action.description}` });
      if (index < footerActions.length - 1) {
        appendSegment(segments, {
          text: separator,
          attributes: TextAttributes.DIM,
        });
      }
    });

    return fitSegmentsToWidth(segments, layoutWidth());
  });

  const renderSegmentLine = (segments: TextSegment[]) => (
    <box flexDirection="row">
      {segments.map((segment) => (
        <text
          {...(segment.fg ? { fg: segment.fg } : {})}
          {...(segment.bg ? { bg: segment.bg } : {})}
          {...(segment.attributes ? { attributes: segment.attributes } : {})}
        >
          {segment.text}
        </text>
      ))}
    </box>
  );

  const selectedWorktreeLabel = createMemo(() => {
    const branches = filteredBranches();
    if (branches.length === 0) {
      return "(none)";
    }
    const selected = branches[selectedIndex()];
    if (!selected) {
      return "(none)";
    }
    if (selected.worktree?.path) {
      return selected.worktree.path;
    }
    if (selected.isCurrent && props.workingDirectory) {
      return props.workingDirectory;
    }
    return "(none)";
  });

  return (
    <box flexDirection="column" height={terminal().height || 24}>
      <Header
        title="gwt - Branch Selection"
        titleColor="cyan"
        width={layoutWidth()}
        {...(props.version !== undefined ? { version: props.version } : {})}
        {...(props.workingDirectory
          ? { workingDirectory: props.workingDirectory }
          : {})}
        {...(props.activeProfile !== undefined
          ? { activeProfile: props.activeProfile }
          : {})}
      />

      {renderSegmentLine(filterLineSegments())}

      {toolStatusSegments() && renderSegmentLine(toolStatusSegments() ?? [])}

      {renderSegmentLine(statsSegments())}

      <box flexDirection="column" flexGrow={1}>
        {Boolean(props.loading) && props.branches.length === 0 ? (
          <LoadingIndicator
            isLoading
            message="Loading Git information..."
            width={layoutWidth()}
            {...(props.loadingIndicatorDelay !== undefined
              ? { delay: props.loadingIndicatorDelay }
              : {})}
          />
        ) : (
          <text>{padLine("", layoutWidth())}</text>
        )}

        {props.error && (
          <>
            <text fg="red" attributes={TextAttributes.BOLD}>
              {padLine(`Error: ${props.error.message}`, layoutWidth())}
            </text>
            {process.env.DEBUG && props.error.stack && (
              <>
                <text attributes={TextAttributes.DIM}>
                  {padLine("", layoutWidth())}
                </text>
                {props.error.stack.split("\n").map((line) => (
                  <text fg="gray" attributes={TextAttributes.DIM}>
                    {padLine(line, layoutWidth())}
                  </text>
                ))}
              </>
            )}
          </>
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
              const rowSegments = renderBranchLine(branch, index);
              return renderSegmentLine(rowSegments);
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
        <text
          {...(props.cleanupUI.footerMessage.color
            ? { fg: props.cleanupUI.footerMessage.color }
            : {})}
        >
          {padLine(
            props.cleanupUI.footerMessage.isSpinning && cleanupSpinnerFrame()
              ? `${cleanupSpinnerFrame()} ${props.cleanupUI.footerMessage.text}`
              : props.cleanupUI.footerMessage.text,
            layoutWidth(),
          )}
        </text>
      )}

      <text attributes={TextAttributes.DIM}>
        {padLine(`Worktree: ${selectedWorktreeLabel()}`, layoutWidth())}
      </text>

      {renderSegmentLine(footerSegments())}
    </box>
  );
}
