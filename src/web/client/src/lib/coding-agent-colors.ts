/**
 * Coding Agent Color Definitions for Web UI
 *
 * This module provides consistent color definitions for coding agents
 * in the Web UI (React/Tailwind) (SPEC-3b0ed29b FR-024~FR-027).
 *
 * Note: This mirrors the CLI color utility at src/utils/coding-agent-colors.ts
 * Both files should be kept in sync.
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
 * Color definitions for coding agents
 */
export const CODING_AGENT_COLORS = {
  [CODING_AGENT_TOOL_IDS.CLAUDE_CODE]: {
    hex: "#f6e05e",
    tailwind: "text-yellow-400",
    bgTailwind: "bg-yellow-400/20",
    borderTailwind: "border-yellow-400",
  },
  [CODING_AGENT_TOOL_IDS.CODEX_CLI]: {
    hex: "#4fd1c5",
    tailwind: "text-cyan-400",
    bgTailwind: "bg-cyan-400/20",
    borderTailwind: "border-cyan-400",
  },
  [CODING_AGENT_TOOL_IDS.GEMINI_CLI]: {
    hex: "#d53f8c",
    tailwind: "text-pink-500",
    bgTailwind: "bg-pink-500/20",
    borderTailwind: "border-pink-500",
  },
  [CODING_AGENT_TOOL_IDS.OPENCODE]: {
    hex: "#48bb78",
    tailwind: "text-green-400",
    bgTailwind: "bg-green-400/20",
    borderTailwind: "border-green-400",
  },
} as const;

/**
 * Default color for unknown/custom agents
 */
export const DEFAULT_AGENT_COLOR = {
  hex: "#a0aec0",
  tailwind: "text-gray-400",
  bgTailwind: "bg-gray-400/20",
  borderTailwind: "border-gray-400",
};

/**
 * Get Tailwind CSS text color class for a coding agent
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
 * Get Tailwind CSS background color class for a coding agent
 *
 * @param toolId - The tool ID (e.g., "claude-code", "codex-cli")
 * @returns Tailwind CSS class name for background
 */
export function getAgentBgTailwindClass(toolId?: string | null): string {
  if (!toolId) {
    return DEFAULT_AGENT_COLOR.bgTailwind;
  }

  const color = CODING_AGENT_COLORS[toolId as keyof typeof CODING_AGENT_COLORS];
  return color?.bgTailwind ?? DEFAULT_AGENT_COLOR.bgTailwind;
}

/**
 * Get hex color for a coding agent
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
