/** @jsxImportSource @opentui/solid */
import { createSignal, createEffect, createMemo, Show } from "solid-js";
import { useKeyboard } from "@opentui/solid";
import type { ToolSessionEntry } from "../../../../config/index.js";
import type { CodingAgentId, InferenceLevel } from "../../types.js";
import { WizardPopup } from "./WizardPopup.js";
import { QuickStartStep } from "./QuickStartStep.js";
import {
  ActionSelectStep,
  type BranchAction,
  BranchTypeStep,
  BranchNameStep,
  AgentSelectStep,
  VersionSelectStep,
  ModelSelectStep,
  ReasoningLevelStep,
  ExecutionModeStep,
  SkipPermissionsStep,
} from "./WizardSteps.js";

export type ExecutionMode = "normal" | "continue" | "resume";

export interface WizardResult {
  tool: CodingAgentId;
  model: string;
  reasoningLevel?: InferenceLevel;
  mode: ExecutionMode;
  skipPermissions: boolean;
  // For new branch creation
  branchType?: string;
  branchName?: string;
  // For action selection
  isNewBranch?: boolean;
  // For version selection
  toolVersion?: string | null;
}

export interface WizardControllerProps {
  visible: boolean;
  selectedBranchName: string;
  history: ToolSessionEntry[];
  onClose: () => void;
  onComplete: (result: WizardResult) => void;
  onResume: (entry: ToolSessionEntry) => void;
  onStartNew: (entry: ToolSessionEntry) => void;
}

type WizardStep =
  | "action-select"
  | "quick-start"
  | "branch-type"
  | "branch-name"
  | "agent-select"
  | "version-select"
  | "model-select"
  | "reasoning-level"
  | "execution-mode"
  | "skip-permissions";

/**
 * WizardController - ウィザードフロー全体を管理するコントローラー
 *
 * FR-047: ステップを同一ポップアップ内で切り替え
 * FR-050: 履歴がある場合はクイック選択を表示
 * FR-051: 「Choose different settings...」で設定選択フローへ
 * FR-056: Codex CLI 選択時のみ推論レベル選択を表示
 * FR-059: Escape キーによる前ステップへの戻り
 */
export function WizardController(props: WizardControllerProps) {
  const [step, setStep] = createSignal<WizardStep>(getInitialStep());
  const [stepHistory, setStepHistory] = createSignal<WizardStep[]>([]);

  // Wizard state
  const [isCreatingNewBranch, setIsCreatingNewBranch] =
    createSignal<boolean>(false);
  const [branchType, setBranchType] = createSignal<string>("");
  const [branchName, setBranchName] = createSignal<string>("");
  const [selectedAgent, setSelectedAgent] = createSignal<CodingAgentId | null>(
    null,
  );
  const [selectedVersion, setSelectedVersion] = createSignal<string | null>(
    null,
  );
  const [selectedModel, setSelectedModel] = createSignal<string>("");
  const [reasoningLevel, setReasoningLevel] = createSignal<
    InferenceLevel | undefined
  >(undefined);
  const [executionMode, setExecutionMode] =
    createSignal<ExecutionMode>("normal");

  // キー伝播防止: 最初のフレームでは focused を無効にする
  const [isInitialFrame, setIsInitialFrame] = createSignal(true);

  // Reset state when wizard becomes visible
  function getInitialStep(): WizardStep {
    // 履歴がある場合はクイック選択を表示
    if (props.history.length > 0) {
      return "quick-start";
    }
    // 履歴がない場合はアクション選択から開始
    return "action-select";
  }

  // Watch for visibility changes to reset state
  const resetWizard = () => {
    setStep(getInitialStep());
    setStepHistory([]);
    setIsCreatingNewBranch(false);
    setBranchType("");
    setBranchName("");
    setSelectedAgent(null);
    setSelectedVersion(null);
    setSelectedModel("");
    setReasoningLevel(undefined);
    setExecutionMode("normal");
  };

  // Reset wizard when it becomes visible
  let prevVisible = false;
  createEffect(() => {
    const visible = props.visible;
    if (visible && !prevVisible) {
      resetWizard();
      // キー伝播防止: 最初の数フレームでは focused を無効にする
      // Enter キーが複数フレームにわたって伝播する可能性があるため、長めに設定
      setIsInitialFrame(true);
      // 十分な時間を置いて focused を有効化 (150ms)
      setTimeout(() => setIsInitialFrame(false), 150);
    }
    prevVisible = visible;
  });

  // Handle keyboard events for step navigation
  // T412: 最初のフレームでは Enter キーをブロックして伝播を防ぐ
  useKeyboard((key) => {
    if (!props.visible) return;
    // 最初のフレームでは Enter キーを無視（ブランチ選択からの伝播防止）
    if (isInitialFrame() && key.name === "return") {
      return;
    }
    if (key.name === "escape") {
      goBack();
    }
  });

  const goToStep = (nextStep: WizardStep) => {
    setStepHistory((prev) => [...prev, step()]);
    setStep(nextStep);
  };

  const goBack = () => {
    const history = stepHistory();
    if (history.length === 0) {
      props.onClose();
      return;
    }
    const previousStep = history[history.length - 1] ?? "agent-select";
    setStepHistory(history.slice(0, -1));
    setStep(previousStep);
  };

  // Determine if reasoning level step is needed
  const needsReasoningLevel = createMemo(() => {
    return selectedAgent() === "codex-cli";
  });

  // Step handlers
  const handleActionSelect = (action: BranchAction) => {
    if (action === "create-new") {
      setIsCreatingNewBranch(true);
      goToStep("branch-type");
    } else {
      setIsCreatingNewBranch(false);
      goToStep("agent-select");
    }
  };

  const handleQuickStartResume = (entry: ToolSessionEntry) => {
    props.onResume(entry);
  };

  const handleQuickStartNew = (entry: ToolSessionEntry) => {
    props.onStartNew(entry);
  };

  const handleChooseDifferent = () => {
    // クイック選択から「別の設定を選択」の場合はアクション選択へ戻る
    goToStep("action-select");
  };

  const handleBranchTypeSelect = (type: string) => {
    setBranchType(type);
    goToStep("branch-name");
  };

  const handleBranchNameSubmit = (name: string) => {
    setBranchName(name);
    goToStep("agent-select");
  };

  const handleAgentSelect = (agentId: string) => {
    setSelectedAgent(agentId as CodingAgentId);
    goToStep("version-select");
  };

  const handleVersionSelect = (version: string) => {
    // "installed" を明示的に保存し、未指定時は後方互換で "latest" にフォールバックできるようにする
    setSelectedVersion(version);
    goToStep("model-select");
  };

  const handleModelSelect = (model: string) => {
    setSelectedModel(model);
    if (needsReasoningLevel()) {
      goToStep("reasoning-level");
    } else {
      goToStep("execution-mode");
    }
  };

  const handleReasoningLevelSelect = (level: string) => {
    setReasoningLevel(level as InferenceLevel);
    goToStep("execution-mode");
  };

  const handleExecutionModeSelect = (mode: string) => {
    setExecutionMode(mode as ExecutionMode);
    goToStep("skip-permissions");
  };

  const handleSkipPermissionsSelect = (skip: boolean) => {
    const agent = selectedAgent();
    if (!agent) return;

    const currentReasoningLevel = reasoningLevel();
    const currentBranchType = branchType();
    const currentBranchName = branchName();
    const creatingNew = isCreatingNewBranch();
    const currentVersion = selectedVersion();

    const result: WizardResult = {
      tool: agent,
      model: selectedModel(),
      mode: executionMode(),
      skipPermissions: skip,
      isNewBranch: creatingNew,
      toolVersion: currentVersion,
      ...(needsReasoningLevel() && currentReasoningLevel !== undefined
        ? { reasoningLevel: currentReasoningLevel }
        : {}),
      ...(creatingNew
        ? { branchType: currentBranchType, branchName: currentBranchName }
        : {}),
    };

    props.onComplete(result);
  };

  const renderStep = () => {
    const currentStep = step();
    const focused = !isInitialFrame();

    if (currentStep === "action-select") {
      return (
        <ActionSelectStep
          branchName={props.selectedBranchName}
          onSelect={handleActionSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "quick-start") {
      return (
        <QuickStartStep
          history={props.history}
          onResume={handleQuickStartResume}
          onStartNew={handleQuickStartNew}
          onChooseDifferent={handleChooseDifferent}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "branch-type") {
      return (
        <BranchTypeStep
          onSelect={handleBranchTypeSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "branch-name") {
      return (
        <BranchNameStep
          branchType={branchType()}
          onSubmit={handleBranchNameSubmit}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "agent-select") {
      return (
        <AgentSelectStep
          onSelect={handleAgentSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "version-select") {
      return (
        <VersionSelectStep
          agentId={selectedAgent() ?? "claude-code"}
          onSelect={handleVersionSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "model-select") {
      return (
        <ModelSelectStep
          agentId={selectedAgent() ?? "claude-code"}
          onSelect={handleModelSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "reasoning-level") {
      return (
        <ReasoningLevelStep
          onSelect={handleReasoningLevelSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "execution-mode") {
      return (
        <ExecutionModeStep
          onSelect={handleExecutionModeSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    if (currentStep === "skip-permissions") {
      return (
        <SkipPermissionsStep
          onSelect={handleSkipPermissionsSelect}
          onBack={goBack}
          focused={focused}
        />
      );
    }

    return null;
  };

  return (
    <Show when={props.visible}>
      <WizardPopup
        visible={props.visible}
        onClose={props.onClose}
        onComplete={() => {}}
      >
        {renderStep()}
      </WizardPopup>
    </Show>
  );
}
