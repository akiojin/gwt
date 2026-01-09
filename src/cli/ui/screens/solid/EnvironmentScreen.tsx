/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { createMemo } from "solid-js";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";
import { useScrollableList } from "../../hooks/solid/useScrollableList.js";
import stringWidth from "string-width";
import { selectionStyle } from "../../core/theme.js";

export interface EnvironmentVariable {
  key: string;
  value: string;
}

export interface EnvironmentScreenProps {
  variables: EnvironmentVariable[];
  onSelect?: (variable: EnvironmentVariable) => void;
  onBack?: () => void;
  version?: string | null;
  highlightKeys?: string[];
  helpVisible?: boolean;
}

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

export function EnvironmentScreen({
  variables,
  onSelect,
  onBack,
  version,
  highlightKeys,
  helpVisible = false,
}: EnvironmentScreenProps) {
  const terminal = useTerminalSize();
  const padLine = (value: string, width: number) => {
    const padding = Math.max(0, width - stringWidth(value));
    return padding > 0 ? `${value}${" ".repeat(padding)}` : value;
  };
  const listHeight = createMemo(() => {
    const headerRows = 2;
    const footerRows = 1;
    const reserved = headerRows + footerRows;
    return Math.max(1, terminal().rows - reserved);
  });

  const highlightSet = createMemo(() => new Set(highlightKeys ?? []));

  const list = useScrollableList({
    items: () => variables,
    visibleCount: listHeight,
    initialIndex: 0,
  });

  const updateSelectedIndex = (value: number | ((prev: number) => number)) => {
    list.setSelectedIndex(value);
  };

  useKeyboard((key) => {
    if (helpVisible) {
      return;
    }

    if (key.name === "escape" || key.name === "q") {
      onBack?.();
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
      updateSelectedIndex(variables.length - 1);
      return;
    }

    if (key.name === "return" || key.name === "linefeed") {
      const item =
        variables[clamp(list.selectedIndex(), 0, variables.length - 1)];
      if (item) {
        onSelect?.(item);
      }
    }
  });

  const footerActions = createMemo(() => {
    const actions = [] as { key: string; description: string }[];
    if (onSelect) {
      actions.push({ key: "enter", description: "Select" });
    }
    if (onBack) {
      actions.push({ key: "esc", description: "Back" });
    }
    return actions;
  });

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header title="gwt - Environment" titleColor="cyan" version={version} />

      <box flexDirection="column" flexGrow={1}>
        {variables.length === 0 ? (
          <text fg="gray">No environment variables.</text>
        ) : (
          <box flexDirection="column">
            {list.visibleItems().map((variable, index) => {
              const absoluteIndex = list.scrollOffset() + index;
              const isSelected = absoluteIndex === list.selectedIndex();
              const isHighlighted = highlightSet().has(variable.key);
              const keyColor = isHighlighted ? "yellow" : undefined;
              return (
                <box flexDirection="row">
                  {isSelected ? (
                    <text fg={selectionStyle.fg} bg={selectionStyle.bg}>
                      {padLine(
                        `${variable.key}=${variable.value}`,
                        terminal().columns,
                      )}
                    </text>
                  ) : (
                    <>
                      <text {...(keyColor ? { fg: keyColor } : {})}>
                        {variable.key}
                      </text>
                      <text>={variable.value}</text>
                    </>
                  )}
                </box>
              );
            })}
          </box>
        )}
      </box>

      <Footer actions={footerActions()} />
    </box>
  );
}
