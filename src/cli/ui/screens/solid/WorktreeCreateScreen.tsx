/** @jsxImportSource @opentui/solid */
import { createSignal } from "solid-js";
import { useKeyboard } from "@opentui/solid";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { TextInput } from "../../components/solid/TextInput.js";
import {
  SelectInput,
  type SelectInputItem,
} from "../../components/solid/SelectInput.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";

export type BranchTypeOption = "feature" | "bugfix" | "hotfix" | "release";

export interface WorktreeCreateScreenProps {
  branchName: string;
  onChange: (value: string) => void;
  onInput?: (value: string) => void;
  onSubmit: (value: string, branchType: BranchTypeOption) => void;
  onCancel?: () => void;
  baseBranch?: string;
  version?: string | null;
  helpVisible?: boolean;
}

const BRANCH_TYPE_OPTIONS: SelectInputItem[] = [
  { label: "feature", value: "feature", description: "New feature" },
  { label: "bugfix", value: "bugfix", description: "Bug fix" },
  { label: "hotfix", value: "hotfix", description: "Critical bug fix" },
  { label: "release", value: "release", description: "Release preparation" },
];

export function WorktreeCreateScreen({
  branchName,
  onChange,
  onInput,
  onSubmit,
  onCancel,
  baseBranch,
  version,
  helpVisible = false,
}: WorktreeCreateScreenProps) {
  const terminal = useTerminalSize();
  const inputHeight = 2;

  const [step, setStep] = createSignal<"type-selection" | "name-input">(
    "type-selection",
  );
  const [selectedType, setSelectedType] =
    createSignal<BranchTypeOption>("feature");

  useKeyboard((key) => {
    if (helpVisible) {
      return;
    }
    if (key.name === "escape") {
      if (step() === "name-input") {
        // Go back to type selection
        setStep("type-selection");
        onChange("");
      } else {
        // Cancel from type selection
        onCancel?.();
      }
    }
  });

  const handleTypeSelect = (item: SelectInputItem) => {
    setSelectedType(item.value as BranchTypeOption);
    setStep("name-input");
  };

  const handleNameSubmit = (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) {
      return;
    }
    onSubmit(trimmed, selectedType());
  };

  const footerActionsTypeSelection = [
    { key: "enter", description: "Select" },
    { key: "up/down", description: "Navigate" },
    ...(onCancel ? [{ key: "esc", description: "Cancel" }] : []),
  ];

  const footerActionsNameInput = [
    { key: "enter", description: "Create" },
    { key: "esc", description: "Back" },
  ];

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header
        title="gwt - Worktree Create"
        titleColor="cyan"
        version={version}
      />
      {baseBranch && <text fg="gray">{`Base: ${baseBranch}`}</text>}

      {step() === "type-selection" ? (
        <>
          <text fg="white">Select branch type:</text>
          <box height={4}>
            <SelectInput
              items={BRANCH_TYPE_OPTIONS}
              selectedIndex={BRANCH_TYPE_OPTIONS.findIndex(
                (item) => item.value === selectedType(),
              )}
              onSelect={handleTypeSelect}
              focused
              showDescription
            />
          </box>
          <Footer actions={footerActionsTypeSelection} />
        </>
      ) : (
        <>
          <text fg="gray">{`Type: ${selectedType()}/`}</text>
          <box height={inputHeight}>
            <TextInput
              label="Branch name"
              value={branchName}
              onChange={onChange}
              onInput={onInput ?? onChange}
              onSubmit={handleNameSubmit}
              focused
              width={32}
            />
          </box>
          <Footer actions={footerActionsNameInput} />
        </>
      )}
    </box>
  );
}
