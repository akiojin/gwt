/** @jsxImportSource @opentui/solid */
import { Header } from "../../components/solid/Header.js";
import { ConfirmScreen } from "./ConfirmScreen.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";

export interface WorktreeDeleteScreenProps {
  branchName: string;
  worktreePath?: string | null;
  onConfirm: (confirmed: boolean) => void;
  version?: string | null;
  helpVisible?: boolean;
}

export function WorktreeDeleteScreen({
  branchName,
  worktreePath,
  onConfirm,
  version,
  helpVisible = false,
}: WorktreeDeleteScreenProps) {
  const terminal = useTerminalSize();
  const message = `Delete worktree for ${branchName}?`;

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header
        title="gwt - Worktree Delete"
        titleColor="cyan"
        version={version}
      />
      {worktreePath && <text fg="gray">{worktreePath}</text>}
      <ConfirmScreen
        message={message}
        onConfirm={onConfirm}
        defaultNo
        helpVisible={helpVisible}
      />
    </box>
  );
}
