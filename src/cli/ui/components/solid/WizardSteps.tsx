/** @jsxImportSource @opentui/solid */
import { createSignal } from "solid-js";
import { SelectInput, type SelectInputItem } from "./SelectInput.js";
import { TextInput } from "./TextInput.js";

/**
 * WizardSteps - ウィザードの各ステップコンポーネント
 *
 * OpenTUI の SelectInput/TextInput を活用した実装
 */

export interface StepProps {
  onBack: () => void;
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
  return (
    <box flexDirection="column">
      <text bold color="cyan">
        Select branch type:
      </text>
      <text> </text>
      <SelectInput
        items={BRANCH_TYPES}
        onSelect={(item) => props.onSelect(item.value)}
        focused={true}
      />
      <text> </text>
      <text dimColor>[Enter] Select [Esc] Back</text>
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

  return (
    <box flexDirection="column">
      <text bold color="cyan">
        Branch name: {props.branchType}
      </text>
      <text> </text>
      <TextInput
        value={name()}
        onChange={setName}
        onSubmit={(value) => props.onSubmit(value)}
        placeholder="Enter branch name..."
        focused={true}
      />
      <text> </text>
      <text dimColor>[Enter] Confirm [Esc] Back</text>
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
];

export function AgentSelectStep(props: AgentSelectStepProps) {
  return (
    <box flexDirection="column">
      <text bold color="cyan">
        Select coding agent:
      </text>
      <text> </text>
      <SelectInput
        items={AGENTS}
        onSelect={(item) => props.onSelect(item.value)}
        focused={true}
      />
      <text> </text>
      <text dimColor>[Enter] Select [Esc] Back</text>
    </box>
  );
}

// T408: モデル選択ステップ
export interface ModelSelectStepProps extends StepProps {
  agentId: string;
  onSelect: (model: string) => void;
}

const MODELS_BY_AGENT: Record<string, SelectInputItem[]> = {
  "claude-code": [
    { label: "claude-sonnet-4-20250514", value: "claude-sonnet-4-20250514" },
    { label: "claude-opus-4-20250514", value: "claude-opus-4-20250514" },
  ],
  "codex-cli": [
    { label: "o3-mini", value: "o3-mini" },
    { label: "o1", value: "o1" },
    { label: "gpt-4.1", value: "gpt-4.1" },
  ],
  "gemini-cli": [
    { label: "gemini-2.5-pro", value: "gemini-2.5-pro" },
    { label: "gemini-2.5-flash", value: "gemini-2.5-flash" },
  ],
};

export function ModelSelectStep(props: ModelSelectStepProps) {
  const models = () => MODELS_BY_AGENT[props.agentId] ?? [];

  return (
    <box flexDirection="column">
      <text bold color="cyan">
        Select Model:
      </text>
      <text> </text>
      <SelectInput
        items={models()}
        onSelect={(item) => props.onSelect(item.value)}
        focused={true}
      />
      <text> </text>
      <text dimColor>[Enter] Select [Esc] Back</text>
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
];

export function ReasoningLevelStep(props: ReasoningLevelStepProps) {
  return (
    <box flexDirection="column">
      <text bold color="cyan">
        Select reasoning level:
      </text>
      <text> </text>
      <SelectInput
        items={REASONING_LEVELS}
        onSelect={(item) => props.onSelect(item.value)}
        focused={true}
      />
      <text> </text>
      <text dimColor>[Enter] Select [Esc] Back</text>
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
  return (
    <box flexDirection="column">
      <text bold color="cyan">
        Select execution mode:
      </text>
      <text> </text>
      <SelectInput
        items={EXECUTION_MODES}
        onSelect={(item) => props.onSelect(item.value)}
        focused={true}
      />
      <text> </text>
      <text dimColor>[Enter] Select [Esc] Back</text>
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
  return (
    <box flexDirection="column">
      <text bold color="cyan">
        Skip permission prompts?
      </text>
      <text> </text>
      <SelectInput
        items={SKIP_OPTIONS}
        onSelect={(item) => props.onSelect(item.value === "true")}
        focused={true}
      />
      <text> </text>
      <text dimColor>[Enter] Select [Esc] Back</text>
    </box>
  );
}
