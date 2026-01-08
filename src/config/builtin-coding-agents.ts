/**
 * ビルトインコーディングエージェント定義
 *
 * Claude Code、Codex、Gemini の CodingAgent 形式定義
 */

import type { CodingAgent } from "../types/tools.js";
import {
  CLAUDE_PERMISSION_SKIP_ARGS,
  CODEX_DEFAULT_ARGS,
} from "../shared/codingAgentConstants.js";

/**
 * Claude Code のビルトイン定義
 */
export const CLAUDE_CODE_TOOL: CodingAgent = {
  id: "claude-code",
  displayName: "Claude Code",
  type: "bunx",
  command: "@anthropic-ai/claude-code",
  modeArgs: {
    normal: [],
    continue: ["-c"],
    resume: ["-r"],
  },
  permissionSkipArgs: Array.from(CLAUDE_PERMISSION_SKIP_ARGS),
  env: {
    ENABLE_LSP_TOOL: "true",
  },
};

/**
 * Codex のビルトイン定義
 */
export const CODEX_CLI_TOOL: CodingAgent = {
  id: "codex-cli",
  displayName: "Codex",
  type: "bunx",
  command: "@openai/codex",
  defaultArgs: Array.from(CODEX_DEFAULT_ARGS),
  modeArgs: {
    normal: [],
    continue: ["resume", "--last"],
    resume: ["resume"],
  },
};

/**
 * Gemini のビルトイン定義
 */
export const GEMINI_CLI_TOOL: CodingAgent = {
  id: "gemini-cli",
  displayName: "Gemini",
  type: "bunx",
  command: "@google/gemini-cli",
  modeArgs: {
    normal: [],
    continue: ["-r", "latest"],
    resume: ["-r", "latest"],
  },
  permissionSkipArgs: ["-y"],
};

/**
 * OpenCode のビルトイン定義
 */
export const OPENCODE_TOOL: CodingAgent = {
  id: "opencode",
  displayName: "OpenCode",
  type: "bunx",
  command: "opencode-ai",
  modeArgs: {
    normal: [],
    continue: ["-c"],
    resume: ["-s"],
  },
};

/**
 * すべてのビルトインコーディングエージェント
 */
export const BUILTIN_CODING_AGENTS: CodingAgent[] = [
  CLAUDE_CODE_TOOL,
  CODEX_CLI_TOOL,
  GEMINI_CLI_TOOL,
  OPENCODE_TOOL,
];
