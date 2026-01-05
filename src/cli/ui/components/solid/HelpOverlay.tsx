/** @jsxImportSource @opentui/solid */
import { useTerminalDimensions } from "@opentui/solid";
import { createMemo } from "solid-js";
import stringWidth from "string-width";
import {
  formatKeyCombination,
  getKeybindingsForAction,
  getKeybindingsForContext,
  type KeybindingDef,
} from "../../core/keybindings.js";

export interface HelpOverlayProps {
  visible: boolean;
  context?: string;
  title?: string;
  maxWidth?: number;
}

interface HelpItem {
  key: string;
  description: string;
}

const DEFAULT_TITLE = "Help";
const DEFAULT_MAX_WIDTH = 76;
const COLUMN_GAP = 2;
const MIN_COLUMN_WIDTH = 18;
const MIN_CONTENT_WIDTH = 10;
const ELLIPSIS = "...";

const measureWidth = (value: string): number => stringWidth(value);

const truncateToWidth = (value: string, width: number): string => {
  if (width <= 0) {
    return "";
  }
  if (measureWidth(value) <= width) {
    return value;
  }
  if (width <= ELLIPSIS.length) {
    return value.slice(0, width);
  }

  let currentWidth = 0;
  let result = "";
  for (const char of Array.from(value)) {
    const charWidth = measureWidth(char);
    if (currentWidth + charWidth > width - ELLIPSIS.length) {
      break;
    }
    result += char;
    currentWidth += charWidth;
  }
  return `${result}${ELLIPSIS}`;
};

const fitText = (value: string, width: number): string => {
  if (width <= 0) {
    return "";
  }
  const truncated = truncateToWidth(value, width);
  const padding = Math.max(0, width - measureWidth(truncated));
  return truncated + " ".repeat(padding);
};

const normalizeKeyLabel = (combo: KeybindingDef["key"]): string => {
  const label = formatKeyCombination(combo);
  return label
    .replaceAll("↑", "Up")
    .replaceAll("↓", "Down")
    .replaceAll("←", "Left")
    .replaceAll("→", "Right");
};

const buildHelpItems = (context?: string): HelpItem[] => {
  const items: HelpItem[] = [];
  const seen = new Set<string>();
  const targetContext = context ?? "branch-list";

  const appendBinding = (binding: KeybindingDef) => {
    const key = normalizeKeyLabel(binding.key);
    const description = binding.description;
    const id = `${key}|${description}`;
    if (seen.has(id)) {
      return;
    }
    seen.add(id);
    items.push({ key, description });
  };

  for (const binding of getKeybindingsForContext(targetContext)) {
    if (binding.action === "show-help") {
      continue;
    }
    appendBinding(binding);
  }

  for (const binding of getKeybindingsForAction("hide-help")) {
    appendBinding(binding);
  }

  return items;
};

export function HelpOverlay({
  visible,
  context,
  title = DEFAULT_TITLE,
  maxWidth = DEFAULT_MAX_WIDTH,
}: HelpOverlayProps) {
  const terminal = useTerminalDimensions();

  const layout = createMemo(() => {
    const width = terminal().width ?? 80;
    const height = terminal().height ?? 24;
    const overlayWidth = Math.min(Math.max(20, width - 2), maxWidth);
    const contentWidth = Math.max(MIN_CONTENT_WIDTH, overlayWidth - 4);
    const maxRows = Math.max(4, height - 6);

    const items = buildHelpItems(context);
    if (items.length === 0) {
      return {
        overlayWidth,
        rows: [fitText("No keybindings available.", contentWidth)],
      };
    }

    const keyWidth = Math.min(
      16,
      Math.max(4, ...items.map((item) => measureWidth(item.key))),
    );

    const lines = items.map(
      (item) => `${fitText(item.key, keyWidth)} ${item.description}`,
    );

    const shouldUseTwoColumns =
      lines.length > maxRows &&
      contentWidth >= MIN_COLUMN_WIDTH * 2 + COLUMN_GAP;
    const columnCount = shouldUseTwoColumns ? 2 : 1;
    const columnGap = columnCount === 2 ? COLUMN_GAP : 0;
    const columnWidth = Math.floor((contentWidth - columnGap) / columnCount);
    const rowsPerColumn = Math.ceil(lines.length / columnCount);

    const rows: string[] = [];
    for (let row = 0; row < rowsPerColumn; row += 1) {
      const left = lines[row] ?? "";
      const right = columnCount === 2 ? (lines[row + rowsPerColumn] ?? "") : "";
      const leftCell = fitText(left, columnWidth);
      const rightCell = columnCount === 2 ? fitText(right, columnWidth) : "";
      const rowText =
        columnCount === 2
          ? `${leftCell}${" ".repeat(columnGap)}${rightCell}`
          : leftCell;
      rows.push(rowText);
    }

    if (rows.length > maxRows) {
      rows.splice(maxRows);
      rows[maxRows - 1] = fitText("... more", columnWidth);
    }

    return { overlayWidth, rows };
  });

  if (!visible) {
    return null;
  }

  return (
    <box
      position="absolute"
      top={0}
      left={0}
      width="100%"
      height="100%"
      justifyContent="center"
      alignItems="center"
      zIndex={100}
    >
      <box
        border
        borderStyle="single"
        borderColor="cyan"
        padding={1}
        width={layout().overlayWidth}
        title={title}
        titleAlignment="center"
      >
        <box flexDirection="column">
          {layout().rows.map((row) => (
            <text>{row}</text>
          ))}
        </box>
      </box>
    </box>
  );
}
