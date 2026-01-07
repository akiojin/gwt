/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";
import { useKeyboard } from "@opentui/solid";
import { createEffect, createSignal } from "solid-js";
import { SelectInput, type SelectInputItem } from "./SelectInput.js";
import { TextInput } from "./TextInput.js";
import { getModelOptions } from "../../utils/modelOptions.js";
import type { CodingAgentId } from "../../types.js";
import { useWizardScroll } from "./WizardPopup.js";

/**
 * WizardSteps - ウィザードの各ステップコンポーネント
 *
 * OpenTUI の SelectInput/TextInput を活用した実装
 */

export interface StepProps {
  onBack: () => void;
  focused?: boolean;
}

const useEnsureSelectionVisible = (options: {
  getSelectedIndex: () => number;
  getItemCount: () => number;
  baseLine: number;
  linesPerItem?: number;
  getFocused?: () => boolean | undefined;
}) => {
  const scroll = useWizardScroll();
  createEffect(() => {
    if (options.getFocused && options.getFocused() === false) {
      return;
    }
    if (!scroll) {
      return;
    }
    const itemCount = options.getItemCount();
    if (itemCount <= 0) {
      return;
    }
    const safeIndex = Math.min(
      Math.max(options.getSelectedIndex(), 0),
      itemCount - 1,
    );
    const linesPerItem = options.linesPerItem ?? 1;
    const startLine = options.baseLine + safeIndex * linesPerItem;
    const endLine = startLine + Math.max(0, linesPerItem - 1);
    scroll.ensureLineVisible(startLine);
    if (endLine !== startLine) {
      scroll.ensureLineVisible(endLine);
    }
  });
};

// アクション選択ステップ（既存を開く / 新規作成）
export type BranchAction = "open-existing" | "create-new";

export interface ActionSelectStepProps extends StepProps {
  branchName: string;
  onSelect: (action: BranchAction) => void;
}

const ACTION_OPTIONS: SelectInputItem[] = [
  {
    label: "Open existing worktree",
    value: "open-existing",
    description: "Open the worktree for this branch",
  },
  {
    label: "Create new branch",
    value: "create-new",
    description: "Create a new branch based on this one",
  },
];

export function ActionSelectStep(props: ActionSelectStepProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  useEnsureSelectionVisible({
    getSelectedIndex: selectedIndex,
    getItemCount: () => ACTION_OPTIONS.length,
    getFocused: () => props.focused,
    baseLine: 4,
  });

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = ACTION_OPTIONS.findIndex(
      (candidate) => candidate.value === item.value,
    );
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
    }
  };

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Branch: {props.branchName}
      </text>
      <text> </text>
      <text>What would you like to do?</text>
      <text> </text>
      <SelectInput
        items={ACTION_OPTIONS}
        onSelect={(item) => props.onSelect(item.value as BranchAction)}
        onChange={handleChange}
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Select [Esc] Cancel</text>
    </box>
  );
}

// T405: ブランチタイプ選択ステップ
export interface BranchTypeStepProps extends StepProps {
  onSelect: (type: string) => void;
}

const BRANCH_TYPES: SelectInputItem[] = [
  { label: "feature/", value: "feature/", description: "New feature branch" },
  { label: "bugfix/", value: "bugfix/", description: "Bug fix branch" },
  { label: "hotfix/", value: "hotfix/", description: "Hotfix branch" },
  { label: "release/", value: "release/", description: "Release branch" },
];

export function BranchTypeStep(props: BranchTypeStepProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  useEnsureSelectionVisible({
    getSelectedIndex: selectedIndex,
    getItemCount: () => BRANCH_TYPES.length,
    getFocused: () => props.focused,
    baseLine: 2,
  });

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = BRANCH_TYPES.findIndex(
      (candidate) => candidate.value === item.value,
    );
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
    }
  };

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Select branch type:
      </text>
      <text> </text>
      <SelectInput
        items={BRANCH_TYPES}
        onSelect={(item) => props.onSelect(item.value)}
        onChange={handleChange}
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Select [Esc] Back</text>
    </box>
  );
}

// T406: ブランチ名入力ステップ
export interface BranchNameStepProps extends StepProps {
  branchType: string;
  onSubmit: (name: string) => void;
}

export function BranchNameStep(props: BranchNameStepProps) {
  const [name, setName] = createSignal("");
  const scroll = useWizardScroll();

  createEffect(() => {
    if (props.focused === false) {
      return;
    }
    if (!scroll) {
      return;
    }
    scroll.ensureLineVisible(2);
  });

  useKeyboard((key) => {
    if (props.focused === false) {
      return;
    }
    if (!scroll) {
      return;
    }
    if (key.name === "up") {
      if (scroll.scrollByLines(-1)) {
        key.preventDefault();
      }
      return;
    }
    if (key.name === "down") {
      if (scroll.scrollByLines(1)) {
        key.preventDefault();
      }
    }
  });

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Branch name: {props.branchType}
      </text>
      <text> </text>
      <TextInput
        value={name()}
        onChange={setName}
        onSubmit={(value) => props.onSubmit(value)}
        placeholder="Enter branch name..."
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Confirm [Esc] Back</text>
    </box>
  );
}

// T407: コーディングエージェント選択ステップ
export interface AgentSelectStepProps extends StepProps {
  onSelect: (agentId: string) => void;
}

const AGENTS: SelectInputItem[] = [
  {
    label: "Claude Code",
    value: "claude-code",
    description: "Anthropic Claude Code",
  },
  { label: "Codex CLI", value: "codex-cli", description: "OpenAI Codex CLI" },
  {
    label: "Gemini CLI",
    value: "gemini-cli",
    description: "Google Gemini CLI",
  },
  {
    label: "OpenCode",
    value: "opencode",
    description: "OpenCode AI",
  },
];

export function AgentSelectStep(props: AgentSelectStepProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  useEnsureSelectionVisible({
    getSelectedIndex: selectedIndex,
    getItemCount: () => AGENTS.length,
    getFocused: () => props.focused,
    baseLine: 2,
  });

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = AGENTS.findIndex(
      (candidate) => candidate.value === item.value,
    );
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
    }
  };

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Select coding agent:
      </text>
      <text> </text>
      <SelectInput
        items={AGENTS}
        onSelect={(item) => props.onSelect(item.value)}
        onChange={handleChange}
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Select [Esc] Back</text>
    </box>
  );
}

// T408: モデル選択ステップ
export interface ModelSelectStepProps extends StepProps {
  agentId: CodingAgentId;
  onSelect: (model: string) => void;
}

function getModelItems(agentId: CodingAgentId): SelectInputItem[] {
  const options = getModelOptions(agentId);
  return options.map((opt) => {
    const item: SelectInputItem = {
      label: opt.label,
      value: opt.id,
    };
    if (opt.description !== undefined) {
      item.description = opt.description;
    }
    return item;
  });
}

export function ModelSelectStep(props: ModelSelectStepProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const models = () => getModelItems(props.agentId);
  useEnsureSelectionVisible({
    getSelectedIndex: selectedIndex,
    getItemCount: () => models().length,
    getFocused: () => props.focused,
    baseLine: 2,
  });

  createEffect(() => {
    const count = models().length;
    if (count <= 0) {
      setSelectedIndex(0);
      return;
    }
    const maxIndex = count - 1;
    if (selectedIndex() > maxIndex) {
      setSelectedIndex(maxIndex);
    }
  });

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = models().findIndex(
      (candidate) => candidate.value === item.value,
    );
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
    }
  };

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Select Model:
      </text>
      <text> </text>
      <SelectInput
        items={models()}
        onSelect={(item) => props.onSelect(item.value)}
        onChange={handleChange}
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Select [Esc] Back</text>
    </box>
  );
}

// T409: 推論レベル選択ステップ（Codexのみ）
export interface ReasoningLevelStepProps extends StepProps {
  onSelect: (level: string) => void;
}

const REASONING_LEVELS: SelectInputItem[] = [
  { label: "low", value: "low", description: "Faster, less thorough" },
  { label: "medium", value: "medium", description: "Balanced" },
  { label: "high", value: "high", description: "Slower, more thorough" },
  { label: "xhigh", value: "xhigh", description: "Extended high reasoning" },
];

export function ReasoningLevelStep(props: ReasoningLevelStepProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  useEnsureSelectionVisible({
    getSelectedIndex: selectedIndex,
    getItemCount: () => REASONING_LEVELS.length,
    getFocused: () => props.focused,
    baseLine: 2,
  });

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = REASONING_LEVELS.findIndex(
      (candidate) => candidate.value === item.value,
    );
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
    }
  };

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Select reasoning level:
      </text>
      <text> </text>
      <SelectInput
        items={REASONING_LEVELS}
        onSelect={(item) => props.onSelect(item.value)}
        onChange={handleChange}
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Select [Esc] Back</text>
    </box>
  );
}

// T410: 実行モード選択ステップ
export interface ExecutionModeStepProps extends StepProps {
  onSelect: (mode: string) => void;
}

const EXECUTION_MODES: SelectInputItem[] = [
  { label: "Normal", value: "normal", description: "Start a new session" },
  {
    label: "Continue",
    value: "continue",
    description: "Continue from last session",
  },
  {
    label: "Resume",
    value: "resume",
    description: "Resume a specific session",
  },
];

export function ExecutionModeStep(props: ExecutionModeStepProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  useEnsureSelectionVisible({
    getSelectedIndex: selectedIndex,
    getItemCount: () => EXECUTION_MODES.length,
    getFocused: () => props.focused,
    baseLine: 2,
  });

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = EXECUTION_MODES.findIndex(
      (candidate) => candidate.value === item.value,
    );
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
    }
  };

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Select execution mode:
      </text>
      <text> </text>
      <SelectInput
        items={EXECUTION_MODES}
        onSelect={(item) => props.onSelect(item.value)}
        onChange={handleChange}
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Select [Esc] Back</text>
    </box>
  );
}

// T411: 権限スキップ確認ステップ
export interface SkipPermissionsStepProps extends StepProps {
  onSelect: (skip: boolean) => void;
}

const SKIP_OPTIONS: SelectInputItem[] = [
  { label: "Yes", value: "true", description: "Skip permission prompts" },
  { label: "No", value: "false", description: "Show permission prompts" },
];

export function SkipPermissionsStep(props: SkipPermissionsStepProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  useEnsureSelectionVisible({
    getSelectedIndex: selectedIndex,
    getItemCount: () => SKIP_OPTIONS.length,
    getFocused: () => props.focused,
    baseLine: 2,
  });

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = SKIP_OPTIONS.findIndex(
      (candidate) => candidate.value === item.value,
    );
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
    }
  };

  return (
    <box flexDirection="column">
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Skip permission prompts?
      </text>
      <text> </text>
      <SelectInput
        items={SKIP_OPTIONS}
        onSelect={(item) => props.onSelect(item.value === "true")}
        onChange={handleChange}
        focused={props.focused ?? true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>[Enter] Select [Esc] Back</text>
    </box>
  );
}
