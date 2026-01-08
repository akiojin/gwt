/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { createEffect, createMemo, createSignal, mergeProps } from "solid-js";
import { TextAttributes } from "@opentui/core";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";
import { useScrollableList } from "../../hooks/solid/useScrollableList.js";
import stringWidth from "string-width";
import { getLogLevelColor, selectionStyle } from "../../core/theme.js";

export interface LogScreenProps {
  entries: FormattedLogEntry[];
  loading?: boolean;
  error?: string | null;
  onBack: () => void;
  onSelect: (entry: FormattedLogEntry) => void;
  onCopy: (entry: FormattedLogEntry) => void;
  onPickDate?: () => void;
  onReload?: () => void;
  onToggleTail?: () => void;
  notification?: { message: string; tone: "success" | "error" } | null;
  version?: string | null;
  selectedDate?: string | null;
  branchLabel?: string | null;
  sourceLabel?: string | null;
  tailing?: boolean;
  helpVisible?: boolean;
}

type LevelMode = "ALL" | "INFO+" | "WARN+" | "ERROR+";

const LEVEL_ORDER: LevelMode[] = ["ALL", "INFO+", "WARN+", "ERROR+"];

const LEVEL_THRESHOLDS: Record<LevelMode, number> = {
  ALL: 0,
  "INFO+": 30,
  "WARN+": 40,
  "ERROR+": 50,
};

const LEVEL_LABEL_VALUES: Record<string, number> = {
  TRACE: 10,
  DEBUG: 20,
  INFO: 30,
  WARN: 40,
  ERROR: 50,
  FATAL: 60,
};

const measureWidth = (value: string): number => stringWidth(value);

const truncateToWidth = (value: string, maxWidth: number): string => {
  if (maxWidth <= 0) {
    return "";
  }
  if (measureWidth(value) <= maxWidth) {
    return value;
  }
  const ellipsis = "...";
  const ellipsisWidth = measureWidth(ellipsis);
  if (ellipsisWidth >= maxWidth) {
    return ellipsis.slice(0, maxWidth);
  }

  let currentWidth = 0;
  let result = "";
  for (const char of Array.from(value)) {
    const charWidth = measureWidth(char);
    if (currentWidth + charWidth + ellipsisWidth > maxWidth) {
      break;
    }
    result += char;
    currentWidth += charWidth;
  }
  return `${result}${ellipsis}`;
};

const padToWidth = (value: string, width: number): string => {
  if (width <= 0) {
    return "";
  }
  const truncated = truncateToWidth(value, width);
  const padding = Math.max(0, width - measureWidth(truncated));
  return `${truncated}${" ".repeat(padding)}`;
};

interface TextSegment {
  text: string;
  fg?: string;
  bg?: string;
  attributes?: TextAttributes;
}

const appendSegment = (segments: TextSegment[], segment: TextSegment) => {
  if (!segment.text) {
    return;
  }
  segments.push(segment);
};

const measureSegmentsWidth = (segments: TextSegment[]): number =>
  segments.reduce((total, segment) => total + measureWidth(segment.text), 0);

const truncateSegmentsToWidth = (
  segments: TextSegment[],
  maxWidth: number,
): TextSegment[] => {
  if (maxWidth <= 0) {
    return [];
  }
  const result: TextSegment[] = [];
  let currentWidth = 0;
  for (const segment of segments) {
    if (currentWidth >= maxWidth) {
      break;
    }
    const segmentWidth = measureWidth(segment.text);
    if (currentWidth + segmentWidth <= maxWidth) {
      result.push(segment);
      currentWidth += segmentWidth;
      continue;
    }
    const remaining = maxWidth - currentWidth;
    const truncated = truncateToWidth(segment.text, remaining);
    if (truncated) {
      result.push({ ...segment, text: truncated });
      currentWidth += measureWidth(truncated);
    }
    break;
  }
  return result;
};

const padSegmentsToWidth = (
  segments: TextSegment[],
  width: number,
  padStyle?: Pick<TextSegment, "fg" | "bg" | "attributes">,
): TextSegment[] => {
  const totalWidth = measureSegmentsWidth(segments);
  if (totalWidth >= width) {
    return segments;
  }
  const padding = " ".repeat(Math.max(0, width - totalWidth));
  if (!padding) {
    return segments;
  }
  return [
    ...segments,
    {
      text: padding,
      ...(padStyle ?? {}),
    },
  ];
};

const applySelectionStyle = (segments: TextSegment[]): TextSegment[] =>
  segments.map((segment) => ({
    text: segment.text,
    fg: selectionStyle.fg,
    bg: selectionStyle.bg,
  }));

const resolveEntryLevel = (entry: FormattedLogEntry): number => {
  const rawLevel = entry.raw?.level;
  if (typeof rawLevel === "number") {
    return rawLevel;
  }
  const label = entry.levelLabel?.toUpperCase() ?? "";
  return LEVEL_LABEL_VALUES[label] ?? 0;
};

export function LogScreen(props: LogScreenProps) {
  const merged = mergeProps(
    {
      loading: false,
      error: null,
      branchLabel: null,
      sourceLabel: null,
      tailing: false,
      helpVisible: false,
    },
    props,
  );
  const terminal = useTerminalSize();
  const [filterQuery, setFilterQuery] = createSignal("");
  const [filterMode, setFilterMode] = createSignal(false);
  const [wrapEnabled, setWrapEnabled] = createSignal(true);
  const [levelMode, setLevelMode] = createSignal<LevelMode>("ALL");

  const filteredEntries = createMemo(() => {
    let result = merged.entries;
    const threshold = LEVEL_THRESHOLDS[levelMode()];
    if (threshold > 0) {
      result = result.filter((entry) => resolveEntryLevel(entry) >= threshold);
    }

    const query = filterQuery().trim().toLowerCase();
    if (!query) {
      return result;
    }
    return result.filter((entry) => {
      const target =
        `${entry.category} ${entry.levelLabel} ${entry.message}`.toLowerCase();
      return target.includes(query);
    });
  });

  const totalCount = createMemo(() => merged.entries.length);
  const filteredCount = createMemo(() => filteredEntries().length);
  const showFilterCount = createMemo(
    () => filterMode() || filterQuery().trim().length > 0,
  );

  const levelWidth = createMemo(() => {
    const MIN = 5;
    return Math.max(
      MIN,
      ...filteredEntries().map((entry) => measureWidth(entry.levelLabel)),
    );
  });

  const categoryWidth = createMemo(() => {
    const MIN = 4;
    const MAX = 20;
    const maxWidth = Math.max(
      MIN,
      ...filteredEntries().map((entry) => measureWidth(entry.category)),
    );
    return Math.min(MAX, maxWidth);
  });

  const listHeight = createMemo(() => {
    const headerRows = 2;
    const infoRows = 3;
    const footerRows = 1;
    const notificationRows = merged.notification ? 1 : 0;
    const reserved = headerRows + infoRows + footerRows + notificationRows;
    return Math.max(1, terminal().rows - reserved);
  });

  const list = useScrollableList({
    items: filteredEntries,
    visibleCount: listHeight,
  });

  const currentEntry = createMemo(
    () => filteredEntries()[list.selectedIndex()],
  );

  createEffect(() => {
    filterQuery();
    levelMode();
    list.setSelectedIndex(0);
    list.setScrollOffset(0);
  });

  const updateSelectedIndex = (value: number | ((prev: number) => number)) => {
    list.setSelectedIndex(value);
  };

  useKeyboard((key) => {
    if (merged.helpVisible) {
      return;
    }

    if (filterMode()) {
      if (key.name === "down") {
        updateSelectedIndex((prev) => prev + 1);
        return;
      }
      if (key.name === "up") {
        updateSelectedIndex((prev) => prev - 1);
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

    if (key.name === "escape" || key.name === "q") {
      merged.onBack();
      return;
    }

    if (key.name === "c") {
      const entry = currentEntry();
      if (entry) {
        merged.onCopy(entry);
      }
      return;
    }

    if (key.name === "d") {
      merged.onPickDate?.();
      return;
    }

    if (key.name === "f") {
      setFilterMode(true);
      return;
    }

    if (key.name === "v") {
      setLevelMode((prev) => {
        const index = LEVEL_ORDER.indexOf(prev);
        const nextIndex = (index + 1) % LEVEL_ORDER.length;
        return LEVEL_ORDER[nextIndex] ?? "ALL";
      });
      return;
    }

    if (key.name === "r") {
      merged.onReload?.();
      return;
    }

    if (key.name === "t") {
      merged.onToggleTail?.();
      return;
    }

    if (key.name === "w") {
      setWrapEnabled((prev) => !prev);
      return;
    }

    if (key.name === "down") {
      updateSelectedIndex((prev) => prev + 1);
      return;
    }

    if (key.name === "up") {
      updateSelectedIndex((prev) => prev - 1);
      return;
    }

    if (key.name === "pageup") {
      updateSelectedIndex((prev) => prev - listHeight());
      return;
    }

    if (key.name === "pagedown") {
      updateSelectedIndex((prev) => prev + listHeight());
      return;
    }

    if (key.name === "home") {
      updateSelectedIndex(0);
      return;
    }

    if (key.name === "end") {
      updateSelectedIndex(filteredEntries().length - 1);
      return;
    }

    if (key.name === "return" || key.name === "linefeed") {
      const entry = currentEntry();
      if (entry) {
        merged.onSelect(entry);
      }
    }
  });

  const footerActions = createMemo(() => {
    const actions = [
      { key: "enter", description: "Detail" },
      { key: "c", description: "Copy" },
      { key: "d", description: "Date" },
      { key: "f", description: "Filter" },
      { key: "v", description: "Level" },
      { key: "r", description: "Reload" },
      { key: "t", description: "Tail" },
      { key: "w", description: "Wrap" },
      { key: "esc", description: "Back" },
    ];
    return actions;
  });

  const formatEntrySegments = (entry: FormattedLogEntry): TextSegment[] => {
    const levelText = padToWidth(entry.levelLabel, levelWidth());
    const categoryText = padToWidth(entry.category, categoryWidth());
    const segments: TextSegment[] = [];
    appendSegment(segments, { text: `[${entry.timeLabel}] ` });
    appendSegment(segments, { text: "[" });
    appendSegment(segments, {
      text: levelText,
      fg: getLogLevelColor(entry.levelLabel),
    });
    appendSegment(segments, { text: "] " });
    appendSegment(segments, { text: "[" });
    appendSegment(segments, { text: categoryText });
    appendSegment(segments, { text: "] " });
    appendSegment(segments, { text: entry.message });
    if (wrapEnabled()) {
      return segments;
    }

    const maxWidth = terminal().columns;
    if (measureSegmentsWidth(segments) <= maxWidth) {
      return segments;
    }
    return truncateSegmentsToWidth(segments, maxWidth);
  };

  const filterLabel = createMemo(() => {
    if (filterQuery()) {
      return filterQuery();
    }
    return filterMode() ? "(type to filter)" : "(press f to filter)";
  });

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header
        title="gwt - Log Viewer"
        titleColor="cyan"
        version={merged.version}
      />

      {merged.notification ? (
        <text fg={merged.notification.tone === "error" ? "red" : "green"}>
          {merged.notification.message}
        </text>
      ) : null}

      <box flexDirection="row">
        <text attributes={TextAttributes.DIM}>Branch: </text>
        <text attributes={TextAttributes.BOLD}>
          {merged.branchLabel ?? "(none)"}
        </text>
        <text attributes={TextAttributes.DIM}> Source: </text>
        <text attributes={TextAttributes.BOLD}>
          {merged.sourceLabel ?? "(none)"}
        </text>
      </box>

      <box flexDirection="row">
        <text attributes={TextAttributes.DIM}>Date: </text>
        <text attributes={TextAttributes.BOLD}>
          {merged.selectedDate ?? "---"}
        </text>
        <text attributes={TextAttributes.DIM}> Total: </text>
        <text attributes={TextAttributes.BOLD}>{totalCount()}</text>
        <text attributes={TextAttributes.DIM}> Level: </text>
        <text attributes={TextAttributes.BOLD}>{levelMode()}</text>
        <text attributes={TextAttributes.DIM}> Tail: </text>
        <text attributes={TextAttributes.BOLD}>
          {merged.tailing ? "ON" : "OFF"}
        </text>
        <text attributes={TextAttributes.DIM}> Wrap: </text>
        <text attributes={TextAttributes.BOLD}>
          {wrapEnabled() ? "ON" : "OFF"}
        </text>
      </box>

      <box flexDirection="row">
        <text attributes={TextAttributes.DIM}>Filter: </text>
        <text attributes={TextAttributes.BOLD}>{filterLabel()}</text>
        {showFilterCount() ? (
          <text attributes={TextAttributes.DIM}>
            {` (Showing ${filteredCount()} of ${totalCount()})`}
          </text>
        ) : null}
      </box>

      <box flexDirection="column" flexGrow={1}>
        {merged.loading ? (
          <text fg="gray">Loading logs...</text>
        ) : filteredEntries().length === 0 ? (
          <text fg="gray">No logs available.</text>
        ) : (
          <box flexDirection="column">
            {list.visibleItems().map((entry, index) => {
              const absoluteIndex = list.scrollOffset() + index;
              const isSelected = absoluteIndex === list.selectedIndex();
              const maxWidth = terminal().columns;
              const baseSegments = formatEntrySegments(entry);
              const selectedSegments = isSelected
                ? applySelectionStyle(baseSegments)
                : baseSegments;
              const displaySegments =
                isSelected && measureSegmentsWidth(selectedSegments) < maxWidth
                  ? padSegmentsToWidth(
                      selectedSegments,
                      maxWidth,
                      selectionStyle,
                    )
                  : selectedSegments;
              return (
                <box flexDirection="row">
                  {displaySegments.map((segment) => (
                    <text
                      {...(segment.fg ? { fg: segment.fg } : {})}
                      {...(segment.bg ? { bg: segment.bg } : {})}
                      {...(segment.attributes !== undefined
                        ? { attributes: segment.attributes }
                        : {})}
                    >
                      {segment.text}
                    </text>
                  ))}
                </box>
              );
            })}
          </box>
        )}

        {merged.error ? <text fg="red">{merged.error}</text> : null}
      </box>

      <Footer actions={footerActions()} />
    </box>
  );
}
