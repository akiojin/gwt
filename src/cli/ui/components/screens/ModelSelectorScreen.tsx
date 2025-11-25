import React, { useEffect, useMemo, useState } from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select, type SelectItem } from "../common/Select.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { AITool, InferenceLevel, ModelOption } from "../../types.js";
import {
  getDefaultInferenceForModel,
  getDefaultModelOption,
  getInferenceLevelsForModel,
  getModelOptions,
} from "../../utils/modelOptions.js";

export interface ModelSelectionResult {
  model: string | null;
  inferenceLevel?: InferenceLevel;
}

interface ModelSelectItem extends SelectItem {
  description?: string;
}

interface InferenceSelectItem extends SelectItem {
  hint?: string;
}

export interface ModelSelectorScreenProps {
  tool: AITool;
  onBack: () => void;
  onSelect: (selection: ModelSelectionResult) => void;
  version?: string | null;
  initialSelection?: ModelSelectionResult | null;
}

const TOOL_LABELS: Record<string, string> = {
  "claude-code": "Claude Code",
  "codex-cli": "Codex",
  "gemini-cli": "Gemini",
  "qwen-cli": "Qwen",
};

const INFERENCE_LABELS: Record<InferenceLevel, string> = {
  low: "Low (lighter reasoning)",
  medium: "Medium (balanced reasoning)",
  high: "High (deeper reasoning)",
  xhigh: "Extra high (maximum reasoning)",
};

/**
 * モデル選択 → (必要なら) 推論レベル選択を行う画面
 */
export function ModelSelectorScreen({
  tool,
  onBack,
  onSelect,
  version,
  initialSelection,
}: ModelSelectorScreenProps) {
  const { rows } = useTerminalSize();

  const [step, setStep] = useState<"model" | "inference">("model");
  const [modelOptions, setModelOptions] = useState<ModelOption[]>([]);
  const [selectedModel, setSelectedModel] = useState<ModelOption | null>(null);

  // モデル候補をツールに応じてロード
  useEffect(() => {
    const options = getModelOptions(tool);
    setModelOptions(options);
    // 初期選択が有効なら保持
    if (initialSelection?.model) {
      const found = options.find((opt) => opt.id === initialSelection.model);
      if (found) {
        setSelectedModel(found);
        setStep("model");
        return;
      }
    }
    setSelectedModel(null);
    setStep("model");
  }, [tool, initialSelection?.model]);

  const modelItems: ModelSelectItem[] = useMemo(
    () =>
      modelOptions.map((option) => ({
        label: option.label,
        value: option.id,
        ...(option.description ? { description: option.description } : {}),
      })),
    [modelOptions],
  );

  const defaultModelIndex = useMemo(() => {
    const initial = initialSelection?.model
      ? modelOptions.findIndex((opt) => opt.id === initialSelection.model)
      : -1;
    if (initial !== -1) return initial;
    const defaultOption = getDefaultModelOption(tool);
    if (!defaultOption) return 0;
    const index = modelOptions.findIndex((opt) => opt.id === defaultOption.id);
    return index >= 0 ? index : 0;
  }, [initialSelection?.model, modelOptions, tool]);

  const inferenceOptions = useMemo(
    () => getInferenceLevelsForModel(selectedModel ?? undefined),
    [selectedModel],
  );

  const inferenceItems: InferenceSelectItem[] = useMemo(
    () => {
      return inferenceOptions.map((level) => {
        if (selectedModel?.id === "gpt-5.1-codex-max") {
          if (level === "low") {
            return {
              label: "Low",
              value: level,
              hint: "Fast responses with lighter reasoning",
            };
          }
          if (level === "medium") {
            return {
              label: "Medium (default)",
              value: level,
              hint: "Balances speed and reasoning depth for everyday tasks",
            };
          }
          if (level === "high") {
            return {
              label: "High",
              value: level,
              hint: "Maximizes reasoning depth for complex problems",
            };
          }
          if (level === "xhigh") {
            return {
              label: "Extra high",
              value: level,
              hint:
                "Extra high reasoning depth; may quickly consume Plus plan rate limits.",
            };
          }
        }

        return {
          label: INFERENCE_LABELS[level],
          value: level,
        };
      });
    },
    [inferenceOptions, selectedModel?.id],
  );

  const defaultInferenceIndex = useMemo(() => {
    const initialLevel = initialSelection?.inferenceLevel;
    if (initialLevel && inferenceOptions.includes(initialLevel)) {
      return inferenceOptions.findIndex((lvl) => lvl === initialLevel);
    }
    const defaultLevel = getDefaultInferenceForModel(selectedModel ?? undefined);
    if (!defaultLevel) return 0;
    const index = inferenceOptions.findIndex((lvl) => lvl === defaultLevel);
    return index >= 0 ? index : 0;
  }, [initialSelection?.inferenceLevel, inferenceOptions, selectedModel]);

  useInput((_input, key) => {
    if (key.escape) {
      if (step === "inference") {
        setStep("model");
        return;
      }
      onBack();
    }
  });

  const handleModelSelect = (item: ModelSelectItem) => {
    const option =
      modelOptions.find((opt) => opt.id === item.value) ?? modelOptions[0];

    if (!option) {
      onSelect({ model: null });
      return;
    }

    setSelectedModel(option);

    const levels = getInferenceLevelsForModel(option);
    if (levels.length > 0) {
      setStep("inference");
    } else {
      onSelect({ model: option.id });
    }
  };

  const handleInferenceSelect = (item: InferenceSelectItem) => {
    if (!selectedModel) {
      setStep("model");
      return;
    }

    onSelect({
      model: selectedModel.id,
      inferenceLevel: item.value as InferenceLevel,
    });
  };

  const footerActions =
    step === "model"
      ? [
          { key: "enter", description: "Select" },
          { key: "esc", description: "Back" },
        ]
      : [
          { key: "enter", description: "Select" },
          { key: "esc", description: "Back to model" },
        ];

  const toolLabel = TOOL_LABELS[tool] ?? tool;

  const renderModelItem = (
    item: ModelSelectItem,
    isSelected: boolean,
  ): React.ReactNode => (
    <Box flexDirection="column">
      {isSelected ? (
        <Text color="cyan">➤ {item.label}</Text>
      ) : (
        <Text>  {item.label}</Text>
      )}
      {item.description ? (
        <Text color="gray">    {item.description}</Text>
      ) : null}
    </Box>
  );

  return (
    <Box flexDirection="column" height={rows}>
      <Header
        title={step === "model" ? "Model Selection" : "Inference Level"}
        titleColor="blue"
        version={version}
      />

      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        {step === "model" ? (
          <>
            {tool === "gemini-cli" ? (
              <Box marginBottom={1} flexDirection="column">
                <Text>Gemini 3 preview is enabled.</Text>
                <Text>
                  Selecting Pro uses gemini-3-pro-preview and falls back to
                  gemini-2.5-pro if unavailable.
                </Text>
                <Text>Use --model to pin a specific Gemini model.</Text>
              </Box>
            ) : null}

            <Box marginBottom={1}>
              <Text>
                Select a model for {toolLabel}
                {modelOptions.length === 0 ? " (no options)" : ""}
              </Text>
            </Box>
            {tool === "qwen-cli" ? (
              <Box marginBottom={1} flexDirection="column">
                <Text>Latest Qwen models from Alibaba Cloud ModelStudio:</Text>
                <Text>• coder-model (qwen3-coder-plus-2025-09-23)</Text>
                <Text>• vision-model (qwen3-vl-plus-2025-09-23)</Text>
              </Box>
            ) : null}

            {modelItems.length === 0 ? (
              <Select
                items={[
                  {
                    label: "No model selection required. Press Enter to continue.",
                    value: "__continue__",
                  },
                ]}
                onSelect={() => onSelect({ model: null })}
              />
            ) : (
              <Select
                items={modelItems}
                onSelect={handleModelSelect}
                initialIndex={defaultModelIndex}
                renderItem={renderModelItem}
              />
            )}
          </>
        ) : (
          <>
            <Box marginBottom={1}>
              <Text>
                Select reasoning level for {selectedModel?.label ?? "model"}
              </Text>
            </Box>
            <Select
              items={inferenceItems}
              onSelect={handleInferenceSelect}
              initialIndex={defaultInferenceIndex}
              renderItem={(item, isSelected) => (
                <Box flexDirection="column">
                  {isSelected ? (
                    <Text color="cyan">➤ {item.label}</Text>
                  ) : (
                    <Text>  {item.label}</Text>
                  )}
                  {"hint" in item && item.hint ? (
                    <Text color="gray">    {item.hint}</Text>
                  ) : null}
                </Box>
              )}
            />
          </>
        )}
      </Box>

      <Footer actions={footerActions} />
    </Box>
  );
}
