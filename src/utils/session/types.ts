/**
 * Session types - common type definitions for session parsers
 */

/**
 * Options for session search operations
 */
export interface SessionSearchOptions {
  /** Minimum mtime (ms since epoch) */
  since?: number;
  /** Maximum mtime (ms since epoch) */
  until?: number;
  /** Reference time for preferring closest match */
  preferClosestTo?: number;
  /** Time window for closest match (default: 30 minutes) */
  windowMs?: number;
  /** Working directory filter */
  cwd?: string | null;
  /** Branch name filter (resolved from session cwd via worktree paths) */
  branch?: string | null;
  /** Optional worktree list to avoid shelling out for branch mapping */
  worktrees?: { path: string; branch: string }[] | null;
}

/**
 * Base session info with ID and modification time
 */
export interface SessionInfo {
  id: string;
  mtime: number;
}

/**
 * Claude Code session info
 */
export type ClaudeSessionInfo = SessionInfo;

/**
 * Codex CLI session info
 */
export type CodexSessionInfo = SessionInfo;

/**
 * Gemini CLI session info
 */
export type GeminiSessionInfo = SessionInfo;

/**
 * OpenCode session info
 */
export type OpenCodeSessionInfo = SessionInfo;
