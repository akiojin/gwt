import type { AITool, InferenceLevel, ModelOption } from "../types.js";

const CODEX_BASE_LEVELS: InferenceLevel[] = ["high", "medium", "low"];
const CODEX_MAX_LEVELS: InferenceLevel[] = ["xhigh", "high", "medium", "low"];

const MODEL_OPTIONS: Record<string, ModelOption[]> = {
  "claude-code": [
    {
      id: "",
      label: "Default (Auto)",
      description: "Use Claude Code default behavior",
      isDefault: true,
    },
    {
      id: "opus",
      label: "Opus 4.5",
      description:
        "Official Opus alias for Claude Code (non-custom, matches /model option).",
    },
    {
      id: "sonnet",
      label: "Sonnet 4.5",
      description: "Official Sonnet alias for Claude Code.",
    },
    {
      id: "haiku",
      label: "Haiku 4.5",
      description:
        "Official Haiku alias for Claude Code (fastest model, non-custom).",
    },
  ],
  "codex-cli": [
    {
      id: "",
      label: "Default (Auto)",
      description: "Use Codex default model",
      isDefault: true,
      inferenceLevels: CODEX_BASE_LEVELS,
      defaultInference: "high",
    },
    {
      id: "gpt-5.1-codex",
      label: "gpt-5.1-codex",
      description: "Standard Codex model",
      inferenceLevels: CODEX_BASE_LEVELS,
      defaultInference: "high",
    },
    {
      id: "gpt-5.2",
      label: "gpt-5.2",
      description: "Latest frontier model with extra high reasoning",
      inferenceLevels: CODEX_MAX_LEVELS,
      defaultInference: "medium",
    },
    {
      id: "gpt-5.1-codex-max",
      label: "gpt-5.1-codex-max",
      description: "Max performance (xhigh available)",
      inferenceLevels: CODEX_MAX_LEVELS,
      defaultInference: "medium",
    },
    {
      id: "gpt-5.1-codex-mini",
      label: "gpt-5.1-codex-mini",
      description: "Lightweight / cost-saving",
      inferenceLevels: CODEX_BASE_LEVELS,
      defaultInference: "medium",
    },
    {
      id: "gpt-5.1",
      label: "gpt-5.1",
      description: "General-purpose GPT-5.1",
      inferenceLevels: CODEX_BASE_LEVELS,
      defaultInference: "high",
    },
  ],
  "gemini-cli": [
    {
      id: "gemini-3",
      label: "Auto (Gemini 3)",
      description:
        "Let Gemini CLI decide the best model for the task: gemini-3-pro, gemini-3-flash",
      isDefault: true,
    },
    {
      id: "gemini-2.5",
      label: "Auto (Gemini 2.5)",
      description:
        "Let Gemini CLI decide the best model for the task: gemini-2.5-pro, gemini-2.5-flash",
    },
    {
      id: "gemini-3-pro-preview",
      label: "Manual (gemini-3-pro-preview)",
      description:
        "Default Pro. Falls back to gemini-2.5-pro when preview is unavailable.",
    },
    {
      id: "gemini-3-flash-preview",
      label: "Manual (gemini-3-flash-preview)",
      description: "Manually select a model",
    },
    {
      id: "gemini-2.5-pro",
      label: "Manual (gemini-2.5-pro)",
      description: "Stable Pro model for deep reasoning and creativity",
    },
    {
      id: "gemini-2.5-flash",
      label: "Manual (gemini-2.5-flash)",
      description: "Balance of speed and reasoning",
    },
    {
      id: "gemini-2.5-flash-lite",
      label: "Manual (gemini-2.5-flash-lite)",
      description: "Fastest for simple tasks",
    },
  ],
  "qwen-cli": [
    {
      id: "",
      label: "Default (Auto)",
      description: "Use Qwen CLI default model",
      isDefault: true,
    },
    {
      id: "coder-model",
      label: "Coder Model",
      description:
        "Latest Qwen Coder model (qwen3-coder-plus-2025-09-23) from Alibaba Cloud ModelStudio",
    },
    {
      id: "vision-model",
      label: "Vision Model",
      description:
        "Latest Qwen Vision model (qwen3-vl-plus-2025-09-23) from Alibaba Cloud ModelStudio",
    },
  ],
};

export function getModelOptions(tool: AITool): ModelOption[] {
  return MODEL_OPTIONS[tool] ?? [];
}

export function getDefaultModelOption(tool: AITool): ModelOption | undefined {
  const options = getModelOptions(tool);
  return options.find((opt) => opt.isDefault) ?? options[0];
}

export function getInferenceLevelsForModel(
  model?: ModelOption,
): InferenceLevel[] {
  if (!model?.inferenceLevels || model.inferenceLevels.length === 0) {
    return [];
  }
  return model.inferenceLevels;
}

export function getDefaultInferenceForModel(
  model?: ModelOption,
): InferenceLevel | undefined {
  if (!model) return undefined;
  if (model.defaultInference) return model.defaultInference;
  const levels = getInferenceLevelsForModel(model);
  return levels[0];
}
