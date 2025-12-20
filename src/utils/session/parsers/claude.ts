/**
 * Claude Code session parser
 *
 * Handles session detection and management for Claude Code CLI.
 * Session files are stored in ~/.claude/projects/<encoded-path>/sessions/
 */

import path from "node:path";
import { homedir } from "node:os";

import type { ClaudeSessionInfo, SessionSearchOptions } from "../types.js";
import {
  isValidUuidSessionId,
  findNewestSessionIdFromDir,
  matchesCwd,
  readFileContent,
  checkFileStat,
} from "../common.js";

/**
 * Encodes a project path for Claude's directory structure.
 * Normalizes separators and replaces special characters with dashes.
 * @param cwd - The working directory path to encode
 * @returns The encoded path string suitable for Claude's directory naming
 */
export function encodeClaudeProjectPath(cwd: string): string {
  // Normalize to forward slashes, drop drive colon, replace / and _ with -
  const normalized = cwd.replace(/\\/g, "/").replace(/:/g, "");
  return normalized.replace(/_/g, "-").replace(/\//g, "-");
}

/**
 * Generates candidate paths for Claude project directory.
 * Handles various encoding patterns used by different Claude versions.
 * @param cwd - The working directory path to encode
 * @returns Array of possible encoded directory names
 */
function generateClaudeProjectPathCandidates(cwd: string): string[] {
  const base = encodeClaudeProjectPath(cwd);
  const dotToDash = cwd
    .replace(/\\/g, "/")
    .replace(/:/g, "")
    .replace(/\./g, "-")
    .replace(/_/g, "-")
    .replace(/\//g, "-");
  const collapsed = dotToDash.replace(/-+/g, "-");
  const candidates = [base, dotToDash, collapsed];
  return Array.from(new Set(candidates));
}

/**
 * Returns the list of possible Claude root directories.
 * Checks CLAUDE_CONFIG_DIR environment variable first, then falls back to
 * standard locations (~/.claude and ~/.config/claude).
 * @returns Array of possible Claude root directory paths
 */
function getClaudeRootCandidates(): string[] {
  const roots: string[] = [];
  if (process.env.CLAUDE_CONFIG_DIR) {
    roots.push(process.env.CLAUDE_CONFIG_DIR);
  }
  roots.push(
    path.join(homedir(), ".claude"),
    path.join(homedir(), ".config", "claude"),
  );
  return roots;
}

/**
 * Finds the latest Claude session for a given working directory.
 *
 * Search order:
 * 1. ~/.claude/projects/<encoded>/sessions/ (official location)
 * 2. ~/.claude/projects/<encoded>/ (root and subdirs)
 * 3. ~/.claude/history.jsonl (global history fallback)
 *
 * @param cwd - The working directory to find sessions for
 * @param options - Search options (since, until, preferClosestTo, windowMs)
 * @returns Session info with ID and modification time, or null if not found
 */
export async function findLatestClaudeSession(
  cwd: string,
  options: Omit<SessionSearchOptions, "cwd"> = {},
): Promise<ClaudeSessionInfo | null> {
  const rootCandidates = getClaudeRootCandidates();
  const encodedPaths = generateClaudeProjectPathCandidates(cwd);

  for (const claudeRoot of rootCandidates) {
    for (const encoded of encodedPaths) {
      const projectDir = path.join(claudeRoot, "projects", encoded);
      const sessionsDir = path.join(projectDir, "sessions");

      // 1) Look under sessions/ (official location)
      const session = await findNewestSessionIdFromDir(
        sessionsDir,
        false,
        options,
      );
      if (session) return session;

      // 2) Look directly under project dir and subdirs
      const rootSession = await findNewestSessionIdFromDir(
        projectDir,
        true,
        options,
      );
      if (rootSession) return rootSession;
    }
  }

  // Fallback: parse ~/.claude/history.jsonl
  try {
    const historyPath = path.join(homedir(), ".claude", "history.jsonl");
    const historyStat = await checkFileStat(historyPath);
    if (!historyStat) return null;

    const content = await readFileContent(historyPath);
    const lines = content.split(/\r?\n/).filter(Boolean);
    for (let i = lines.length - 1; i >= 0; i -= 1) {
      try {
        const line = lines[i] ?? "";
        const parsed = JSON.parse(line) as Record<string, unknown>;
        const project =
          typeof parsed.project === "string" ? parsed.project : null;
        const sessionId =
          typeof parsed.sessionId === "string" ? parsed.sessionId : null;
        if (project && sessionId && matchesCwd(project, cwd)) {
          return { id: sessionId, mtime: historyStat.mtimeMs };
        }
      } catch {
        // ignore malformed lines
      }
    }
  } catch {
    // ignore if history not present
  }

  return null;
}

/**
 * Finds the latest Claude session ID for a given working directory.
 * @param cwd - The working directory to find sessions for
 * @param options - Search options (since, until, preferClosestTo, windowMs)
 * @returns Session ID string or null if not found
 */
export async function findLatestClaudeSessionId(
  cwd: string,
  options: Omit<SessionSearchOptions, "cwd"> = {},
): Promise<string | null> {
  const found = await findLatestClaudeSession(cwd, options);
  return found?.id ?? null;
}

/**
 * Polls for a Claude session ID until found or timeout.
 * @param cwd - The working directory to find sessions for
 * @param options - Polling options including timeout, interval, and search filters
 * @returns Session ID string or null if timeout reached
 */
export async function waitForClaudeSessionId(
  cwd: string,
  options: {
    timeoutMs?: number;
    pollIntervalMs?: number;
    since?: number;
    until?: number;
    preferClosestTo?: number;
    windowMs?: number;
  } = {},
): Promise<string | null> {
  const timeoutMs = options.timeoutMs ?? 120_000;
  const pollIntervalMs = options.pollIntervalMs ?? 2_000;
  const deadline = Date.now() + timeoutMs;

  // Build search options once outside the loop
  const searchOptions: Omit<SessionSearchOptions, "cwd"> = {};
  if (options.since !== undefined) searchOptions.since = options.since;
  if (options.until !== undefined) searchOptions.until = options.until;
  if (options.preferClosestTo !== undefined)
    searchOptions.preferClosestTo = options.preferClosestTo;
  if (options.windowMs !== undefined) searchOptions.windowMs = options.windowMs;

  while (Date.now() < deadline) {
    const found = await findLatestClaudeSession(cwd, searchOptions);
    if (found?.id) return found.id;
    await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
  }
  return null;
}

/**
 * Checks if a Claude session file exists for the given session ID and worktree path.
 * @param sessionId - The session ID to check
 * @param worktreePath - The worktree path (used to determine project encoding)
 * @returns true if a session file exists for this ID
 */
export async function claudeSessionFileExists(
  sessionId: string,
  worktreePath: string,
): Promise<boolean> {
  if (!isValidUuidSessionId(sessionId)) {
    return false;
  }

  const encodedPaths = generateClaudeProjectPathCandidates(worktreePath);
  const roots = getClaudeRootCandidates();

  for (const root of roots) {
    for (const enc of encodedPaths) {
      // Check official sessions/ location first
      const sessionsCandidate = path.join(
        root,
        "projects",
        enc,
        "sessions",
        `${sessionId}.jsonl`,
      );
      const sessionsInfo = await checkFileStat(sessionsCandidate);
      if (sessionsInfo) {
        return true;
      }

      // Then check project root
      const candidate = path.join(root, "projects", enc, `${sessionId}.jsonl`);
      const info = await checkFileStat(candidate);
      if (info) {
        return true;
      }
    }
  }
  return false;
}
