/**
 * Session module - unified session management for AI tools
 *
 * This module provides session detection and management for various AI CLI tools:
 * - Claude Code
 * - Codex CLI
 * - Gemini CLI
 */

// Type exports
export type {
  SessionSearchOptions,
  SessionInfo,
  ClaudeSessionInfo,
  CodexSessionInfo,
  GeminiSessionInfo,
} from "./types.js";

// Common utilities
export { isValidUuidSessionId } from "./common.js";

// Claude Code parser
export {
  encodeClaudeProjectPath,
  findLatestClaudeSession,
  findLatestClaudeSessionId,
  waitForClaudeSessionId,
  claudeSessionFileExists,
} from "./parsers/claude.js";

// Codex CLI parser
export {
  findLatestCodexSession,
  findLatestCodexSessionId,
  waitForCodexSessionId,
} from "./parsers/codex.js";

// Gemini CLI parser
export {
  findLatestGeminiSession,
  findLatestGeminiSessionId,
} from "./parsers/gemini.js";
