/**
 * ビルトインAIツール定義
 *
 * Claude Code と Codex CLI の CustomAITool 形式定義
 */

import type { CustomAITool } from "../types/tools.js";
import {
  CLAUDE_PERMISSION_SKIP_ARGS,
  CODEX_DEFAULT_ARGS,
} from "../shared/aiToolConstants.js";

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
  permissionSkipArgs: Array.from(CLAUDE_PERMISSION_SKIP_ARGS),
};

/**
 * Codex CLI のビルトイン定義
 */
export const CODEX_CLI_TOOL: CustomAITool = {
  id: "codex-cli",
  displayName: "Codex CLI",
  type: "bunx",
  command: "@openai/codex@latest",
  defaultArgs: Array.from(CODEX_DEFAULT_ARGS),
  modeArgs: {
    normal: [],
    continue: ["resume", "--last"],
    resume: ["resume"],
  },
};

/**
 * すべてのビルトインツール
 */
export const BUILTIN_TOOLS: CustomAITool[] = [CLAUDE_CODE_TOOL, CODEX_CLI_TOOL];
