import { useKeyboard } from "@opentui/solid";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { TextInput } from "../../components/solid/TextInput.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";

export interface WorktreeCreateScreenProps {
  branchName: string;
  onChange: (value: string) => void;
  onSubmit: (value: string) => void;
  onCancel?: () => void;
  baseBranch?: string;
  version?: string | null;
}

export function WorktreeCreateScreen({
  branchName,
  onChange,
  onSubmit,
  onCancel,
  baseBranch,
  version,
}: WorktreeCreateScreenProps) {
  const terminal = useTerminalSize();
  const inputHeight = 2;

  useKeyboard((key) => {
    if (key.name === "escape") {
      onCancel?.();
    }
  });

  const footerActions = [
    { key: "enter", description: "Create" },
    ...(onCancel ? [{ key: "esc", description: "Cancel" }] : []),
  ];

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header
        title="gwt - Worktree Create"
        titleColor="cyan"
        version={version}
      />
      {baseBranch && <text fg="gray">{`Base: ${baseBranch}`}</text>}

      <box height={inputHeight}>
        <TextInput
          label="Branch name"
          value={branchName}
          onChange={onChange}
          onSubmit={onSubmit}
          focused
        />
      </box>

      <Footer actions={footerActions} />
    </box>
  );
}
