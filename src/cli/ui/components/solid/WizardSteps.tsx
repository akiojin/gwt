/** @jsxImportSource @opentui/solid */

/**
 * WizardSteps - ウィザードの各ステップコンポーネント
 *
 * TODO: 実装予定（TDD RED状態）
 */

export interface StepProps {
  onBack: () => void;
}

// T405: ブランチタイプ選択ステップ
export interface BranchTypeStepProps extends StepProps {
  onSelect: (type: string) => void;
}

export function BranchTypeStep(_props: BranchTypeStepProps) {
  return (
    <box flexDirection="column">
      <text>Select branch type:</text>
      <text>feature/</text>
      <text>bugfix/</text>
      <text>hotfix/</text>
      <text>release/</text>
    </box>
  );
}

// T406: ブランチ名入力ステップ
export interface BranchNameStepProps extends StepProps {
  branchType: string;
  onSubmit: (name: string) => void;
}

export function BranchNameStep(_props: BranchNameStepProps) {
  return (
    <box flexDirection="column">
      <text>Branch name: {_props.branchType}</text>
      <text>[Input field]</text>
    </box>
  );
}

// T407: コーディングエージェント選択ステップ
export interface AgentSelectStepProps extends StepProps {
  onSelect: (agentId: string) => void;
}

export function AgentSelectStep(_props: AgentSelectStepProps) {
  return (
    <box flexDirection="column">
      <text>Select coding agent:</text>
      <text>Claude Code</text>
      <text>Codex CLI</text>
      <text>Gemini CLI</text>
    </box>
  );
}

// T408: モデル選択ステップ
export interface ModelSelectStepProps extends StepProps {
  agentId: string;
  onSelect: (model: string) => void;
}

export function ModelSelectStep(_props: ModelSelectStepProps) {
  return (
    <box flexDirection="column">
      <text>Select Model:</text>
      <text>[Model list for {_props.agentId}]</text>
    </box>
  );
}

// T409: 推論レベル選択ステップ（Codexのみ）
export interface ReasoningLevelStepProps extends StepProps {
  onSelect: (level: string) => void;
}

export function ReasoningLevelStep(_props: ReasoningLevelStepProps) {
  return (
    <box flexDirection="column">
      <text>Select reasoning level:</text>
      <text>low</text>
      <text>medium</text>
      <text>high</text>
    </box>
  );
}

// T410: 実行モード選択ステップ
export interface ExecutionModeStepProps extends StepProps {
  onSelect: (mode: string) => void;
}

export function ExecutionModeStep(_props: ExecutionModeStepProps) {
  return (
    <box flexDirection="column">
      <text>Select execution mode:</text>
      <text>Normal</text>
      <text>Continue</text>
      <text>Resume</text>
    </box>
  );
}

// T411: 権限スキップ確認ステップ
export interface SkipPermissionsStepProps extends StepProps {
  onSelect: (skip: boolean) => void;
}

export function SkipPermissionsStep(_props: SkipPermissionsStepProps) {
  return (
    <box flexDirection="column">
      <text>Skip permission prompts?</text>
      <text>Yes</text>
      <text>No</text>
    </box>
  );
}
