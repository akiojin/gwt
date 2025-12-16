/**
 * Session common utilities - shared helper functions for session parsers
 */

import path from "node:path";
import { readdir, readFile, stat } from "node:fs/promises";

import type { SessionSearchOptions } from "./types.js";

/**
 * Regular expression for UUID matching
 */
export const UUID_REGEX =
  /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

/**
 * Validates that a string is a properly formatted UUID session ID.
 * @param id - The string to validate
 * @returns true if the string is a valid UUID format
 */
export function isValidUuidSessionId(id: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
    id,
  );
}

/**
 * Extracts session ID from an object by checking common key names.
 * Only returns valid UUIDs.
 */
export function pickSessionIdFromObject(obj: unknown): string | null {
  if (!obj || typeof obj !== "object") return null;
  const candidate = obj as Record<string, unknown>;
  const keys = ["sessionId", "session_id", "id", "conversation_id"];
  for (const key of keys) {
    const value = candidate[key];
    if (typeof value === "string" && value.trim().length > 0) {
      const trimmed = value.trim();
      if (isValidUuidSessionId(trimmed)) {
        return trimmed;
      }
    }
  }
  return null;
}

/**
 * Extracts working directory from an object by checking common key names.
 * Also checks nested payload object (for Codex session format).
 */
export function pickCwdFromObject(obj: unknown): string | null {
  if (!obj || typeof obj !== "object") return null;
  const candidate = obj as Record<string, unknown>;
  const keys = [
    "cwd",
    "workingDirectory",
    "workdir",
    "directory",
    "projectPath",
  ];
  for (const key of keys) {
    const value = candidate[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value;
    }
  }
  // Check nested payload object (for Codex session format)
  const payload = candidate["payload"];
  if (payload && typeof payload === "object") {
    const nested = pickCwdFromObject(payload);
    if (nested) return nested;
  }
  return null;
}

/**
 * Extracts session ID from text content.
 * Tries JSON parsing first, then JSONL lines, then regex fallback.
 */
export function pickSessionIdFromText(content: string): string | null {
  // Try whole content as JSON
  try {
    const parsed = JSON.parse(content);
    const fromObject = pickSessionIdFromObject(parsed);
    if (fromObject) return fromObject;
  } catch {
    // ignore
  }

  // Try JSONL lines
  const lines = content.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const parsedLine = JSON.parse(trimmed);
      const fromLine = pickSessionIdFromObject(parsedLine);
      if (fromLine) return fromLine;
    } catch {
      // ignore
    }
    const match = trimmed.match(UUID_REGEX);
    if (match) return match[0];
  }

  // Fallback: find any UUID in the whole text
  const match = content.match(UUID_REGEX);
  return match ? match[0] : null;
}

/**
 * Finds the latest file in a directory matching a filter.
 */
export async function findLatestFile(
  dir: string,
  filter: (name: string) => boolean,
): Promise<string | null> {
  try {
    const entries = await readdir(dir, { withFileTypes: true });
    const files = entries.filter((e) => e.isFile()).map((e) => e.name);
    const filtered = files.filter(filter);
    if (!filtered.length) return null;

    const withStats = await Promise.all(
      filtered.map(async (name) => {
        const fullPath = path.join(dir, name);
        try {
          const info = await stat(fullPath);
          return { fullPath, mtime: info.mtimeMs };
        } catch {
          return null;
        }
      }),
    );

    const valid = withStats.filter(
      (entry): entry is { fullPath: string; mtime: number } => Boolean(entry),
    );
    if (!valid.length) return null;

    valid.sort((a, b) => b.mtime - a.mtime);
    return valid[0]?.fullPath ?? null;
  } catch {
    return null;
  }
}

/**
 * Collects files recursively from a directory matching a filter.
 * Uses queue-based iteration to avoid stack overflow on deep directory structures.
 * @param dir - The root directory to search
 * @param filter - Function to filter files by name
 * @returns Array of matching files with their paths and modification times
 */
export async function collectFilesRecursive(
  dir: string,
  filter: (name: string) => boolean,
): Promise<{ fullPath: string; mtime: number }[]> {
  const results: { fullPath: string; mtime: number }[] = [];
  const queue: string[] = [dir];

  while (queue.length > 0) {
    const currentDir = queue.shift();
    if (!currentDir) break;
    try {
      const entries = await readdir(currentDir, { withFileTypes: true });
      for (const entry of entries) {
        const fullPath = path.join(currentDir, entry.name);
        if (entry.isDirectory()) {
          queue.push(fullPath);
        } else if (entry.isFile() && filter(entry.name)) {
          try {
            const info = await stat(fullPath);
            results.push({ fullPath, mtime: info.mtimeMs });
          } catch {
            // ignore unreadable file
          }
        }
      }
    } catch {
      // ignore unreadable directory
    }
  }
  return results;
}

/**
 * Reads session ID from a file.
 * Priority: filename UUID > file content > filename UUID fallback
 */
export async function readSessionIdFromFile(
  filePath: string,
): Promise<string | null> {
  try {
    // Priority 1: Use filename UUID (most reliable for Claude session files)
    const basename = path.basename(filePath);
    const filenameWithoutExt = basename.replace(/\.(json|jsonl)$/i, "");
    if (isValidUuidSessionId(filenameWithoutExt)) {
      return filenameWithoutExt;
    }

    // Priority 2: Extract from file content
    const content = await readFile(filePath, "utf-8");
    const fromContent = pickSessionIdFromText(content);
    if (fromContent) return fromContent;

    // Priority 3: Fallback to any UUID in filename
    const filenameMatch = basename.match(UUID_REGEX);
    return filenameMatch ? filenameMatch[0] : null;
  } catch {
    return null;
  }
}

/**
 * Reads session info (ID and cwd) from a file.
 */
export async function readSessionInfoFromFile(
  filePath: string,
): Promise<{ id: string | null; cwd: string | null }> {
  try {
    const content = await readFile(filePath, "utf-8");
    try {
      const parsed = JSON.parse(content);
      const id = pickSessionIdFromObject(parsed);
      const cwd = pickCwdFromObject(parsed);
      if (id || cwd) return { id, cwd };
    } catch {
      // ignore
    }

    const lines = content.split(/\r?\n/);
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      try {
        const parsedLine = JSON.parse(trimmed);
        const id = pickSessionIdFromObject(parsedLine);
        const cwd = pickCwdFromObject(parsedLine);
        if (id || cwd) return { id, cwd };
      } catch {
        // ignore
      }
    }

    // Fallback: filename UUID
    const filenameMatch = path.basename(filePath).match(UUID_REGEX);
    if (filenameMatch) return { id: filenameMatch[0], cwd: null };
  } catch {
    // ignore unreadable
  }
  return { id: null, cwd: null };
}

/**
 * Finds newest session ID from a directory with optional time filtering.
 */
export async function findNewestSessionIdFromDir(
  dir: string,
  recursive: boolean,
  options: Omit<SessionSearchOptions, "cwd"> = {},
): Promise<{ id: string; mtime: number } | null> {
  try {
    const files: { fullPath: string; mtime: number }[] = [];

    const processDir = async (currentDir: string) => {
      const currentEntries = await readdir(currentDir, { withFileTypes: true });
      for (const entry of currentEntries) {
        const fullPath = path.join(currentDir, entry.name);
        if (entry.isDirectory()) {
          if (recursive) {
            await processDir(fullPath);
          }
          continue;
        }
        if (!entry.isFile()) continue;
        if (!entry.name.endsWith(".json") && !entry.name.endsWith(".jsonl"))
          continue;
        try {
          const info = await stat(fullPath);
          files.push({ fullPath, mtime: info.mtimeMs });
        } catch {
          // ignore unreadable
        }
      }
    };

    await processDir(dir);

    // Apply since/until filters
    const filtered = files.filter((f) => {
      if (options.since !== undefined && f.mtime < options.since) return false;
      if (options.until !== undefined && f.mtime > options.until) return false;
      return true;
    });

    if (!filtered.length) return null;

    // Sort by mtime descending (newest first)
    let pool = filtered.sort((a, b) => b.mtime - a.mtime);

    // Apply preferClosestTo window if specified
    const ref = options.preferClosestTo;
    if (typeof ref === "number") {
      const window = options.windowMs ?? 30 * 60 * 1000;
      const withinWindow = pool.filter(
        (f) => Math.abs(f.mtime - ref) <= window,
      );
      if (withinWindow.length) {
        pool = withinWindow.sort((a, b) => b.mtime - a.mtime);
      }
    }

    for (const file of pool) {
      const id = await readSessionIdFromFile(file.fullPath);
      if (id) return { id, mtime: file.mtime };
    }
  } catch {
    // ignore
  }
  return null;
}

/**
 * Finds the latest nested session file from subdirectories.
 */
export async function findLatestNestedSessionFile(
  baseDir: string,
  subPath: string[],
  predicate: (name: string) => boolean,
): Promise<string | null> {
  try {
    const entries = await readdir(baseDir);
    if (!entries.length) return null;

    const candidates: { fullPath: string; mtime: number }[] = [];

    for (const entry of entries) {
      const dirPath = path.join(baseDir, entry, ...subPath);
      const latest = await findLatestFile(dirPath, predicate);
      if (latest) {
        try {
          const info = await stat(latest);
          candidates.push({ fullPath: latest, mtime: info.mtimeMs });
        } catch {
          // ignore
        }
      }
    }

    if (!candidates.length) return null;
    candidates.sort((a, b) => b.mtime - a.mtime);
    return candidates[0]?.fullPath ?? null;
  } catch {
    return null;
  }
}

/**
 * Reads text content from a file.
 * Wrapper for fs.readFile to centralize fs operations for testability.
 */
export async function readFileContent(filePath: string): Promise<string> {
  return readFile(filePath, "utf-8");
}

/**
 * Checks if a file exists and returns stat info.
 * Returns null if file does not exist.
 */
export async function checkFileStat(
  filePath: string,
): Promise<{ mtimeMs: number } | null> {
  try {
    const info = await stat(filePath);
    return { mtimeMs: info.mtimeMs };
  } catch {
    return null;
  }
}

/**
 * Checks if a session's cwd matches the target cwd.
 * Matching rules:
 * - Exact match
 * - Session cwd starts with target cwd (session is in subdirectory)
 * - Target cwd starts with session cwd (for worktree subdirectories)
 *
 * @param sessionCwd - The cwd from the session file
 * @param targetCwd - The target cwd to match against
 * @returns true if the cwd matches
 */
export function matchesCwd(
  sessionCwd: string | null,
  targetCwd: string,
): boolean {
  if (!sessionCwd) return false;
  return (
    sessionCwd === targetCwd ||
    sessionCwd.startsWith(targetCwd) ||
    targetCwd.startsWith(sessionCwd)
  );
}
