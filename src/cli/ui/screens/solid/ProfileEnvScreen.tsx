/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";
import { useKeyboard } from "@opentui/solid";
import { createMemo } from "solid-js";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";
import { useScrollableList } from "../../hooks/solid/useScrollableList.js";

export interface ProfileEnvVariable {
  key: string;
  value: string;
}

export interface ProfileEnvScreenProps {
  profileName: string;
  variables: ProfileEnvVariable[];
  onAdd?: () => void;
  onEdit?: (variable: ProfileEnvVariable) => void;
  onDelete?: (variable: ProfileEnvVariable) => void;
  onViewOsEnv?: () => void;
  onBack?: () => void;
  version?: string | null;
  helpVisible?: boolean;
}

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

export function ProfileEnvScreen({
  profileName,
  variables,
  onAdd,
  onEdit,
  onDelete,
  onViewOsEnv,
  onBack,
  version,
  helpVisible = false,
}: ProfileEnvScreenProps) {
  const terminal = useTerminalSize();
  const listHeight = createMemo(() => {
    const headerRows = 2;
    const footerRows = 1;
    const reserved = headerRows + footerRows;
    return Math.max(1, terminal().rows - reserved);
  });

  const list = useScrollableList({
    items: () => variables,
    visibleCount: listHeight,
    initialIndex: 0,
  });

  const updateSelectedIndex = (value: number | ((prev: number) => number)) => {
    list.setSelectedIndex(value);
  };

  const getSelectedVariable = () => {
    const index = clamp(list.selectedIndex(), 0, variables.length - 1);
    return variables[index];
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

    if (key.name === "a") {
      onAdd?.();
      return;
    }

    if (key.name === "e" || key.name === "return" || key.name === "linefeed") {
      const variable = getSelectedVariable();
      if (variable) {
        onEdit?.(variable);
      }
      return;
    }

    if (key.name === "d") {
      const variable = getSelectedVariable();
      if (variable) {
        onDelete?.(variable);
      }
      return;
    }

    if (key.name === "o") {
      onViewOsEnv?.();
    }
  });

  const footerActions = createMemo(() => {
    const actions = [] as { key: string; description: string }[];
    if (onAdd) {
      actions.push({ key: "a", description: "Add" });
    }
    if (onEdit) {
      actions.push({ key: "enter", description: "Edit" });
    }
    if (onDelete) {
      actions.push({ key: "d", description: "Delete" });
    }
    if (onViewOsEnv) {
      actions.push({ key: "o", description: "OS Env" });
    }
    if (onBack) {
      actions.push({ key: "esc", description: "Back" });
    }
    return actions;
  });

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header
        title={`gwt - Profile: ${profileName}`}
        titleColor="cyan"
        version={version}
      />

      <box flexDirection="column" flexGrow={1}>
        {variables.length === 0 ? (
          <text fg="gray">No variables in profile.</text>
        ) : (
          <box flexDirection="column">
            {list.visibleItems().map((variable, index) => {
              const absoluteIndex = list.scrollOffset() + index;
              const isSelected = absoluteIndex === list.selectedIndex();
              const attributes = isSelected ? TextAttributes.BOLD : undefined;
              const color = isSelected ? "cyan" : undefined;
              return (
                <box flexDirection="row">
                  <text
                    {...(color ? { fg: color } : {})}
                    {...(attributes !== undefined ? { attributes } : {})}
                  >
                    {variable.key}
                  </text>
                  <text
                    {...(color ? { fg: color } : {})}
                    {...(attributes !== undefined ? { attributes } : {})}
                  >
                    ={variable.value}
                  </text>
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
