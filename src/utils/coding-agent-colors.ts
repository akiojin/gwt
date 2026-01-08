/**
 * Coding Agent Color Definitions
 *
 * This module provides consistent color definitions for coding agents
 * across CLI and Web UI (SPEC-3b0ed29b FR-024~FR-027).
 */

/**
 * Tool ID constants for coding agents
 */
export const CODING_AGENT_TOOL_IDS = {
  CLAUDE_CODE: "claude-code",
  CODEX_CLI: "codex-cli",
  GEMINI_CLI: "gemini-cli",
  OPENCODE: "opencode",
} as const;

/**
 * Terminal color names for CLI (OpenTUI/SolidJS)
 */
export type TerminalColorName =
  | "yellow"
  | "cyan"
  | "magenta"
  | "green"
  | "gray"
  | "white";

/**
 * Color definitions for coding agents
 * - Terminal colors: Used in CLI UI (OpenTUI/SolidJS)
 * - Hex colors: Used in Web UI (React/CSS)
 */
export const CODING_AGENT_COLORS = {
  [CODING_AGENT_TOOL_IDS.CLAUDE_CODE]: {
    terminal: "yellow" as TerminalColorName,
    hex: "#f6e05e",
    tailwind: "text-yellow-400",
  },
  [CODING_AGENT_TOOL_IDS.CODEX_CLI]: {
    terminal: "cyan" as TerminalColorName,
    hex: "#4fd1c5",
    tailwind: "text-cyan-400",
  },
  [CODING_AGENT_TOOL_IDS.GEMINI_CLI]: {
    terminal: "magenta" as TerminalColorName,
    hex: "#d53f8c",
    tailwind: "text-pink-500",
  },
  [CODING_AGENT_TOOL_IDS.OPENCODE]: {
    terminal: "green" as TerminalColorName,
    hex: "#48bb78",
    tailwind: "text-green-400",
  },
} as const;

/**
 * Default color for unknown/custom agents
 */
export const DEFAULT_AGENT_COLOR = {
  terminal: "gray" as TerminalColorName,
  hex: "#a0aec0",
  tailwind: "text-gray-400",
};

/**
 * Get terminal color name for a coding agent
 * Used in CLI UI (OpenTUI/SolidJS)
 *
 * @param toolId - The tool ID (e.g., "claude-code", "codex-cli")
 * @param label - Optional label for fallback (used for unknown detection)
 * @returns Terminal color name
 */
export function getAgentTerminalColor(
  toolId?: string | null,
  label?: string,
): TerminalColorName {
  if (!toolId) {
    const trimmed = label?.trim().toLowerCase();
    if (!trimmed || trimmed === "unknown") {
      return "gray";
    }
    return "white";
  }

  const color = CODING_AGENT_COLORS[toolId as keyof typeof CODING_AGENT_COLORS];
  return color?.terminal ?? DEFAULT_AGENT_COLOR.terminal;
}

/**
 * Get hex color for a coding agent
 * Used in Web UI (React/CSS)
 *
 * @param toolId - The tool ID (e.g., "claude-code", "codex-cli")
 * @returns Hex color string
 */
export function getAgentHexColor(toolId?: string | null): string {
  if (!toolId) {
    return DEFAULT_AGENT_COLOR.hex;
  }

  const color = CODING_AGENT_COLORS[toolId as keyof typeof CODING_AGENT_COLORS];
  return color?.hex ?? DEFAULT_AGENT_COLOR.hex;
}

/**
 * Get Tailwind CSS class for a coding agent
 * Used in Web UI (React/Tailwind)
 *
 * @param toolId - The tool ID (e.g., "claude-code", "codex-cli")
 * @returns Tailwind CSS class name
 */
export function getAgentTailwindClass(toolId?: string | null): string {
  if (!toolId) {
    return DEFAULT_AGENT_COLOR.tailwind;
  }

  const color = CODING_AGENT_COLORS[toolId as keyof typeof CODING_AGENT_COLORS];
  return color?.tailwind ?? DEFAULT_AGENT_COLOR.tailwind;
}

/**
 * Normalize agent type string to tool ID
 * Handles various naming conventions used across the codebase
 *
 * @param agentType - Agent type string (e.g., "claude", "codex-cli", "Claude Code")
 * @returns Normalized tool ID
 */
export function normalizeAgentToToolId(
  agentType?: string | null,
): string | null {
  if (!agentType) return null;

  const normalized = agentType.toLowerCase().trim();

  // Direct matches
  if (normalized === "claude-code" || normalized === "claude") {
    return CODING_AGENT_TOOL_IDS.CLAUDE_CODE;
  }
  if (normalized === "codex-cli" || normalized === "codex") {
    return CODING_AGENT_TOOL_IDS.CODEX_CLI;
  }
  if (normalized === "gemini-cli" || normalized === "gemini") {
    return CODING_AGENT_TOOL_IDS.GEMINI_CLI;
  }
  if (normalized === "opencode" || normalized === "open-code") {
    return CODING_AGENT_TOOL_IDS.OPENCODE;
  }

  // Label-based matches
  if (normalized.includes("claude")) {
    return CODING_AGENT_TOOL_IDS.CLAUDE_CODE;
  }
  if (normalized.includes("codex")) {
    return CODING_AGENT_TOOL_IDS.CODEX_CLI;
  }
  if (normalized.includes("gemini")) {
    return CODING_AGENT_TOOL_IDS.GEMINI_CLI;
  }
  if (normalized.includes("opencode")) {
    return CODING_AGENT_TOOL_IDS.OPENCODE;
  }

  return agentType; // Return as-is for custom agents
}
