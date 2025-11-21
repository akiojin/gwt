import type { AITool, InferenceLevel, ModelOption } from "../types.js";

const CODEX_BASE_LEVELS: InferenceLevel[] = ["high", "medium", "low"];
const CODEX_MAX_LEVELS: InferenceLevel[] = ["xhigh", "high", "medium", "low"];

const MODEL_OPTIONS: Record<string, ModelOption[]> = {
  "claude-code": [
    {
      id: "default",
      label: "Default (recommended) — Sonnet 4.5",
      description:
        "Official default alias. Tracks the recommended Claude Code model (currently Sonnet 4.5) and shows as a standard model in /model.",
      isDefault: true,
    },
    {
      id: "opus",
      label: "Opus 4.1",
      description:
        "Official Opus alias for Claude Code (non-custom, matches /model option).",
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
      id: "gpt-5.1-codex",
      label: "gpt-5.1-codex",
      description: "Standard Codex model",
      inferenceLevels: CODEX_BASE_LEVELS,
      defaultInference: "high",
      isDefault: true,
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
      id: "gemini-3-pro-preview",
      label: "Pro (gemini-3-pro-preview)",
      description:
        "Default Pro. Falls back to gemini-2.5-pro when preview is unavailable.",
      isDefault: true,
    },
    {
      id: "gemini-2.5-pro",
      label: "Pro (gemini-2.5-pro)",
      description: "Stable Pro model for deep reasoning and creativity",
    },
    {
      id: "gemini-2.5-flash",
      label: "Flash (gemini-2.5-flash)",
      description: "Balance of speed and reasoning",
    },
    {
      id: "gemini-2.5-flash-lite",
      label: "Flash-Lite (gemini-2.5-flash-lite)",
      description: "Fastest for simple tasks",
    },
  ],
  "qwen-cli": [
    {
      id: "coder-model",
      label: "Coder Model",
      description:
        "Latest Qwen Coder model (qwen3-coder-plus-2025-09-23) from Alibaba Cloud ModelStudio",
      isDefault: true,
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
