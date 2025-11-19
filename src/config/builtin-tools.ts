/**
 * ビルトインAIツール定義
 *
 * Claude Code と Codex CLI の CustomAITool 形式定義
 */

import type { CustomAITool } from "../types/tools.js";

/**
 * Claude Code のビルトイン定義
 */
export const CLAUDE_CODE_TOOL: CustomAITool = {
  id: "claude-code",
  displayName: "Claude Code",
  type: "bunx",
  command: "@anthropic-ai/claude-code@latest",
  modeArgs: {
    normal: [],
    continue: ["-c"],
    resume: ["-r"],
  },
  permissionSkipArgs: ["--yes"],
};

/**
 * Codex CLI のビルトイン定義
 */
export const CODEX_CLI_TOOL: CustomAITool = {
  id: "codex-cli",
  displayName: "Codex CLI",
  type: "bunx",
  command: "@openai/codex@latest",
  defaultArgs: ["--auto-approve", "--verbose"],
  modeArgs: {
    normal: [],
    continue: ["resume", "--last"],
    resume: ["resume"],
  },
};

/**
 * Gemini CLI のビルトイン定義
 */
export const GEMINI_CLI_TOOL: CustomAITool = {
  id: "gemini-cli",
  displayName: "Gemini CLI",
  type: "bunx",
  command: "@google/gemini-cli@latest",
  modeArgs: {
    normal: [],
    continue: ["-r", "latest"],
    resume: ["-r", "latest"],
  },
  permissionSkipArgs: ["-y"],
};

/**
 * すべてのビルトインツール
 */
export const BUILTIN_TOOLS: CustomAITool[] = [
  CLAUDE_CODE_TOOL,
  CODEX_CLI_TOOL,
  GEMINI_CLI_TOOL,
];
