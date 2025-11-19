/**
 * ビルトインAIツール定義
 *
 * Claude Code、Codex、Gemini、Qwen の CustomAITool 形式定義
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
 * Codex のビルトイン定義
 */
export const CODEX_CLI_TOOL: CustomAITool = {
  id: "codex-cli",
  displayName: "Codex",
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
 * Gemini のビルトイン定義
 */
export const GEMINI_CLI_TOOL: CustomAITool = {
  id: "gemini-cli",
  displayName: "Gemini",
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
 * Qwen のビルトイン定義
 */
export const QWEN_CLI_TOOL: CustomAITool = {
  id: "qwen-cli",
  displayName: "Qwen",
  type: "bunx",
  command: "@qwen-code/qwen-code@latest",
  defaultArgs: ["--checkpointing"],
  modeArgs: {
    normal: [],
    continue: [],
    resume: [],
  },
  permissionSkipArgs: ["--yolo"],
};

/**
 * すべてのビルトインツール
 */
export const BUILTIN_TOOLS: CustomAITool[] = [
  CLAUDE_CODE_TOOL,
  CODEX_CLI_TOOL,
  GEMINI_CLI_TOOL,
  QWEN_CLI_TOOL,
];
