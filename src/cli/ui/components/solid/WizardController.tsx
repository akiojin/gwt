/** @jsxImportSource @opentui/solid */
import { createSignal, createMemo, Show } from "solid-js";
import { useKeyboard } from "@opentui/solid";
import type { ToolSessionEntry } from "../../../../config/index.js";
import type { CodingAgentId, InferenceLevel } from "../../types.js";
import { WizardPopup } from "./WizardPopup.js";
import { QuickStartStep } from "./QuickStartStep.js";
import {
  BranchTypeStep,
  BranchNameStep,
  AgentSelectStep,
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
}

export interface WizardControllerProps {
  visible: boolean;
  isNewBranch: boolean;
  history: ToolSessionEntry[];
  onClose: () => void;
  onComplete: (result: WizardResult) => void;
  onResume: (entry: ToolSessionEntry) => void;
  onStartNew: (entry: ToolSessionEntry) => void;
}

type WizardStep =
  | "quick-start"
  | "branch-type"
  | "branch-name"
  | "agent-select"
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
  const [branchType, setBranchType] = createSignal<string>("");
  const [branchName, setBranchName] = createSignal<string>("");
  const [selectedAgent, setSelectedAgent] = createSignal<CodingAgentId | null>(
    null,
  );
  const [selectedModel, setSelectedModel] = createSignal<string>("");
  const [reasoningLevel, setReasoningLevel] = createSignal<
    InferenceLevel | undefined
  >(undefined);
  const [executionMode, setExecutionMode] =
    createSignal<ExecutionMode>("normal");

  // Reset state when wizard becomes visible
  function getInitialStep(): WizardStep {
    if (props.isNewBranch) {
      return "branch-type";
    }
    if (props.history.length > 0) {
      return "quick-start";
    }
    return "agent-select";
  }

  // Watch for visibility changes to reset state
  const resetWizard = () => {
    setStep(getInitialStep());
    setStepHistory([]);
    setBranchType("");
    setBranchName("");
    setSelectedAgent(null);
    setSelectedModel("");
    setReasoningLevel(undefined);
    setExecutionMode("normal");
  };

  // Reset wizard when it becomes visible
  let prevVisible = false;
  createMemo(() => {
    const visible = props.visible;
    if (visible && !prevVisible) {
      resetWizard();
    }
    prevVisible = visible;
  });

  // Handle keyboard events for step navigation
  useKeyboard((key) => {
    if (!props.visible) return;
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
  const handleQuickStartResume = (entry: ToolSessionEntry) => {
    props.onResume(entry);
  };

  const handleQuickStartNew = (entry: ToolSessionEntry) => {
    props.onStartNew(entry);
  };

  const handleChooseDifferent = () => {
    if (props.isNewBranch) {
      goToStep("branch-type");
    } else {
      goToStep("agent-select");
    }
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

    const result: WizardResult = {
      tool: agent,
      model: selectedModel(),
      mode: executionMode(),
      skipPermissions: skip,
      ...(needsReasoningLevel() && currentReasoningLevel !== undefined
        ? { reasoningLevel: currentReasoningLevel }
        : {}),
      ...(props.isNewBranch
        ? { branchType: currentBranchType, branchName: currentBranchName }
        : {}),
    };

    props.onComplete(result);
  };

  const renderStep = () => {
    const currentStep = step();

    if (currentStep === "quick-start") {
      return (
        <QuickStartStep
          history={props.history}
          onResume={handleQuickStartResume}
          onStartNew={handleQuickStartNew}
          onChooseDifferent={handleChooseDifferent}
          onBack={goBack}
        />
      );
    }

    if (currentStep === "branch-type") {
      return (
        <BranchTypeStep onSelect={handleBranchTypeSelect} onBack={goBack} />
      );
    }

    if (currentStep === "branch-name") {
      return (
        <BranchNameStep
          branchType={branchType()}
          onSubmit={handleBranchNameSubmit}
          onBack={goBack}
        />
      );
    }

    if (currentStep === "agent-select") {
      return <AgentSelectStep onSelect={handleAgentSelect} onBack={goBack} />;
    }

    if (currentStep === "model-select") {
      return (
        <ModelSelectStep
          agentId={selectedAgent() ?? "claude-code"}
          onSelect={handleModelSelect}
          onBack={goBack}
        />
      );
    }

    if (currentStep === "reasoning-level") {
      return (
        <ReasoningLevelStep
          onSelect={handleReasoningLevelSelect}
          onBack={goBack}
        />
      );
    }

    if (currentStep === "execution-mode") {
      return (
        <ExecutionModeStep
          onSelect={handleExecutionModeSelect}
          onBack={goBack}
        />
      );
    }

    if (currentStep === "skip-permissions") {
      return (
        <SkipPermissionsStep
          onSelect={handleSkipPermissionsSelect}
          onBack={goBack}
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
