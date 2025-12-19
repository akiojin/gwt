import type { AITool, InferenceLevel, ModelOption } from "../types.js";

const CODEX_BASE_LEVELS: InferenceLevel[] = ["high", "medium", "low"];
const CODEX_MAX_LEVELS: InferenceLevel[] = ["xhigh", "high", "medium", "low"];
const CLAUDE_MODEL_ALIASES = new Set(["opus", "sonnet", "haiku"]);

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
      id: "gpt-5.2-codex",
      label: "gpt-5.2-codex",
      description: "Latest frontier agentic coding model",
      inferenceLevels: CODEX_MAX_LEVELS,
      defaultInference: "high",
    },
    {
      id: "gpt-5.1-codex-max",
      label: "gpt-5.1-codex-max",
      description: "Codex-optimized flagship for deep and fast reasoning.",
      inferenceLevels: CODEX_MAX_LEVELS,
      defaultInference: "medium",
    },
    {
      id: "gpt-5.1-codex-mini",
      label: "gpt-5.1-codex-mini",
      description: "Optimized for codex. Cheaper, faster, but less capable.",
      inferenceLevels: CODEX_BASE_LEVELS,
      defaultInference: "medium",
    },
    {
      id: "gpt-5.2",
      label: "gpt-5.2",
      description:
        "Latest frontier model with improvements across knowledge, reasoning and coding",
      inferenceLevels: CODEX_MAX_LEVELS,
      defaultInference: "medium",
    },
  ],
  "gemini-cli": [
    {
      id: "",
      label: "Default (Auto)",
      description: "Use Gemini CLI default model",
      isDefault: true,
    },
    {
      id: "gemini-3-pro-preview",
      label: "Pro (gemini-3-pro-preview)",
      description:
        "Default Pro. Falls back to gemini-2.5-pro when preview is unavailable.",
    },
    {
      id: "gemini-3-flash-preview",
      label: "Flash (gemini-3-flash-preview)",
      description: "Next-generation high-speed model",
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

/**
 * Normalize a model identifier for consistent display and persistence.
 */
export function normalizeModelId(
  tool: AITool,
  model?: string | null,
): string | null {
  if (model === null || model === undefined) return model ?? null;
  const trimmed = model.trim();
  if (!trimmed) return trimmed;
  if (tool === "claude-code") {
    const lower = trimmed.toLowerCase();
    if (lower === "opuss") return "opus";
    if (CLAUDE_MODEL_ALIASES.has(lower)) return lower;
  }
  return trimmed;
}
